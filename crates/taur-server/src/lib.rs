//! HTTP front end: a thin axum layer over `taur_core`.

use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRef, Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use taur_core::LimitClass;
use taur_core::accounts::User;
use taur_core::{Error, Role, Store, rpc};

type St = State<Arc<Store>>;

pub mod acme;
pub mod cli;
pub mod githttp;
pub mod grpc;
pub mod mcp;
pub mod ssh;
pub mod web;

/// Server settings shared by handlers.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    /// Binary that implements `hook pre-receive|post-receive` (normally `taur` itself).
    pub hook_bin: std::path::PathBuf,
}

impl FromRef<AppState> for Arc<Store> {
    fn from_ref(s: &AppState) -> Arc<Store> {
        s.store.clone()
    }
}

/// Router using the current executable as hook binary.
pub fn app(store: Arc<Store>) -> Router {
    let hook_bin = std::env::current_exe().unwrap_or_else(|_| "taur".into());
    app_with(AppState { store, hook_bin })
}

pub fn app_with(state: AppState) -> Router {
    let mut router = Router::new()
        .route("/static/app.js", get(web::app_js))
        .route("/static/style.css", get(web::style));
    for page in web::PAGES {
        router = router.route(page, get(web::index));
    }
    router
        .route("/rpc", get(rpc_legacy).post(rpc_legacy_post))
        .route("/rpc/", get(rpc_legacy).post(rpc_legacy_post))
        .route("/rpc/v5/{kind}", get(rpc_v5).post(rpc_v5_post))
        .route("/rpc/v5/{kind}/{arg}", get(rpc_v5_arg))
        .route("/cgit/aur.git/snapshot/{file}", get(snapshot))
        .route("/cgit/aur.git/plain/{*path}", get(plain_file))
        .route("/packages.gz", get(dump_packages))
        .route("/pkgbase.gz", get(dump_pkgbases))
        .route("/packages-meta-v1.json.gz", get(dump_meta))
        .route("/packages-meta-ext-v1.json.gz", get(dump_meta_ext))
        .route("/mcp", post(mcp_post).get(mcp_get).delete(mcp_get))
        .route("/api/v1/server", get(server_info))
        .route("/api/v1/register", post(register))
        .route("/api/v1/login", post(login))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/account", get(account).delete(unregister))
        .route("/api/v1/verify", post(verify))
        .route("/api/v1/verify/resend", post(resend_verification))
        .route("/api/v1/password-reset", post(reset_request))
        .route("/api/v1/password-reset/confirm", post(reset_confirm))
        .route("/api/v1/orgs", get(list_orgs).post(create_org))
        .route("/api/v1/orgs/{org}", get(show_org).delete(delete_org))
        .route(
            "/api/v1/orgs/{org}/members/{user}",
            axum::routing::put(set_member).delete(remove_member),
        )
        .route("/api/v1/packages", get(my_packages))
        .route("/api/v1/packages/{pkgbase}/owner", post(set_owner))
        .route("/api/v1/packages/{pkgbase}/disown", post(disown))
        .route("/api/v1/packages/{pkgbase}/adopt", post(adopt))
        .route("/api/v1/packages/{pkgbase}/restore", post(restore_pkg))
        .route(
            "/api/v1/packages/{pkgbase}/vote",
            get(vote_status).post(vote).delete(unvote),
        )
        .route(
            "/api/v1/packages/{pkgbase}/flag",
            get(flag_info).post(flag).delete(unflag),
        )
        .route(
            "/api/v1/packages/{pkgbase}/comments",
            get(list_comments).post(add_comment),
        )
        .route(
            "/api/v1/packages/{pkgbase}/comaintainers",
            axum::routing::put(set_comaintainers),
        )
        .route("/api/v1/packages/{pkgbase}/requests", post(file_request))
        .route(
            "/api/v1/packages/{pkgbase}/notify",
            get(notify_status).post(notify_on).delete(notify_off),
        )
        .route(
            "/api/v1/comments/{id}/pin",
            post(pin_comment).delete(unpin_comment),
        )
        .route(
            "/api/v1/comments/{id}",
            axum::routing::patch(edit_comment).delete(delete_comment),
        )
        .route("/api/v1/requests", get(list_requests))
        .route("/api/v1/requests/{id}", get(show_request))
        .route("/api/v1/requests/{id}/accept", post(accept_request))
        .route("/api/v1/requests/{id}/reject", post(reject_request))
        .route(
            "/api/v1/admin/moderators/{name}",
            axum::routing::put(set_moderator),
        )
        .route(
            "/api/v1/moderation/packages/{name}/reject",
            post(reject_package),
        )
        .route("/api/v1/moderation/users/{name}/reject", post(reject_user))
        .route(
            "/api/v1/registries",
            get(list_registries).post(create_registry_route),
        )
        .route(
            "/api/v1/registries/{name}/keys",
            post(add_registry_key_route),
        )
        .route(
            "/api/v1/registries/{name}/identities",
            post(bind_registry_identity),
        )
        .route(
            "/api/v1/registries/{name}/expectations",
            post(expect_registry_package),
        )
        .route("/api/v1/audit", get(list_audit))
        .route("/api/v1/audit/{id}/reverse", post(reverse_audit))
        .route("/api/v1/audit/{id}/accept", post(accept_audit))
        .route("/api/v1/audit/{id}/revoke", post(revoke_audit))
        .route("/api/v1/ssh-keys", get(list_keys).post(add_key))
        .route("/api/v1/ssh-keys/{id}", axum::routing::delete(remove_key))
        .fallback(githttp::handle)
        .layer(DefaultBodyLimit::max(githttp::MAX_BODY))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            limit_requests,
        ))
        .with_state(state)
}

