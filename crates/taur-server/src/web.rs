//! Static web UI (plain HTML/CSS/JS, embedded in the binary). Page routes all serve
//! `index.html`; the client-side router renders the page and talks to the JSON API
//! with a bearer token stored in localStorage.

use axum::http::header;
use axum::response::{IntoResponse, Response};

const INDEX: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE: &str = include_str!("../web/style.css");

/// Paths rendered by the web UI (`{x}` = one segment).
pub const PAGES: [&str; 11] = [
    "/",
    "/packages",
    "/packages/{name}",
    "/login",
    "/register",
    "/verify",
    "/reset",
    "/account",
    "/orgs",
    "/orgs/{name}",
    "/requests",
];

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

fn asset(body: &'static str, ctype: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, ctype),
            (header::CACHE_CONTROL, "no-cache"),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        body,
    )
        .into_response()
}

pub async fn index() -> Response {
    asset(INDEX, "text/html; charset=utf-8")
}

pub async fn app_js() -> Response {
    asset(APP_JS, "text/javascript; charset=utf-8")
}

pub async fn style() -> Response {
    asset(STYLE, "text/css; charset=utf-8")
}
