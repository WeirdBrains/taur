//! Static web UI (plain HTML/CSS/JS, embedded in the binary). Page routes all serve
//! `index.html`; the client-side router renders the page and talks to the JSON API
//! with a bearer token stored in localStorage.

use axum::http::header;
use axum::response::{IntoResponse, Response};
use std::sync::OnceLock;

const INDEX_TEMPLATE: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE: &str = include_str!("../web/style.css");
const FAVICON: &[u8] = include_bytes!("../web/favicon.ico");
const LOGO_DARK: &[u8] = include_bytes!("../web/logo-dark.svg");
const VERSION_PLACEHOLDER: &str = "__TAUR_VERSION__";

fn index_html() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| INDEX_TEMPLATE.replace(VERSION_PLACEHOLDER, env!("CARGO_PKG_VERSION")))
        .as_str()
}

/// Paths rendered by the web UI (`{x}` = one segment).
pub const PAGES: [&str; 22] = [
    "/",
    "/packages",
    "/packages/{name}",
    "/packages/{name}/revisions/{rev}",
    "/users/{name}",
    "/login",
    "/register",
    "/verify",
    "/reset",
    "/account",
    "/orgs",
    "/orgs/{name}",
    "/requests",
    "/audit",
    "/admin",
    "/registries",
    "/registries/{name}",
    "/mirrors",
    "/mirrors/{name}",
    "/docs",
    "/docs/{slug}",
    "/upload",
];

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

fn asset(body: &'static str, ctype: &'static str) -> Response {
    asset_bytes(body.as_bytes(), ctype)
}

fn asset_bytes(body: &'static [u8], ctype: &'static str) -> Response {
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
    asset(index_html(), "text/html; charset=utf-8")
}

pub async fn app_js() -> Response {
    asset(APP_JS, "text/javascript; charset=utf-8")
}

pub async fn style() -> Response {
    asset(STYLE, "text/css; charset=utf-8")
}

pub async fn favicon() -> Response {
    asset_bytes(FAVICON, "image/x-icon")
}

pub async fn logo_dark() -> Response {
    asset_bytes(LOGO_DARK, "image/svg+xml")
}

#[cfg(test)]
mod tests {
    #[test]
    fn index_template_has_a_single_version_placeholder() {
        let n = super::INDEX_TEMPLATE
            .matches(super::VERSION_PLACEHOLDER)
            .count();
        assert_eq!(
            n,
            1,
            "footer template must include {ph} once",
            ph = super::VERSION_PLACEHOLDER
        );
        let html = super::index_html();
        assert!(html.contains(env!("CARGO_PKG_VERSION")));
        assert!(!html.contains(super::VERSION_PLACEHOLDER));
        assert!(html.contains(r#"href="/favicon.ico""#));
    }

    #[test]
    fn favicon_is_a_windows_icon() {
        assert_eq!(
            &super::FAVICON[..4],
            &[0, 0, 1, 0],
            "favicon.ico must be an ICO"
        );
        assert!(super::FAVICON.len() > 16);
    }

    #[test]
    fn index_brand_uses_the_existing_dark_logo_and_links_home() {
        let html = super::INDEX_TEMPLATE;
        assert!(html.contains(r#"href="/" class="brand""#));
        assert!(html.contains(r#"src="/static/logo-dark.svg""#));
        assert!(html.contains(r#"alt="TAUR mark: an angular tau, not the Arch Linux logo""#));
        assert!(html.contains(r#"aria-label="TAUR home""#));
    }

    #[test]
    fn logo_dark_is_the_existing_site_mark() {
        let svg = std::str::from_utf8(super::LOGO_DARK).unwrap();
        assert!(svg.contains("<svg"), "logo-dark.svg must be an SVG");
        assert!(svg.contains(r#"viewBox="0 0 64 64""#));
        assert!(
            svg.contains(r##"fill="#d6e6f2""##)
                && svg.contains(r##"fill="#8fe9f2""##)
                && svg.contains(r##"fill="#3fd8e8""##),
            "must keep the existing light-on-dark fills, not a new mark"
        );
    }
}