fn limit_class(path: &str) -> Option<LimitClass> {
    if path == "/rpc" || path.starts_with("/rpc/") {
        return Some(LimitClass::Rpc);
    }
    if path == "/mcp" {
        return Some(LimitClass::Mcp);
    }
    if matches!(
        path,
        "/packages.gz"
            | "/pkgbase.gz"
            | "/packages-meta-v1.json.gz"
            | "/packages-meta-ext-v1.json.gz"
    ) || path.starts_with("/cgit/aur.git/snapshot/")
    {
        return Some(LimitClass::Dump);
    }
    if path.starts_with("/cgit/aur.git/plain/") || githttp::is_git_path(path) {
        return Some(LimitClass::Git);
    }
    if matches!(
        path,
        "/api/v1/register"
            | "/api/v1/login"
            | "/api/v1/verify"
            | "/api/v1/verify/resend"
            | "/api/v1/password-reset"
            | "/api/v1/password-reset/confirm"
    ) {
        return Some(LimitClass::Auth);
    }
    if path == "/api/v1/server" || path.starts_with("/static/") {
        return None;
    }
    if path.starts_with("/api/v1/") {
        return Some(LimitClass::Api);
    }
    None
}

async fn limit_requests(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    if let Some(class) = limit_class(req.uri().path()) {
        let peer = req
            .extensions()
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip());
        let forwarded = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok());
        if let Err(Error::RateLimited { retry_after }) =
            state.store.enforce_limit(class, peer, forwarded)
        {
            return limited(class, retry_after);
        }
    }
    next.run(req).await
}

fn limited(class: LimitClass, retry_after: u64) -> Response {
    let mut res = if class == LimitClass::Rpc {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(rpc::error("Rate limit reached")),
        )
            .into_response()
    } else if class == LimitClass::Git {
        (StatusCode::TOO_MANY_REQUESTS, "Rate limit reached\n").into_response()
    } else {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": "Rate limit reached" })),
        )
            .into_response()
    };
    if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
        res.headers_mut().insert(header::RETRY_AFTER, value);
    }
    res
}

// ---- RPC ----

fn pairs(q: Option<&str>) -> Vec<(String, String)> {
    q.and_then(|q| serde_urlencoded::from_str(q).ok())
        .unwrap_or_default()
}

