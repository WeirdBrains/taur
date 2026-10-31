//! Git smart HTTP via `git http-backend` (CGI). Clones are anonymous; pushes need HTTP
//! Basic auth with the account name and a session token as password (or a Bearer token).

use crate::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use std::io::Write;
use std::process::{Command, Stdio};
use taur_core::srcinfo::validate_name;

/// Largest accepted request body (pack upload).
pub const MAX_BODY: usize = 64 * 1024 * 1024;

fn decode_b64(s: &str) -> Option<Vec<u8>> {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0);
    for c in s.trim().bytes().filter(|&c| c != b'=') {
        buf = (buf << 6) | T.iter().position(|&t| t == c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Account name of the pusher, from Basic (name:token) or Bearer credentials.
fn pusher(state: &AppState, h: &HeaderMap) -> Option<String> {
    let v = h.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (name, token) = if let Some(b) = v.strip_prefix("Basic ") {
        let raw = String::from_utf8(decode_b64(b)?).ok()?;
        let (n, t) = raw.split_once(':')?;
        (Some(n.to_string()), t.to_string())
    } else {
        (None, v.strip_prefix("Bearer ")?.trim().to_string())
    };
    let user = state.store.user_for_token(&token).ok()?;
    match name {
        Some(n) if !n.eq_ignore_ascii_case(&user.name) => None,
        _ => Some(user.name),
    }
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"TAUR\"")],
        "authentication required: use your account name and a session token\n",
    )
        .into_response()
}

pub async fn handle(
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_string();
    let Some((name, rest)) = path.trim_start_matches('/').split_once(".git/") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if validate_name(name).is_err() || rest.split('/').any(|c| c == ".." || c.is_empty()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let name = name.to_string();
    let query = uri.query().unwrap_or("").to_string();
    let is_push = rest == "git-receive-pack" || query.contains("service=git-receive-pack");
    let repos = state.store.repos_dir();
    let mut user = None;
    if is_push {
        let Some(u) = pusher(&state, &headers) else {
            return unauthorized();
        };
        if let Err(e) = taur_core::repo::ensure_repo(&repos, &name) {
            return crate::ApiError(e).into_response();
        }
        user = Some(u);
    } else if !repos.join(format!("{name}.git")).is_dir() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let hdr = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let mut cmd = Command::new("git");
    cmd.arg("http-backend")
        .env_clear()
        .envs(taur_core::base_child_env())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", taur_core::NULL_DEVICE)
        .env("GIT_PROJECT_ROOT", &repos)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("PATH_INFO", &path)
        .env("QUERY_STRING", &query)
        .env("REQUEST_METHOD", method.as_str())
        .env("CONTENT_TYPE", hdr("content-type"))
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("HTTP_CONTENT_ENCODING", hdr("content-encoding"))
        .env("GIT_PROTOCOL", hdr("git-protocol"))
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("TAUR_HOOK_BIN", &state.hook_bin)
        .env("TAUR_HOME", state.store.home())
        .envs(state.store.hook_env());
    if let Some(u) = &user {
        cmd.env("REMOTE_USER", u).env("TAUR_PUSHER", u);
    }
    let out = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("stdin");
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&body);
        });
        let out = child.wait_with_output()?;
        let _ = writer.join();
        Ok(out.stdout)
    })
    .await;
    let raw = match out {
        Ok(Ok(raw)) => raw,
        _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    cgi_response(&raw)
}

/// Turn CGI output (headers, blank line, body) into a response.
fn cgi_response(raw: &[u8]) -> Response {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| (p, p + 4))
        .or_else(|| {
            raw.windows(2)
                .position(|w| w == b"\n\n")
                .map(|p| (p, p + 2))
        });
    let Some((hend, bstart)) = split else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let mut status = StatusCode::OK;
    let mut resp = Response::builder();
    for line in String::from_utf8_lossy(&raw[..hend]).lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        if k.eq_ignore_ascii_case("status") {
            if let Some(code) = v.split_whitespace().next().and_then(|c| c.parse().ok()) {
                status = StatusCode::from_u16(code).unwrap_or(StatusCode::OK);
            }
        } else if let Ok(hv) = HeaderValue::from_str(v) {
            resp = resp.header(k.trim(), hv);
        }
    }
    resp.status(status)
        .body(axum::body::Body::from(raw[bstart..].to_vec()))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}
