//! HTTP front end: a thin axum layer over `taur_core`.

use axum::extract::{DefaultBodyLimit, FromRef, Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use taur_core::accounts::User;
use taur_core::{Error, Role, Store, rpc};

type St = State<Arc<Store>>;

pub mod cli;
pub mod githttp;
pub mod ssh;

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
    Router::new()
        .route("/rpc", get(rpc_legacy).post(rpc_legacy_post))
        .route("/rpc/", get(rpc_legacy).post(rpc_legacy_post))
        .route("/rpc/v5/{kind}", get(rpc_v5).post(rpc_v5_post))
        .route("/rpc/v5/{kind}/{arg}", get(rpc_v5_arg))
        .route("/cgit/aur.git/snapshot/{file}", get(snapshot))
        .route("/packages.gz", get(dump_packages))
        .route("/pkgbase.gz", get(dump_pkgbases))
        .route("/packages-meta-v1.json.gz", get(dump_meta))
        .route("/api/v1/register", post(register))
        .route("/api/v1/login", post(login))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/account", get(account).delete(unregister))
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
        .route("/api/v1/ssh-keys", get(list_keys).post(add_key))
        .route("/api/v1/ssh-keys/{id}", axum::routing::delete(remove_key))
        .fallback(githttp::handle)
        .layer(DefaultBodyLimit::max(githttp::MAX_BODY))
        .with_state(state)
}

// ---- RPC ----

fn pairs(q: Option<&str>) -> Vec<(String, String)> {
    q.and_then(|q| serde_urlencoded::from_str(q).ok())
        .unwrap_or_default()
}

fn param<'a>(p: &'a [(String, String)], key: &str) -> Option<&'a str> {
    p.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn run(
    store: &Store,
    kind: Option<&str>,
    p: &[(String, String)],
    first: Option<String>,
) -> Json<Value> {
    let mut args: Vec<String> = first.into_iter().collect();
    args.extend(
        p.iter()
            .filter(|(k, _)| k == "arg" || k == "arg[]")
            .map(|(_, v)| v.clone()),
    );
    Json(rpc::handle(store, kind, &args, param(p, "by")))
}

fn legacy(store: &Store, p: &[(String, String)]) -> Json<Value> {
    match param(p, "v") {
        Some("5") => run(store, param(p, "type"), p, None),
        None => Json(rpc::error("Please specify an API version.")),
        Some(_) => Json(rpc::error("Invalid version specified.")),
    }
}

async fn rpc_legacy(State(s): St, RawQuery(q): RawQuery) -> Json<Value> {
    legacy(&s, &pairs(q.as_deref()))
}

async fn rpc_legacy_post(State(s): St, RawQuery(q): RawQuery, body: String) -> Json<Value> {
    let mut p = pairs(q.as_deref());
    p.extend(pairs(Some(&body)));
    legacy(&s, &p)
}

async fn rpc_v5(State(s): St, Path(kind): Path<String>, RawQuery(q): RawQuery) -> Json<Value> {
    run(&s, Some(&kind), &pairs(q.as_deref()), None)
}

async fn rpc_v5_post(State(s): St, Path(kind): Path<String>, body: String) -> Json<Value> {
    run(&s, Some(&kind), &pairs(Some(&body)), None)
}

async fn rpc_v5_arg(
    State(s): St,
    Path((kind, arg)): Path<(String, String)>,
    RawQuery(q): RawQuery,
) -> Json<Value> {
    run(&s, Some(&kind), &pairs(q.as_deref()), Some(arg))
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

async fn dump_packages(State(s): St) -> Response {
    gz(s.dump_packages())
}

async fn dump_pkgbases(State(s): St) -> Response {
    gz(s.dump_pkgbases())
}

async fn dump_meta(State(s): St) -> Response {
    gz(s.dump_packages_meta())
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
            Error::Forbidden => (StatusCode::FORBIDDEN, self.0.to_string()),
            Error::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            Error::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            Error::Invalid(m) | Error::Parse(m) => (StatusCode::BAD_REQUEST, m.clone()),
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
    let u = auth(&s, &h)?;
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
    let u = auth(&s, &h)?;
    s.delete_org(u.id, s.org_id(&org)?)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_member(
    State(s): St,
    h: HeaderMap,
    Path((org, user)): Path<(String, String)>,
    Json(r): Json<RoleReq>,
) -> Result<StatusCode, ApiError> {
    let u = auth(&s, &h)?;
    s.set_member(u.id, s.org_id(&org)?, s.user_id(&user)?, r.role)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_member(
    State(s): St,
    h: HeaderMap,
    Path((org, user)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let u = auth(&s, &h)?;
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
    let u = auth(&s, &h)?;
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
    let u = auth(&s, &h)?;
    s.disown(u.id, pkgbase(&s, &name)?)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn adopt(
    State(s): St,
    h: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let u = auth(&s, &h)?;
    s.adopt(u.id, pkgbase(&s, &name)?)?;
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
    let u = auth(&s, &h)?;
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