fn param<'a>(p: &'a [(String, String)], key: &str) -> Option<&'a str> {
    p.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// aurweb `JSONP_EXPR`: callback names of 1–128 `[A-Za-z0-9()_.]`.
fn jsonp_ok(cb: &str) -> bool {
    (1..=128).contains(&cb.len())
        && cb
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '(' | ')' | '_' | '.'))
}

fn rpc_body(value: Value, callback: Option<&str>) -> Response {
    if let Some(cb) = callback.filter(|c| !c.is_empty()) {
        if !jsonp_ok(cb) {
            return Json(rpc::error("Invalid callback name.")).into_response();
        }
        let body = format!(
            "/**/{cb}({})",
            serde_json::to_string(&value).unwrap_or_else(|_| "null".into())
        );
        return (
            [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
            body,
        )
            .into_response();
    }
    Json(value).into_response()
}

/// aurweb's legacy `arg` / `arg[]` rule: the last key wins the shape.
/// A final `arg` is the only argument. A final `arg[]` keeps the trailing
/// `arg[]` values (in reverse query order, matching aurweb). A path argument
/// (`/rpc/v5/info/{name}`) is prepended and query args stay in URL order.
fn collect_args(p: &[(String, String)], first: Option<String>) -> Vec<String> {
    if let Some(first) = first {
        let mut args = vec![first];
        args.extend(
            p.iter()
                .filter(|(k, _)| k == "arg" || k == "arg[]")
                .map(|(_, v)| v.clone()),
        );
        return args;
    }
    let indexed: Vec<(usize, &str)> = p
        .iter()
        .enumerate()
        .filter(|(_, (k, _))| k == "arg" || k == "arg[]")
        .map(|(i, (_, v))| (i, v.as_str()))
        .collect();
    let Some(&(last, _)) = indexed.last() else {
        return Vec::new();
    };
    if p[last].0 == "arg" {
        return vec![p[last].1.clone()];
    }
    let mut args = Vec::new();
    for i in indexed.into_iter().map(|(i, _)| i).rev() {
        if p[i].0 != "arg[]" {
            break;
        }
        args.push(p[i].1.clone());
    }
    args
}

fn finish(
    store: &Store,
    kind: Option<&str>,
    p: &[(String, String)],
    first: Option<String>,
) -> Response {
    let value = rpc::handle(store, kind, &collect_args(p, first), param(p, "by"));
    rpc_body(value, param(p, "callback"))
}

fn legacy(store: &Store, p: &[(String, String)]) -> Response {
    match param(p, "v") {
        Some("5") => finish(store, param(p, "type"), p, None),
        None => rpc_body(
            rpc::error("Please specify an API version."),
            param(p, "callback"),
        ),
        Some(_) => rpc_body(
            rpc::error("Invalid version specified."),
            param(p, "callback"),
        ),
    }
}

async fn rpc_legacy(State(s): St, RawQuery(q): RawQuery) -> Response {
    legacy(&s, &pairs(q.as_deref()))
}

async fn rpc_legacy_post(State(s): St, RawQuery(q): RawQuery, body: String) -> Response {
    let mut p = pairs(q.as_deref());
    p.extend(pairs(Some(&body)));
    legacy(&s, &p)
}

async fn rpc_v5(State(s): St, Path(kind): Path<String>, RawQuery(q): RawQuery) -> Response {
    finish(&s, Some(&kind), &pairs(q.as_deref()), None)
}

async fn rpc_v5_post(
    State(s): St,
    Path(kind): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let ctype = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ctype.contains("application/json") {
        return rpc_json(&s, &kind, &body);
    }
    finish(&s, Some(&kind), &pairs(Some(&body)), None)
}

async fn rpc_v5_arg(
    State(s): St,
    Path((kind, arg)): Path<(String, String)>,
    RawQuery(q): RawQuery,
) -> Response {
    finish(&s, Some(&kind), &pairs(q.as_deref()), Some(arg))
}

/// aurweb OpenAPI JSON bodies: `info` takes `arg` as an array; `search` takes
/// string `arg` and `by`.
fn rpc_json(store: &Store, kind: &str, body: &str) -> Response {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc::error("request body is not valid JSON")),
            )
                .into_response();
        }
    };
    if kind == "info" || kind == "multiinfo" {
        let Some(arr) = v.get("arg").and_then(|a| a.as_array()) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc::error("the 'arg' parameter must be of array type")),
            )
                .into_response();
        };
        let args: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
        if args.len() != arr.len() {
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc::error("the 'arg' parameter must be of array type")),
            )
                .into_response();
        }
        return rpc_body(rpc::handle(store, Some(kind), &args, None), None);
    }
    if kind == "search" || kind == "msearch" {
        let by = match v.get("by") {
            None => None,
            Some(b) => match b.as_str() {
                Some(s) => Some(s),
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(rpc::error("the 'by' parameter must be of string type")),
                    )
                        .into_response();
                }
            },
        };
        let arg = match v.get("arg") {
            None => String::new(),
            Some(a) => match a.as_str() {
                Some(s) => s.to_string(),
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(rpc::error("the 'arg' parameter must be of string type")),
                    )
                        .into_response();
                }
            },
        };
        return rpc_body(rpc::handle(store, Some(kind), &[arg], by), None);
    }
    finish(store, Some(kind), &[], None)
}

// ---- git, snapshots, dumps ----

fn gz(r: taur_core::Result<Vec<u8>>) -> Response {
    match r {
        Ok(b) => ([(header::CONTENT_TYPE, "application/gzip")], b).into_response(),
        Err(Error::NotFound(_)) | Err(Error::Invalid(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => ApiError(e).into_response(),
    }
}

async fn snapshot(State(s): St, Path(file): Path<String>) -> Response {
    match file.strip_suffix(".tar.gz") {
        Some(name) => gz(s.snapshot(name)),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct PlainQuery {
    /// Package base (aurweb cgit `?h=`). Each base is its own repository.
    h: Option<String>,
    /// Git revision inside that repository (default `HEAD`).
    id: Option<String>,
}

/// `/cgit/aur.git/plain/<path>?h=<pkgbase>` — one file from the package tree.
async fn plain_file(
    State(s): St,
    Path(path): Path<String>,
    axum::extract::Query(q): axum::extract::Query<PlainQuery>,
) -> Response {
    let Some(pkgbase) = q.h.filter(|h| !h.is_empty()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match s.plain(&pkgbase, q.id.as_deref().unwrap_or("HEAD"), &path) {
        Ok(bytes) => {
            let ctype = if bytes.contains(&0) {
                "application/octet-stream"
            } else {
                "text/plain; charset=utf-8"
            };
            ([(header::CONTENT_TYPE, ctype)], bytes).into_response()
        }
        Err(Error::NotFound(_)) | Err(Error::Invalid(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => ApiError(e).into_response(),
    }
}

async fn dump_packages(State(s): St) -> Response {
    gz(s.dump_packages())
}

async fn dump_pkgbases(State(s): St) -> Response {
    gz(s.dump_pkgbases())
}

async fn dump_meta(State(s): St) -> Response {
    gz(s.dump_packages_meta())
}

async fn dump_meta_ext(State(s): St) -> Response {
    gz(s.dump_packages_meta_ext())
}

async fn server_info(State(s): St) -> Response {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "base_url": s.base_url(),
        "ssh_clone_base": s.ssh_clone_base(),
    }))
    .into_response()
}

// ---- MCP ----

async fn mcp_post(State(s): St, h: HeaderMap, body: axum::body::Bytes) -> Response {
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return Json(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}}))
                .into_response();
        }
    };
    let token = bearer(&h).ok();
    let store = s.clone();
    let out = tokio::task::spawn_blocking(move || mcp::handle(&store, token.as_deref(), msg))
        .await
        .ok()
        .flatten();
    match out {
        Some(v) => Json(v).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Stateless server: no server-initiated SSE stream and no sessions to delete.
async fn mcp_get() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

// ---- accounts ----

pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, msg) = match &self.0 {
            Error::Unauthorized => (StatusCode::UNAUTHORIZED, self.0.to_string()),
            Error::Forbidden | Error::Unverified | Error::Suspended(_) => {
                (StatusCode::FORBIDDEN, self.0.to_string())
            }
            Error::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            Error::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            Error::Invalid(m) | Error::Parse(m) => (StatusCode::BAD_REQUEST, m.clone()),
            Error::Mail(m) => {
                eprintln!("taur: mail delivery failed: {m}");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "mail server unreachable".into(),
                )
            }
            Error::RateLimited { retry_after } => {
                let mut res = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({ "error": "Rate limit reached" })),
                )
                    .into_response();
                if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
                    res.headers_mut().insert(header::RETRY_AFTER, value);
                }
                return res;
            }
            other => {
                // Error values never contain passwords or tokens.
                eprintln!("taur: internal error: {other}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (code, Json(json!({ "error": msg }))).into_response()
    }
}

pub(crate) fn bearer(h: &HeaderMap) -> Result<String, ApiError> {
    h.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or(ApiError(Error::Unauthorized))
}

#[derive(Deserialize)]
struct RegisterReq {
    name: String,
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginReq {
    name: String,
    password: String,
}

#[derive(Deserialize)]
struct PasswordReq {
    password: String,
}

async fn register(State(s): St, Json(r): Json<RegisterReq>) -> Result<Response, ApiError> {
    let user = s.register(&r.name, &r.email, &r.password)?;
    Ok((StatusCode::CREATED, Json(user)).into_response())
}

async fn login(State(s): St, Json(r): Json<LoginReq>) -> Result<Response, ApiError> {
    Ok(Json(s.login(&r.name, &r.password)?).into_response())
}

async fn logout(State(s): St, h: HeaderMap) -> Result<StatusCode, ApiError> {
    s.logout(&bearer(&h)?)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn account(State(s): St, h: HeaderMap) -> Result<Response, ApiError> {
    Ok(Json(s.user_for_token(&bearer(&h)?)?).into_response())
}

#[derive(Deserialize)]
struct ReasonBody {
    reason: String,
}

#[derive(Deserialize)]
struct ModeratorBody {
    on: bool,
}

#[derive(Deserialize)]
struct RegistryBody {
    name: String,
}

#[derive(Deserialize)]
struct RegistryKeyBody {
    label: String,
    public_key_hex: String,
}

#[derive(Deserialize)]
struct IdentityBody {
    nick: String,
    email: String,
}

#[derive(Deserialize)]
struct ExpectationBody {
    pkgbase: String,
    repository: String,
    version: String,
    signature_hex: String,
    key_fingerprint: String,
    nick: String,
}

async fn list_registries(State(s): St, h: HeaderMap) -> Result<Json<Value>, ApiError> {
    let u = auth_verified(&s, &h)?;
    if !s.is_admin(u.id)? {
        return Err(Error::Forbidden.into());
    }
    Ok(Json(
        serde_json::to_value(s.list_registries()?).map_err(Error::from)?,
    ))
}

async fn create_registry_route(
    State(s): St,
    h: HeaderMap,
    Json(body): Json<RegistryBody>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.create_registry(u.id, &body.name)?;
    Ok(StatusCode::CREATED)
}

async fn add_registry_key_route(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<RegistryKeyBody>,
) -> Result<Json<Value>, ApiError> {
    let u = auth_verified(&s, &h)?;
    let key = taur_core::registry::parse_hex_key(&body.public_key_hex, 32)?;
    let fp = s.add_registry_key(u.id, &name, &body.label, &key)?;
    Ok(Json(json!({ "fingerprint": fp, "provenance": "operator" })))
}

async fn bind_registry_identity(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<IdentityBody>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.bind_identity(u.id, &name, &body.nick, &body.email)?;
    Ok(StatusCode::CREATED)
}

async fn expect_registry_package(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<ExpectationBody>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    let signature = taur_core::registry::parse_hex_key(&body.signature_hex, 64)?;
    s.expect_package(
        u.id,
        &name,
        &body.pkgbase,
        &body.repository,
        &body.version,
        &signature,
        &body.key_fingerprint,
        &body.nick,
    )?;
    Ok(StatusCode::CREATED)
}

async fn set_moderator(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<ModeratorBody>,
) -> Result<StatusCode, ApiError> {
    let u = auth(&s, &h)?;
    s.set_moderator(u.id, &name, body.on)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reject_package(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<ReasonBody>,
) -> Result<Json<Value>, ApiError> {
    let u = auth_verified(&s, &h)?;
    let id = s.reject_package(u.id, &name, &body.reason)?;
    Ok(Json(json!({ "event": id })))
}

async fn reject_user(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<ReasonBody>,
) -> Result<Json<Value>, ApiError> {
    let u = auth_verified(&s, &h)?;
    let id = s.reject_user(u.id, &name, &body.reason)?;
    Ok(Json(json!({ "event": id })))
}

async fn list_audit(State(s): St, h: HeaderMap) -> Result<Json<Value>, ApiError> {
    let u = auth(&s, &h)?;
    let rows = s.list_audit(u.id)?;
    Ok(Json(serde_json::to_value(rows).map_err(Error::from)?))
}

async fn accept_audit(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<ReasonBody>,
) -> Result<Json<Value>, ApiError> {
    let u = auth_verified(&s, &h)?;
    let id = s.accept_rejection(u.id, id, &body.reason)?;
    Ok(Json(json!({ "event": id })))
}

async fn revoke_audit(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<ReasonBody>,
) -> Result<Json<Value>, ApiError> {
    let u = auth_verified(&s, &h)?;
    let id = s.revoke_acceptance(u.id, id, &body.reason)?;
    Ok(Json(json!({ "event": id })))
}

async fn reverse_audit(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<ReasonBody>,
) -> Result<Json<Value>, ApiError> {
    let u = auth_verified(&s, &h)?;
    let id = s.reverse_audit(u.id, id, &body.reason)?;
    Ok(Json(json!({ "event": id })))
}

async fn unregister(
    State(s): St,
    h: HeaderMap,
    Json(r): Json<PasswordReq>,
) -> Result<StatusCode, ApiError> {
    s.unregister(&bearer(&h)?, &r.password)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- organizations and package ownership ----

fn auth(s: &Store, h: &HeaderMap) -> Result<User, ApiError> {
    Ok(s.user_for_token(&bearer(h)?)?)
}

/// Like `auth`, but the account's email must be verified.
fn auth_verified(s: &Store, h: &HeaderMap) -> Result<User, ApiError> {
    let u = auth(s, h)?;
    s.require_verified(&u)?;
    Ok(u)
}

fn pkgbase(s: &Store, name: &str) -> Result<i64, ApiError> {
    s.pkgbase_id(name)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("package base {name}"))))
}

#[derive(Deserialize)]
struct NameReq {
    name: String,
}

#[derive(Deserialize)]
struct RoleReq {
    role: Role,
}

#[derive(Deserialize)]
struct OwnerReq {
    org: Option<String>,
    user: Option<String>,
}

async fn list_orgs(State(s): St, h: HeaderMap) -> Result<Response, ApiError> {
    let u = auth(&s, &h)?;
    Ok(Json(s.orgs_for(u.id)?).into_response())
}

async fn create_org(
    State(s): St,
    h: HeaderMap,
    Json(r): Json<NameReq>,
) -> Result<Response, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.create_org(u.id, &r.name)?;
    Ok((StatusCode::CREATED, Json(s.org_detail(u.id, &r.name)?)).into_response())
}

async fn show_org(
    State(s): St,
    h: HeaderMap,
    Path(org): Path<String>,
) -> Result<Response, ApiError> {
    let u = auth(&s, &h)?;
    Ok(Json(s.org_detail(u.id, &org)?).into_response())
}

async fn delete_org(
    State(s): St,
    h: HeaderMap,
    Path(org): Path<String>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.delete_org(u.id, s.org_id(&org)?)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_member(
    State(s): St,
    h: HeaderMap,
    Path((org, user)): Path<(String, String)>,
    Json(r): Json<RoleReq>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.set_member(u.id, s.org_id(&org)?, s.user_id(&user)?, r.role)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_member(
    State(s): St,
    h: HeaderMap,
    Path((org, user)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.remove_member(u.id, s.org_id(&org)?, s.user_id(&user)?)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn my_packages(State(s): St, h: HeaderMap) -> Result<Response, ApiError> {
    let u = auth(&s, &h)?;
    Ok(Json(s.packages_for(u.id)?).into_response())
}

async fn set_owner(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(r): Json<OwnerReq>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    let pb = pkgbase(&s, &name)?;
    match (r.org, r.user) {
        (Some(org), None) => s.set_pkgbase_org(u.id, pb, Some(s.org_id(&org)?))?,
        (None, Some(user)) => s.transfer_to_user(u.id, pb, s.user_id(&user)?)?,
        _ => {
            return Err(ApiError(Error::Invalid(
                "give exactly one of \"org\" or \"user\"".into(),
            )));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn disown(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.disown(u.id, pkgbase(&s, &name)?)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn adopt(State(s): St, h: HeaderMap, Path(name): Path<String>) -> Result<Response, ApiError> {
    let u = auth_verified(&s, &h)?;
    match s.adopt(u.id, pkgbase(&s, &name)?)? {
        taur_core::orgs::Adopted::Maintainer => Ok(StatusCode::NO_CONTENT.into_response()),
        taur_core::orgs::Adopted::Request(id) => Ok((
            StatusCode::ACCEPTED,
            Json(json!({"request": id, "status": "pending"})),
        )
            .into_response()),
    }
}

async fn restore_pkg(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.restore_pkgbase(u.id, &name)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- ssh keys ----

#[derive(Deserialize)]
struct KeyReq {
    public_key: String,
}

async fn list_keys(State(s): St, h: HeaderMap) -> Result<Response, ApiError> {
    let u = auth(&s, &h)?;
    Ok(Json(s.list_ssh_keys(u.id)?).into_response())
}

async fn add_key(State(s): St, h: HeaderMap, Json(r): Json<KeyReq>) -> Result<Response, ApiError> {
    let u = auth_verified(&s, &h)?;
    Ok((
        StatusCode::CREATED,
        Json(s.add_ssh_key(u.id, &r.public_key)?),
    )
        .into_response())
}

async fn remove_key(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let u = auth(&s, &h)?;
    s.remove_ssh_key(u.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- email verification and password reset ----

#[derive(Deserialize)]
struct TokenReq {
    token: String,
}

#[derive(Deserialize)]
struct EmailReq {
    email: String,
}

#[derive(Deserialize)]
struct ResetReq {
    token: String,
    password: String,
}

async fn verify(State(s): St, Json(r): Json<TokenReq>) -> Result<Response, ApiError> {
    Ok(Json(s.verify_email(&r.token)?).into_response())
}

async fn resend_verification(State(s): St, h: HeaderMap) -> Result<StatusCode, ApiError> {
    let u = auth(&s, &h)?;
    s.resend_verification(&u)?;
    Ok(StatusCode::ACCEPTED)
}

async fn reset_request(State(s): St, Json(r): Json<EmailReq>) -> Result<StatusCode, ApiError> {
    s.request_password_reset(&r.email)?;
    Ok(StatusCode::ACCEPTED)
}

async fn reset_confirm(State(s): St, Json(r): Json<ResetReq>) -> Result<StatusCode, ApiError> {
    s.reset_password(&r.token, &r.password)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- community: votes, flags, comments, co-maintainers, requests ----

#[derive(Deserialize)]
struct CommentReq {
    #[serde(default)]
    comment: String,
}

#[derive(Deserialize)]
struct ContentReq {
    content: String,
}

#[derive(Deserialize)]
struct UsersReq {
    users: Vec<String>,
}

#[derive(Deserialize)]
struct FileRequestReq {
    kind: String,
    merge_into: Option<String>,
    comment: String,
}

#[derive(Deserialize)]
struct RequestsQuery {
    #[serde(default)]
    all: bool,
}

async fn vote(State(s): St, h: HeaderMap, Path(p): Path<String>) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.vote(u.id, &p)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn vote_status(
    State(s): St,
    h: HeaderMap,
    Path(p): Path<String>,
) -> Result<Response, ApiError> {
    let u = auth(&s, &h)?;
    Ok(Json(json!({ "voted": s.has_voted(u.id, &p)? })).into_response())
}

async fn notify_status(
    State(s): St,
    h: HeaderMap,
    Path(p): Path<String>,
) -> Result<Response, ApiError> {
    let u = auth(&s, &h)?;
    Ok(Json(json!({ "subscribed": s.is_subscribed(u.id, &p)? })).into_response())
}

async fn notify_on(
    State(s): St,
    h: HeaderMap,
    Path(p): Path<String>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.subscribe(u.id, &p, true)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn notify_off(
    State(s): St,
    h: HeaderMap,
    Path(p): Path<String>,
) -> Result<StatusCode, ApiError> {
    let u = auth(&s, &h)?;
    s.subscribe(u.id, &p, false)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pin_comment(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.pin_comment(u.id, id, true)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unpin_comment(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.pin_comment(u.id, id, false)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unvote(State(s): St, h: HeaderMap, Path(p): Path<String>) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.unvote(u.id, &p)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn flag_info(State(s): St, Path(p): Path<String>) -> Result<Response, ApiError> {
    Ok(Json(s.flag_info(&p)?).into_response())
}

async fn flag(
    State(s): St,
    h: HeaderMap,
    Path(p): Path<String>,
    Json(r): Json<CommentReq>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.flag_out_of_date(u.id, &p, &r.comment)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unflag(State(s): St, h: HeaderMap, Path(p): Path<String>) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.unflag(u.id, &p)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_comments(State(s): St, Path(p): Path<String>) -> Result<Response, ApiError> {
    Ok(Json(s.comments(&p)?).into_response())
}

async fn add_comment(
    State(s): St,
    h: HeaderMap,
    Path(p): Path<String>,
    Json(r): Json<ContentReq>,
) -> Result<Response, ApiError> {
    let u = auth_verified(&s, &h)?;
    Ok((
        StatusCode::CREATED,
        Json(s.add_comment(u.id, &p, &r.content)?),
    )
        .into_response())
}

async fn edit_comment(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(r): Json<ContentReq>,
) -> Result<Response, ApiError> {
    let u = auth_verified(&s, &h)?;
    Ok(Json(s.edit_comment(u.id, id, &r.content)?).into_response())
}

async fn delete_comment(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.delete_comment(u.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_comaintainers(
    State(s): St,
    h: HeaderMap,
    Path(p): Path<String>,
    Json(r): Json<UsersReq>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.set_comaintainers(u.id, &p, &r.users)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn file_request(
    State(s): St,
    h: HeaderMap,
    Path(p): Path<String>,
    Json(r): Json<FileRequestReq>,
) -> Result<Response, ApiError> {
    let u = auth_verified(&s, &h)?;
    let req = s.file_request(u.id, &p, &r.kind, r.merge_into.as_deref(), &r.comment)?;
    Ok((StatusCode::CREATED, Json(req)).into_response())
}

async fn list_requests(
    State(s): St,
    h: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<RequestsQuery>,
) -> Result<Response, ApiError> {
    auth(&s, &h)?;
    Ok(Json(s.list_requests(!q.all)?).into_response())
}

async fn show_request(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    auth(&s, &h)?;
    Ok(Json(s.request(id)?).into_response())
}

async fn accept_request(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(r): Json<CommentReq>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.accept_request(u.id, id, &r.comment)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reject_request(
    State(s): St,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(r): Json<CommentReq>,
) -> Result<StatusCode, ApiError> {
    let u = auth_verified(&s, &h)?;
    s.reject_request(u.id, id, &r.comment)?;
    Ok(StatusCode::NO_CONTENT)
}
