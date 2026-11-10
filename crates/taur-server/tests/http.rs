use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use taur_core::Store;
use tower::ServiceExt;

fn setup() -> (tempfile::TempDir, axum::Router) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur");
    s.import_dir(&fx, None).unwrap();
    (d, taur_server::app(Arc::new(s)))
}

async fn call(app: &axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let r = app.clone().oneshot(req).await.unwrap();
    let st = r.status();
    (
        st,
        r.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Vec<u8>) {
    call(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

async fn get_json(app: &axum::Router, uri: &str) -> Value {
    let (st, b) = get(app, uri).await;
    assert_eq!(st, StatusCode::OK, "{uri}");
    serde_json::from_slice(&b).unwrap()
}

fn send(method: &str, uri: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        if method != "GET" && method != "HEAD" {
            b = b.header("x-csrf-token", t);
        }
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn rpc_endpoints() {
    let (_d, app) = setup();
    let v = get_json(&app, "/rpc/v5/info?arg[]=hello-taur&arg[]=hello-taur-docs").await;
    assert_eq!(v["resultcount"], 2);
    assert_eq!(v["type"], "multiinfo");
    let v = get_json(&app, "/rpc/v5/info/hello-taur").await;
    assert_eq!(v["results"][0]["Version"], "1:1.2.3-1");
    let v = get_json(&app, "/rpc/v5/search/tiny").await;
    assert_eq!(v["resultcount"], 1);
    let v = get_json(&app, "/rpc/v5/search/glibc?by=depends").await;
    assert_eq!(v["results"][0]["Name"], "hello-taur");
    let v = get_json(&app, "/rpc/v5/suggest/hello").await;
    assert_eq!(v, json!(["hello-taur", "hello-taur-docs"]));
    let v = get_json(&app, "/rpc/v5/suggest-pkgbase/hello").await;
    assert_eq!(v, json!(["hello-taur"]));
    let v = get_json(&app, "/rpc?v=5&type=info&arg[]=hello-taur").await;
    assert_eq!(v["resultcount"], 1);
    let v = get_json(&app, "/rpc/?v=5&type=multiinfo&arg=hello-taur-docs").await;
    assert_eq!(v["resultcount"], 1);
    let v = get_json(&app, "/rpc?v=5&type=search&by=name&arg=hello").await;
    assert_eq!(v["resultcount"], 2);
    let v = get_json(&app, "/rpc?v=4&type=info&arg=x").await;
    assert_eq!(v["error"], "Invalid version specified.");
    let req = Request::post("/rpc/v5/info")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("arg[]=hello-taur"))
        .unwrap();
    let (_, b) = call(&app, req).await;
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["resultcount"], 1);
}

#[tokio::test]
async fn rpc_rate_limit_keys_on_the_socket_peer() {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;
    use std::sync::Arc;
    let d = tempfile::tempdir().unwrap();
    let s = Arc::new(Store::open(d.path()).unwrap());
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur");
    s.import_dir(&fx, None).unwrap();
    s.configure_rate_limits(&policy(1, ""));
    let app = taur_server::app(s);
    let once = |ip: [u8; 4], forwarded: &str| {
        let mut req = Request::get("/rpc?v=5&type=info&arg[]=hello-taur")
            .header("x-forwarded-for", forwarded)
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((ip, 9))));
        req
    };
    let (st, _) = call(&app, once([10, 1, 0, 1], "1.2.3.4")).await;
    assert_eq!(st, StatusCode::OK);
    let (st, b, retry) = call_retry(&app, once([10, 1, 0, 1], "8.8.8.8")).await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["error"], "Rate limit reached");
    assert_eq!(v["type"], "error");
    let retry: u64 = retry.unwrap().parse().unwrap();
    assert!((1..=60).contains(&retry), "{retry}");
    let (st, _) = call(&app, once([10, 9, 0, 2], "1.2.3.4")).await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
async fn trusted_proxy_chain_and_auth_and_git_limits() {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;
    use std::sync::Arc;
    let d = tempfile::tempdir().unwrap();
    let s = Arc::new(Store::open(d.path()).unwrap());
    s.configure_rate_limits(&policy(1, "10.0.0.0/8"));
    let app = taur_server::app(s);
    let hit = |ip: [u8; 4], forwarded: &str, path: &str| {
        let mut req = Request::get(path)
            .header("x-forwarded-for", forwarded)
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((ip, 9))));
        req
    };
    let (st, _) = call(
        &app,
        hit(
            [10, 1, 0, 1],
            "9.9.9.9, 203.0.113.5",
            "/rpc?v=5&type=info&arg=x",
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        &app,
        hit(
            [10, 1, 0, 1],
            "9.9.9.9, 203.0.113.6",
            "/rpc?v=5&type=info&arg=x",
        ),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "a different forwarded client has its own bucket"
    );
    let (st, _) = call(
        &app,
        hit(
            [10, 1, 0, 1],
            "1.1.1.1, 203.0.113.5",
            "/rpc?v=5&type=info&arg=x",
        ),
    )
    .await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
    let (st, _) = call(
        &app,
        hit([198, 51, 100, 8], "203.0.113.5", "/rpc?v=5&type=info&arg=x"),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "an untrusted peer ignores X-Forwarded-For"
    );

    let login = |ip: [u8; 4]| {
        let mut req = Request::post("/api/v1/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"name":"nobody","password":"nope"}"#))
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((ip, 9))));
        req
    };
    let (st, _) = call(&app, login([203, 0, 113, 9])).await;
    assert_ne!(st, StatusCode::TOO_MANY_REQUESTS);
    let (st, b) = call(&app, login([203, 0, 113, 9])).await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["error"], "Rate limit reached");

    let git = |ip: [u8; 4]| {
        let mut req = Request::get("/hello.git/info/refs?service=git-upload-pack")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((ip, 9))));
        req
    };
    let (st, _) = call(&app, git([203, 0, 113, 20])).await;
    assert_ne!(st, StatusCode::TOO_MANY_REQUESTS);
    let (st, b) = call(&app, git([203, 0, 113, 20])).await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
    assert!(b.starts_with(b"Rate limit reached"));
}

fn policy(max: u32, proxies: &str) -> taur_core::RatePolicy {
    use std::time::Duration;
    use taur_core::LimitClass;
    use taur_core::ratelimit::ClassLimit;
    let class = |class, max| ClassLimit {
        class,
        max,
        window: Duration::from_secs(60),
    };
    taur_core::RatePolicy {
        classes: vec![
            class(LimitClass::Rpc, max),
            class(LimitClass::Auth, max),
            class(LimitClass::Git, max),
        ],
        trusted_proxies: taur_core::ratelimit::parse_trusted_proxies(proxies).unwrap(),
        max_keys: 100,
    }
}

async fn call_retry(
    app: &axum::Router,
    req: Request<Body>,
) -> (StatusCode, Vec<u8>, Option<String>) {
    let r = app.clone().oneshot(req).await.unwrap();
    let retry = r
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let status = r.status();
    let b = r.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, b, retry)
}

#[tokio::test]
async fn rpc_jsonp_quirk_json_post_and_cgit_plain() {
    let (_d, app) = setup();
    // JSONP (aurweb wraps the payload as `/**/callback(...)`).
    let r = app
        .clone()
        .oneshot(
            Request::get("/rpc?v=5&type=info&arg[]=hello-taur&callback=cb")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        r.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    let body =
        String::from_utf8(r.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(body.starts_with("/**/cb("), "{body}");
    assert!(body.contains("hello-taur"), "{body}");
    let v = get_json(
        &app,
        "/rpc?v=5&type=info&arg=hello-taur&callback=bad%20name",
    )
    .await;
    assert_eq!(v["error"], "Invalid callback name.");

    // aurweb: a final `arg` replaces every earlier arg[]; a final `arg[]` keeps
    // only the trailing arg[] run.
    let v = get_json(
        &app,
        "/rpc?v=5&type=info&arg[]=hello-taur-docs&arg=hello-taur",
    )
    .await;
    assert_eq!(v["resultcount"], 1);
    assert_eq!(v["results"][0]["Name"], "hello-taur");
    let v = get_json(
        &app,
        "/rpc?v=5&type=info&arg=hello-taur&arg[]=hello-taur-docs",
    )
    .await;
    assert_eq!(v["resultcount"], 1);
    assert_eq!(v["results"][0]["Name"], "hello-taur-docs");

    // The query strings yay sends (`--aurrpcurl …/rpc?`).
    let v = get_json(&app, "/rpc?v=5&type=search&by=name-desc&arg=hello").await;
    assert_eq!(v["type"], "search");
    assert_eq!(v["resultcount"], 2);
    let v = get_json(&app, "/rpc?v=5&type=info&arg[]=hello-taur").await;
    assert_eq!(v["results"][0]["PackageBase"], "hello-taur");
    let v = get_json(&app, "/rpc?v=5&type=suggest&arg=hello").await;
    assert_eq!(v[0], "hello-taur");

    let req = Request::post("/rpc/v5/search")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"arg":"hello","by":"name"}"#))
        .unwrap();
    let (st, b) = call(&app, req).await;
    assert_eq!(st, StatusCode::OK);
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["resultcount"], 2);
    let req = Request::post("/rpc/v5/info")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"arg":"hello-taur"}"#))
        .unwrap();
    let (st, b) = call(&app, req).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["error"], "the 'arg' parameter must be of array type");
    let req = Request::post("/rpc/v5/info")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"arg":["hello-taur","hello-taur-docs"]}"#))
        .unwrap();
    let (st, b) = call(&app, req).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&b).unwrap()["resultcount"],
        2
    );

    let (st, b) = get(&app, "/cgit/aur.git/plain/PKGBUILD?h=hello-taur").await;
    assert_eq!(st, StatusCode::OK);
    let text = String::from_utf8(b).unwrap();
    assert!(text.contains("pkgbase=hello-taur"), "{text}");
    let (st, b) = get(&app, "/cgit/aur.git/plain/.SRCINFO?h=hello-taur&id=HEAD").await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        String::from_utf8(b)
            .unwrap()
            .contains("pkgbase = hello-taur")
    );
    assert_eq!(
        get(&app, "/cgit/aur.git/plain/PKGBUILD").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/cgit/aur.git/plain/missing?h=hello-taur")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/cgit/aur.git/plain/foo/../../PKGBUILD?h=hello-taur")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn git_snapshot_and_dumps() {
    let (_d, app) = setup();
    let (st, b) = get(&app, "/cgit/aur.git/snapshot/hello-taur.tar.gz").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(&b[..2], &[0x1f, 0x8b]);
    assert_eq!(
        get(&app, "/cgit/aur.git/snapshot/nope.tar.gz").await.0,
        StatusCode::NOT_FOUND
    );
    let (st, b) = get(&app, "/hello-taur.git/info/refs?service=git-upload-pack").await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        String::from_utf8(b.clone())
            .unwrap()
            .contains("refs/heads/master")
    );
    // paru clones `/{pkgbase}` with no `.git` suffix; advertisement must match.
    let (st, bare) = get(&app, "/hello-taur/info/refs?service=git-upload-pack").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(bare, b);
    assert_eq!(
        get(&app, "/hello-taur/not-a-git-path").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/hello-taur.git/config").await.0,
        StatusCode::NOT_FOUND
    );
    let (_, b) = get(&app, "/packages.gz").await;
    let mut s = String::new();
    std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&b[..]), &mut s).unwrap();
    assert_eq!(s, "hello-taur\nhello-taur-docs\n");
    assert_eq!(get(&app, "/pkgbase.gz").await.0, StatusCode::OK);
    assert_eq!(
        get(&app, "/packages-meta-v1.json.gz").await.0,
        StatusCode::OK
    );
    let (st, b) = get(&app, "/packages-meta-ext-v1.json.gz").await;
    assert_eq!(st, StatusCode::OK);
    let mut s = String::new();
    std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&b[..]), &mut s).unwrap();
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert!(v[0]["License"].is_array());
}

#[tokio::test]
async fn notify_and_pin_api() {
    let (d, app) = setup();
    let a = token(&app, d.path(), "alice").await;
    let b = token(&app, d.path(), "bob").await;
    let store = Store::open(d.path()).unwrap();
    store
        .set_admin(store.user_id("alice").unwrap(), true)
        .unwrap();
    let n = json!(null);
    let u = "/api/v1/packages/hello-taur/notify";
    assert_eq!(
        call(&app, send("POST", u, Some(&b), n.clone())).await.0,
        StatusCode::NO_CONTENT
    );
    let (_, body) = call(&app, send("GET", u, Some(&b), n.clone())).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["subscribed"],
        true
    );
    let (_, body) = call(
        &app,
        send(
            "POST",
            "/api/v1/packages/hello-taur/comments",
            Some(&a),
            json!({"content": "pin me"}),
        ),
    )
    .await;
    let id = serde_json::from_slice::<Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap();
    let pin = format!("/api/v1/comments/{id}/pin");
    assert_eq!(
        call(&app, send("POST", &pin, Some(&b), n.clone())).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, send("POST", &pin, Some(&a), n.clone())).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get_json(&app, "/api/v1/packages/hello-taur/comments").await[0]["pinned"],
        true
    );
    assert_eq!(
        call(&app, send("DELETE", u, Some(&b), n.clone())).await.0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn account_api() {
    let (_d, app) = setup();
    let reg = json!({"name": "alice", "email": "a@example.com", "password": "correct horse"});
    let (st, b) = call(&app, send("POST", "/api/v1/register", None, reg.clone())).await;
    assert_eq!(st, StatusCode::CREATED);
    assert!(!String::from_utf8_lossy(&b).contains("correct horse"));
    assert_eq!(
        call(&app, send("POST", "/api/v1/register", None, reg))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let bad = json!({"name": "alice", "password": "nope-nope"});
    assert_eq!(
        call(&app, send("POST", "/api/v1/login", None, bad)).await.0,
        StatusCode::UNAUTHORIZED
    );
    let good = json!({"name": "alice", "password": "correct horse"});
    let (st, b) = call(&app, send("POST", "/api/v1/login", None, good.clone())).await;
    assert_eq!(st, StatusCode::OK);
    let token = serde_json::from_slice::<Value>(&b).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, b) = call(
        &app,
        send("GET", "/api/v1/account", Some(&token), json!(null)),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&b).unwrap()["name"],
        "alice"
    );
    assert_eq!(
        call(&app, send("GET", "/api/v1/account", None, json!(null)))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            send("POST", "/api/v1/logout", Some(&token), json!(null))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send("GET", "/api/v1/account", Some(&token), json!(null))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (_, b) = call(&app, send("POST", "/api/v1/login", None, good)).await;
    let token = serde_json::from_slice::<Value>(&b).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let wrong = json!({"password": "wrong-wrong"});
    assert_eq!(
        call(&app, send("DELETE", "/api/v1/account", Some(&token), wrong))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let right = json!({"password": "correct horse"});
    assert_eq!(
        call(&app, send("DELETE", "/api/v1/account", Some(&token), right))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send("GET", "/api/v1/account", Some(&token), json!(null))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

async fn token(app: &axum::Router, home: &std::path::Path, name: &str) -> String {
    let email = format!("{name}@x");
    let reg = json!({"name": name, "email": email, "password": "longenough"});
    call(app, send("POST", "/api/v1/register", None, reg)).await;
    let t =
        taur_core::mail::FileMailer::find_token(&home.join("outbox"), &email, "verify").unwrap();
    let (st, _) = call(
        app,
        send("POST", "/api/v1/verify", None, json!({"token": t})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (_, b) = call(
        app,
        send(
            "POST",
            "/api/v1/login",
            None,
            json!({"name": name, "password": "longenough"}),
        ),
    )
    .await;
    serde_json::from_slice::<Value>(&b).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn org_and_package_api() {
    let (d, app) = setup();
    // The first account is the site owner. alice stays an ordinary maintainer.
    call(
        &app,
        send(
            "POST",
            "/api/v1/register",
            None,
            json!({"name": "owner", "email": "owner@x", "password": "longenough"}),
        ),
    )
    .await;
    let a = token(&app, d.path(), "alice").await;
    let b = token(&app, d.path(), "bob").await;
    let c = token(&app, d.path(), "carol").await;
    let n = json!(null);
    let (st, body) = call(
        &app,
        send("POST", "/api/v1/orgs", Some(&a), json!({"name": "brains"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["members"][0]["role"],
        "owner"
    );
    assert_eq!(
        call(
            &app,
            send("GET", "/api/v1/orgs/brains", Some(&b), n.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/orgs/brains/members/bob",
                Some(&a),
                json!({"role": "admin"})
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // admin adds a contributor but cannot create owners
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/orgs/brains/members/carol",
                Some(&b),
                json!({"role": "owner"})
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/orgs/brains/members/carol",
                Some(&b),
                json!({"role": "contributor"})
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/orgs/brains/members/alice",
                Some(&c),
                json!({"role": "contributor"})
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, body) = call(&app, send("GET", "/api/v1/orgs", Some(&c), n.clone())).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!([{"name": "brains", "role": "contributor"}])
    );
    // A normal user files an adoption request. A site admin grants it.
    let (st, body) = call(
        &app,
        send(
            "POST",
            "/api/v1/packages/hello-taur/adopt",
            Some(&a),
            n.clone(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let rid = serde_json::from_slice::<Value>(&body).unwrap()["request"]
        .as_i64()
        .unwrap();
    let store = Store::open(d.path()).unwrap();
    store
        .set_admin(store.user_id("bob").unwrap(), true)
        .unwrap();
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                &format!("/api/v1/requests/{rid}/accept"),
                Some(&b),
                json!({"comment": "granted"}),
            ),
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    store
        .set_admin(store.user_id("bob").unwrap(), false)
        .unwrap();
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/packages/hello-taur/owner",
                Some(&c),
                json!({"org": "brains"})
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/packages/hello-taur/owner",
                Some(&a),
                json!({"org": "brains"})
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (_, body) = call(&app, send("GET", "/api/v1/packages", Some(&c), n.clone())).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()[0]["org"],
        "brains"
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/packages/hello-taur/owner",
                Some(&c),
                json!({"user": "carol"})
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/packages/hello-taur/owner",
                Some(&c),
                json!({})
            )
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/packages/hello-taur/disown",
                Some(&a),
                n.clone()
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/packages/hello-taur/disown",
                Some(&c),
                n.clone()
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send("POST", "/api/v1/packages/nope/adopt", Some(&a), n.clone())
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            send(
                "DELETE",
                "/api/v1/orgs/brains/members/carol",
                Some(&b),
                n.clone()
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send("DELETE", "/api/v1/orgs/brains", Some(&b), n.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send("DELETE", "/api/v1/orgs/brains", Some(&a), n.clone())
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, send("GET", "/api/v1/orgs/brains", Some(&a), n))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[test]
fn taur_man_pages_up_to_date() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../man");
    let update = std::env::var_os("TAUR_UPDATE_MAN").is_some();
    for (name, roff) in taur_server::cli::man_pages() {
        let path = dir.join(&name);
        if update {
            std::fs::write(&path, &roff).unwrap();
        } else {
            let current = std::fs::read(&path).unwrap_or_default();
            assert!(
                current == roff,
                "{name} is out of date: run TAUR_UPDATE_MAN=1 cargo test"
            );
        }
    }
}

#[tokio::test]
async fn resend_reports_an_unreachable_mail_server() {
    struct Down;
    impl taur_core::mail::Mailer for Down {
        fn send(&self, _: &str, _: &str, _: &str) -> taur_core::Result<()> {
            Err(taur_core::Error::Mail(
                "smtp.gmail.com:465: Network is unreachable (os error 101)".into(),
            ))
        }
    }
    let d = tempfile::tempdir().unwrap();
    let s = Arc::new(Store::open(d.path()).unwrap());
    s.set_mailer(Arc::new(Down));
    let app = taur_server::app(s);
    let (st, _) = call(
        &app,
        send(
            "POST",
            "/api/v1/register",
            None,
            json!({"name": "ada", "email": "ada@x", "password": "longenough"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _) = call(
        &app,
        send(
            "POST",
            "/api/v1/register",
            None,
            json!({"name": "erin", "email": "erin@x", "password": "longenough"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (_, body) = call(
        &app,
        send(
            "POST",
            "/api/v1/login",
            None,
            json!({"name": "erin", "password": "longenough"}),
        ),
    )
    .await;
    let token = serde_json::from_slice::<Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, body) = call(
        &app,
        send("POST", "/api/v1/verify/resend", Some(&token), json!(null)),
    )
    .await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("mail server unreachable"), "{text}");
    assert!(
        !text.contains("secret") && !text.contains("os error"),
        "{text}"
    );
}

#[tokio::test]
async fn admin_pages_follow_the_same_roles_as_the_api() {
    let (d, app) = setup();
    let outbox = d.path().join("outbox");
    let owner = token(&app, d.path(), "ada").await;
    let plain = token(&app, d.path(), "erin").await;
    let (st, body) = call(
        &app,
        send(
            "GET",
            "/api/v1/admin/users?q=ada",
            Some(&owner),
            json!(null),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("ada@x"), "{text}");
    assert_eq!(
        call(&app, send("GET", "/api/v1/admin/users", None, json!(null)))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            send("GET", "/api/v1/admin/users", Some(&plain), json!(null))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/admin/moderators/erin",
                Some(&plain),
                json!({"on": true}),
            ),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/admin/moderators/erin",
                Some(&owner),
                json!({"on": true}),
            ),
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let mod_token = {
        let (_, b) = call(
            &app,
            send(
                "POST",
                "/api/v1/login",
                None,
                json!({"name": "erin", "password": "longenough"}),
            ),
        )
        .await;
        serde_json::from_slice::<Value>(&b).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (st, body) = call(
        &app,
        send(
            "GET",
            "/api/v1/admin/users?q=ada",
            Some(&mod_token),
            json!(null),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    assert!(!text.contains("ada@x"), "{text}");
    assert!(text.contains("ada"), "{text}");
    for (method, path, body) in [
        ("POST", "/api/v1/admin/users/erin/verify", json!(null)),
        ("PUT", "/api/v1/admin/users/erin/admin", json!({"on": true})),
        ("POST", "/api/v1/registries", json!({"name": "vendor"})),
        (
            "POST",
            "/api/v1/mirrors",
            json!({"name": "aur", "url": "http://127.0.0.1:9/recipes"}),
        ),
        ("GET", "/api/v1/audit", json!(null)),
    ] {
        assert_eq!(
            call(&app, send(method, path, Some(&mod_token), body))
                .await
                .0,
            StatusCode::FORBIDDEN,
            "{method} {path}"
        );
    }
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/moderation/users/ada/reject",
                Some(&mod_token),
                json!({"reason": "no"}),
            ),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    call(
        &app,
        send(
            "POST",
            "/api/v1/register",
            None,
            json!({"name": "gary", "email": "gary@x", "password": "longenough"}),
        ),
    )
    .await;
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/admin/users/gary/verify",
                Some(&owner),
                json!(null)
            ),
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/admin/users/gary/password-reset",
                Some(&owner),
                json!(null),
            ),
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let reset = taur_core::mail::FileMailer::find_token(&outbox, "gary@x", "reset").unwrap();
    let (st, body) = call(
        &app,
        send(
            "POST",
            "/api/v1/admin/users/gary/password-reset",
            Some(&owner),
            json!(null),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    assert!(!String::from_utf8(body).unwrap().contains(&reset));
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/admin/users/ada/admin",
                Some(&owner),
                json!({"on": false}),
            ),
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (st, sessions) = call(
        &app,
        send("GET", "/api/v1/sessions", Some(&owner), json!(null)),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let sessions: Vec<Value> = serde_json::from_slice(&sessions).unwrap();
    assert!(sessions.iter().any(|s| s["current"] == true));
    assert!(!String::from_utf8_lossy(&serde_json::to_vec(&sessions).unwrap()).contains(&owner));
    let no_csrf = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/users/gary/resend")
        .header(header::AUTHORIZATION, format!("Bearer {owner}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (st, body) = call(&app, no_csrf).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert!(String::from_utf8(body).unwrap().contains("CSRF"));
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/registries",
                Some(&owner),
                json!({"name": "vendor"})
            ),
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/mirrors",
                Some(&owner),
                json!({"name": "lab", "url": "http://127.0.0.1:9/recipes"}),
            ),
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/mirrors/lab/select",
                Some(&owner),
                json!({"pkgbases": ["demo"]}),
            ),
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (st, body) = call(
        &app,
        send("GET", "/api/v1/mirrors/lab", Some(&owner), json!(null)),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(String::from_utf8(body).unwrap().contains("demo"));
    assert_eq!(
        call(
            &app,
            send("DELETE", "/api/v1/mirrors/lab", Some(&owner), json!(null)),
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn guides_follow_the_role() {
    let (d, app) = setup();
    let (st, body) = get(&app, "/api/v1/docs").await;
    assert_eq!(st, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("\"slug\":\"publish\""), "{text}");
    assert!(!text.contains("operating"), "{text}");
    assert!(!text.contains("administration"), "{text}");
    assert!(!text.contains("\"slug\":\"moderation\""), "{text}");
    let (st, body) = get(&app, "/api/v1/docs/publish").await;
    assert_eq!(st, StatusCode::OK);
    assert!(String::from_utf8(body).unwrap().contains("git push"));
    assert_eq!(
        get(&app, "/api/v1/docs/operating").await.0,
        StatusCode::NOT_FOUND
    );
    let owner = token(&app, d.path(), "ada").await;
    assert_eq!(
        call(
            &app,
            send("GET", "/api/v1/docs/operating", Some(&owner), json!(null)),
        )
        .await
        .0,
        StatusCode::OK
    );
    let admin = token(&app, d.path(), "erin").await;
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/admin/users/erin/admin",
                Some(&owner),
                json!({"on": true}),
            ),
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send("GET", "/api/v1/docs/operating", Some(&admin), json!(null)),
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            send(
                "GET",
                "/api/v1/docs/administration",
                Some(&admin),
                json!(null),
            ),
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/registries",
                Some(&admin),
                json!({"name": "vendor"}),
            ),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/admin/users/ada/admin",
                Some(&admin),
                json!({"on": false}),
            ),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let moderator = token(&app, d.path(), "modd").await;
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/admin/moderators/modd",
                Some(&admin),
                json!({"on": true}),
            ),
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send(
                "GET",
                "/api/v1/docs/moderation",
                Some(&moderator),
                json!(null)
            ),
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            send(
                "GET",
                "/api/v1/docs/administration",
                Some(&moderator),
                json!(null),
            ),
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/admin/users/ada/owner",
                Some(&owner),
                json!({"on": false}),
            ),
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn verification_and_reset_api() {
    let (d, app) = setup();
    let outbox = d.path().join("outbox");
    // The first account on a fresh server is the site admin. dave is the ordinary one.
    call(
        &app,
        send(
            "POST",
            "/api/v1/register",
            None,
            json!({"name": "ada", "email": "ada@x", "password": "longenough"}),
        ),
    )
    .await;
    let reg = json!({"name": "dave", "email": "dave@x", "password": "longenough"});
    call(&app, send("POST", "/api/v1/register", None, reg)).await;
    let (_, b) = call(
        &app,
        send(
            "POST",
            "/api/v1/login",
            None,
            json!({"name": "dave", "password": "longenough"}),
        ),
    )
    .await;
    let t = serde_json::from_slice::<Value>(&b).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    // unverified accounts can't manage anything
    let (st, b) = call(
        &app,
        send("POST", "/api/v1/orgs", Some(&t), json!({"name": "x-org"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert!(String::from_utf8_lossy(&b).contains("not verified"));
    assert_eq!(
        call(
            &app,
            send("POST", "/api/v1/verify/resend", Some(&t), json!(null))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let vt = taur_core::mail::FileMailer::find_token(&outbox, "dave@x", "verify").unwrap();
    assert_eq!(
        call(
            &app,
            send("POST", "/api/v1/verify", None, json!({"token": vt.clone()}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            send("POST", "/api/v1/verify", None, json!({"token": vt}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            send("POST", "/api/v1/orgs", Some(&t), json!({"name": "x-org"}))
        )
        .await
        .0,
        StatusCode::CREATED
    );
    // reset: unknown email still 202; real one mails a token
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/password-reset",
                None,
                json!({"email": "nobody@x"})
            )
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/password-reset",
                None,
                json!({"email": "dave@x"})
            )
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let rt = taur_core::mail::FileMailer::find_token(&outbox, "dave@x", "reset").unwrap();
    let body = json!({"token": rt.clone(), "password": "brand new pass"});
    assert_eq!(
        call(
            &app,
            send("POST", "/api/v1/password-reset/confirm", None, body.clone())
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            send("POST", "/api/v1/password-reset/confirm", None, body)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&app, send("GET", "/api/v1/account", Some(&t), json!(null)))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let ok = json!({"name": "dave", "password": "brand new pass"});
    assert_eq!(
        call(&app, send("POST", "/api/v1/login", None, ok)).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn community_api() {
    let (d, app) = setup();
    let a = token(&app, d.path(), "alice").await;
    let b = token(&app, d.path(), "bob").await;
    let n = json!(null);
    let store = Store::open(d.path()).unwrap();
    let alice = store.user_id("alice").unwrap();
    store.set_admin(alice, true).unwrap();
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/packages/hello-taur/vote",
                Some(&b),
                n.clone()
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get_json(&app, "/rpc/v5/info/hello-taur").await["results"][0]["NumVotes"],
        1
    );
    assert_eq!(
        call(
            &app,
            send("POST", "/api/v1/packages/hello-taur/vote", None, n.clone())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/packages/hello-taur/flag",
                Some(&b),
                json!({"comment": "old"})
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(get_json(&app, "/rpc/v5/info/hello-taur").await["results"][0]["OutOfDate"].is_i64());
    assert_eq!(
        get_json(&app, "/api/v1/packages/hello-taur/flag").await["flagger"],
        "bob"
    );
    assert_eq!(
        call(
            &app,
            send(
                "DELETE",
                "/api/v1/packages/hello-taur/flag",
                Some(&b),
                n.clone()
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (st, body) = call(
        &app,
        send(
            "POST",
            "/api/v1/packages/hello-taur/comments",
            Some(&b),
            json!({"content": "hi"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let id = serde_json::from_slice::<Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        get_json(&app, "/api/v1/packages/hello-taur/comments").await[0]["content"],
        "hi"
    );
    assert_eq!(
        call(
            &app,
            send(
                "PATCH",
                &format!("/api/v1/comments/{id}"),
                Some(&a),
                json!({"content": "x"})
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "PATCH",
                &format!("/api/v1/comments/{id}"),
                Some(&b),
                json!({"content": "hello"})
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            send(
                "DELETE",
                &format!("/api/v1/comments/{id}"),
                Some(&a),
                n.clone()
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // alice (site admin) sets co-maintainers on the orphaned fixture
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/packages/hello-taur/comaintainers",
                Some(&b),
                json!({"users": ["bob"]})
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "PUT",
                "/api/v1/packages/hello-taur/comaintainers",
                Some(&a),
                json!({"users": ["bob"]})
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get_json(&app, "/rpc/v5/info/hello-taur").await["results"][0]["CoMaintainers"],
        json!(["bob"])
    );
    let (st, body) = call(
        &app,
        send(
            "POST",
            "/api/v1/packages/hello-taur/requests",
            Some(&b),
            json!({"kind": "deletion", "comment": "obsolete"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let rid = serde_json::from_slice::<Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap();
    let (_, body) = call(&app, send("GET", "/api/v1/requests", Some(&b), n.clone())).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()[0]["status"],
        "pending"
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                &format!("/api/v1/requests/{rid}/accept"),
                Some(&b),
                json!({})
            )
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                &format!("/api/v1/requests/{rid}/accept"),
                Some(&a),
                json!({"comment": "done"})
            )
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get_json(&app, "/rpc/v5/info/hello-taur").await["resultcount"],
        0
    );
    assert_eq!(
        get(&app, "/hello-taur.git/info/refs").await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn web_ui_is_served() {
    let (d, app) = setup();
    for page in [
        "/",
        "/packages/hello-taur",
        "/users/alice",
        "/login",
        "/register",
        "/verify?token=abc",
        "/reset?token=abc",
        "/account",
        "/orgs",
        "/orgs/x",
        "/requests",
        "/docs",
        "/docs/publish",
        "/upload",
        "/?q=hello",
    ] {
        let r = app
            .clone()
            .oneshot(Request::get(page).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{page}");
        assert!(
            r.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert!(r.headers().contains_key(header::CONTENT_SECURITY_POLICY));
        let body = r.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&body).contains(r#"<script src="/static/app.js">"#));
    }
    for (path, ctype) in [
        ("/static/app.js", "text/javascript"),
        ("/static/style.css", "text/css"),
    ] {
        let r = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(
            r.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with(ctype)
        );
    }
    let (_, js) = get(&app, "/static/app.js").await;
    let js = String::from_utf8(js).unwrap();
    // every API path the UI calls exists
    for p in [
        "/api/v1/login",
        "/api/v1/register",
        "/api/v1/verify",
        "/api/v1/password-reset/confirm",
        "/api/v1/ssh-keys",
        "/api/v1/orgs",
        "/api/v1/requests",
        "/api/v1/docs",
        "/api/v1/packages/upload",
        "/upload",
    ] {
        assert!(js.contains(p), "{p}");
    }
    assert!(!js.contains("innerHTML"), "UI must not inject HTML");
    assert!(
        !js.contains("document.cookie"),
        "UI uses bearer tokens, not cookies"
    );
    // vote status endpoint used by the package page
    let t = token(&app, d.path(), "alice").await;
    let (_, b) = call(
        &app,
        send(
            "GET",
            "/api/v1/packages/hello-taur/vote",
            Some(&t),
            json!(null),
        ),
    )
    .await;
    assert_eq!(
        serde_json::from_slice::<Value>(&b).unwrap(),
        json!({"voted": false})
    );
    call(
        &app,
        send(
            "POST",
            "/api/v1/packages/hello-taur/vote",
            Some(&t),
            json!(null),
        ),
    )
    .await;
    let (_, b) = call(
        &app,
        send(
            "GET",
            "/api/v1/packages/hello-taur/vote",
            Some(&t),
            json!(null),
        ),
    )
    .await;
    assert_eq!(
        serde_json::from_slice::<Value>(&b).unwrap(),
        json!({"voted": true})
    );
    // git and RPC still win over page routes
    assert_eq!(
        get(&app, "/hello-taur.git/info/refs").await.0,
        StatusCode::OK
    );
}

async fn mcp(app: &axum::Router, token: Option<&str>, msg: Value) -> (StatusCode, Value) {
    let (st, b) = call(app, send("POST", "/mcp", token, msg)).await;
    (st, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

fn tool(id: i64, name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

#[tokio::test]
async fn mcp_endpoint() {
    let (d, app) = setup();
    let (st, v) = mcp(
        &app,
        None,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
    assert!(v["result"]["capabilities"]["tools"].is_object());
    // notifications get 202 and no body
    let (st, _) = mcp(
        &app,
        None,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let (_, v) = mcp(
        &app,
        None,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    let tools = v["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "search_packages"));
    let disown = tools.iter().find(|t| t["name"] == "disown").unwrap();
    assert_eq!(disown["annotations"]["destructiveHint"], true);
    assert_eq!(disown["inputSchema"]["required"][0], "pkgbase");

    let (_, v) = mcp(
        &app,
        None,
        tool(3, "search_packages", json!({"query": "hello"})),
    )
    .await;
    assert_eq!(v["result"]["isError"], false);
    assert_eq!(v["result"]["structuredContent"]["resultcount"], 2);
    let (_, v) = mcp(
        &app,
        None,
        tool(4, "package_info", json!({"names": ["hello-taur"]})),
    )
    .await;
    assert_eq!(
        v["result"]["structuredContent"]["results"][0]["Name"],
        "hello-taur"
    );

    // management needs a token
    let (_, v) = mcp(
        &app,
        None,
        tool(5, "vote", json!({"pkgbase": "hello-taur"})),
    )
    .await;
    assert_eq!(v["result"]["isError"], true);
    let t = token(&app, d.path(), "alice").await;
    let (_, v) = mcp(&app, Some(&t), tool(6, "whoami", json!({}))).await;
    assert_eq!(v["result"]["structuredContent"]["name"], "alice");
    let (_, v) = mcp(
        &app,
        Some(&t),
        tool(7, "vote", json!({"pkgbase": "hello-taur"})),
    )
    .await;
    assert_eq!(v["result"]["isError"], false, "{v}");
    assert_eq!(
        get_json(&app, "/rpc/v5/info/hello-taur").await["results"][0]["NumVotes"],
        1
    );
    let (_, v) = mcp(
        &app,
        Some(&t),
        tool(8, "create_org", json!({"org": "weirdbrains"})),
    )
    .await;
    assert_eq!(v["result"]["isError"], false, "{v}");
    let (_, v) = mcp(&app, Some(&t), tool(9, "list_orgs", json!({}))).await;
    assert_eq!(
        v["result"]["structuredContent"]["result"][0]["name"], "weirdbrains",
        "{v}"
    );
    let (_, v) = mcp(
        &app,
        Some(&t),
        tool(10, "add_comment", json!({"pkgbase": "hello-taur"})),
    )
    .await;
    assert_eq!(v["result"]["isError"], true);
    assert!(
        v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("content")
    );

    // protocol errors
    let (_, v) = mcp(
        &app,
        None,
        json!({"jsonrpc": "2.0", "id": 11, "method": "nope"}),
    )
    .await;
    assert_eq!(v["error"]["code"], -32601);
    let (_, v) = mcp(&app, None, tool(12, "nope", json!({}))).await;
    assert_eq!(v["error"]["code"], -32602);
    let (_, v) = mcp(
        &app,
        None,
        json!([{"jsonrpc": "2.0", "id": 13, "method": "ping"}, {"jsonrpc": "2.0", "method": "notifications/x"}]),
    )
    .await;
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(get(&app, "/mcp").await.0, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn http_and_mcp_acceptance_is_what_the_gate_reads() {
    let home = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(home.path()).unwrap());
    let admin = store
        .register("admin", "admin@example.com", "correct horse")
        .unwrap()
        .id;
    let alice = store
        .register("alice", "alice@example.com", "correct horse")
        .unwrap()
        .id;
    let modr = store
        .register("moddy", "mod@example.com", "correct horse")
        .unwrap()
        .id;
    store.set_admin(admin, true).unwrap();
    store.mark_verified(admin).unwrap();
    store.mark_verified(alice).unwrap();
    store.mark_verified(modr).unwrap();
    store.grant_moderator(modr, true).unwrap();
    store.insert_registry("vendor", "enforce").unwrap();
    let (signing, verifying) = taur_core::registry::generate_key();
    let fp = store
        .insert_registry_key("vendor", "lab", verifying.as_bytes())
        .unwrap();
    store
        .insert_identity("vendor", "alice", "alice@example.com")
        .unwrap();
    let src = "pkgbase = demo\npkgver = 1.0.0\npkgrel = 1\n\npkgname = demo\n";
    let pkg = "pkgver=1.0.0\npkgrel=1\npkgname=(demo)\n";
    let sig = taur_core::registry::sign_recipe(
        signing.as_bytes(),
        "vendor",
        "extra",
        "demo",
        "1.0.0-1",
        src.as_bytes(),
        pkg.as_bytes(),
    );
    store
        .insert_expectation("vendor", "demo", "extra", "1.0.0-1", &sig, &fp, "alice")
        .unwrap();
    let bad = home.path().join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join(".SRCINFO"), src).unwrap();
    std::fs::write(bad.join("PKGBUILD"), format!("{pkg}# changed\n")).unwrap();
    std::fs::write(bad.join("TAUR-REGISTRY"), "vendor\n").unwrap();
    std::fs::write(bad.join("TAUR-REPOSITORY"), "extra\n").unwrap();
    std::fs::write(
        bad.join("TAUR-RECIPE.sig"),
        sig.iter().map(|b| format!("{b:02x}")).collect::<String>(),
    )
    .unwrap();
    assert!(
        store
            .import_into(&bad, Some("alice"), Some("vendor"))
            .unwrap_err()
            .to_string()
            .contains("rejected")
    );
    assert!(store.pkgbase_id("demo").unwrap().is_none());
    let app = taur_server::app(store.clone());
    let admin_token = store.login("admin", "correct horse").unwrap().token;
    let mod_token = store.login("moddy", "correct horse").unwrap().token;
    let (st, body) = call(
        &app,
        send("GET", "/api/v1/audit", Some(&admin_token), json!(null)),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let rows: Value = serde_json::from_slice(&body).unwrap();
    let rejection = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["target_kind"] == "registry_upload" && e["open"] == true)
        .unwrap();
    let id = rejection["id"].as_i64().unwrap();
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                &format!("/api/v1/audit/{id}/accept"),
                Some(&mod_token),
                json!({"reason": "no"}),
            ),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (st, body) = call(
        &app,
        send(
            "POST",
            &format!("/api/v1/audit/{id}/accept"),
            Some(&admin_token),
            json!({"reason": "same bytes"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let accepted = serde_json::from_slice::<Value>(&body).unwrap()["event"]
        .as_i64()
        .unwrap();
    store
        .import_into(&bad, Some("alice"), Some("vendor"))
        .unwrap();
    std::fs::write(bad.join("PKGBUILD"), format!("{pkg}# changed\n# more\n")).unwrap();
    assert!(
        store
            .import_into(&bad, Some("alice"), Some("vendor"))
            .unwrap_err()
            .to_string()
            .contains("rejected")
    );

    store
        .insert_identity("vendor", "carol", "carol@example.com")
        .unwrap();
    assert!(
        store
            .register("carol", "other@example.com", "correct horse")
            .unwrap_err()
            .to_string()
            .contains("operator identity")
    );
    let (_, listed) = mcp(
        &app,
        Some(&admin_token),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"accept_rejection"));
    assert!(names.contains(&"revoke_acceptance"));
    let reg = store
        .list_audit(admin)
        .unwrap()
        .into_iter()
        .find(|e| e.target_kind == "registration" && e.open)
        .unwrap();
    let (_, denied) = mcp(
        &app,
        Some(&mod_token),
        tool(2, "accept_rejection", json!({"id": reg.id, "reason": "no"})),
    )
    .await;
    assert_eq!(denied["result"]["isError"], true);
    let (_, ok) = mcp(
        &app,
        Some(&admin_token),
        tool(
            3,
            "accept_rejection",
            json!({"id": reg.id, "reason": "this claim"}),
        ),
    )
    .await;
    assert_eq!(ok["result"]["isError"], false);
    assert!(
        store
            .register("carol", "third@example.com", "correct horse")
            .unwrap_err()
            .to_string()
            .contains("operator identity")
    );
    assert_eq!(
        store
            .register("carol", "other@example.com", "correct horse")
            .unwrap()
            .name,
        "carol"
    );
    let (_, revoked) = mcp(
        &app,
        Some(&admin_token),
        tool(
            4,
            "revoke_acceptance",
            json!({"id": accepted, "reason": "withdraw the upload"}),
        ),
    )
    .await;
    assert_eq!(revoked["result"]["isError"], false);
    std::fs::write(bad.join("PKGBUILD"), format!("{pkg}# changed\n")).unwrap();
    assert!(
        store
            .import_into(&bad, Some("alice"), Some("vendor"))
            .unwrap_err()
            .to_string()
            .contains("rejected")
    );
}

fn recipe_gzip(rel: &str) -> Vec<u8> {
    let pkg = format!("pkgbase=webup\npkgname=(webup)\npkgver=1.0.0\npkgrel={rel}\narch=(any)\n");
    let src = format!(
        "pkgbase = webup\n\tpkgdesc = Uploaded\n\tpkgver = 1.0.0\n\tpkgrel = {rel}\n\tarch = any\n\npkgname = webup\n"
    );
    fn ustar(name: &str, body: &[u8]) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        let put = |h: &mut [u8], at: usize, text: &str| {
            h[at..at + text.len()].copy_from_slice(text.as_bytes());
        };
        put(&mut h, 100, "0000644\0");
        put(&mut h, 108, "0000000\0");
        put(&mut h, 116, "0000000\0");
        put(&mut h, 124, &format!("{:011o}\0", body.len()));
        put(&mut h, 136, "00000000000\0");
        h[148..156].fill(b' ');
        h[156] = b'0';
        h[257..262].copy_from_slice(b"ustar");
        h[263] = b'0';
        h[264] = b'0';
        let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
        put(&mut h, 148, &format!("{sum:06o}\0 "));
        let mut out = h;
        out.extend_from_slice(body);
        while !out.len().is_multiple_of(512) {
            out.push(0);
        }
        out
    }
    let mut raw = Vec::new();
    raw.extend(ustar("PKGBUILD", pkg.as_bytes()));
    raw.extend(ustar(".SRCINFO", src.as_bytes()));
    raw.extend(vec![0u8; 1024]);
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    use std::io::Write;
    enc.write_all(&raw).unwrap();
    enc.finish().unwrap()
}

fn upload_request(token: Option<&str>, bytes: &[u8], csrf: bool) -> Request<Body> {
    let boundary = "taurboundary";
    let mut body = Vec::new();
    body.extend(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"webup.tar.gz\"\r\nContent-Type: application/gzip\r\n\r\n"
        )
        .into_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/packages/upload")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        );
    if let Some(token) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        if csrf {
            req = req.header("x-csrf-token", token);
        }
    }
    req.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn package_upload_creates_updates_and_checks_permission() {
    let (d, app) = setup();
    let gzip = recipe_gzip("1");
    assert_eq!(
        call(&app, upload_request(None, &gzip, false)).await.0,
        StatusCode::UNAUTHORIZED
    );
    let ada = token(&app, d.path(), "ada").await;
    let (st, _) = call(&app, upload_request(Some(&ada), &gzip, false)).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, body) = call(&app, upload_request(Some(&ada), &gzip, true)).await;
    assert_eq!(
        st,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let created: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(created["pkgbase"], "webup");
    assert_eq!(created["created"], true);
    let again = recipe_gzip("2");
    let (st, body) = call(&app, upload_request(Some(&ada), &again, true)).await;
    assert_eq!(st, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["created"],
        false
    );
    assert!(
        get_json(&app, "/rpc/v5/info/webup").await["results"][0]["Version"]
            .as_str()
            .unwrap()
            .ends_with("-2")
    );
    let bob = token(&app, d.path(), "bob").await;
    assert_eq!(
        call(&app, upload_request(Some(&bob), &again, true)).await.0,
        StatusCode::FORBIDDEN
    );
    call(
        &app,
        send(
            "POST",
            "/api/v1/register",
            None,
            json!({"name": "cara", "email": "cara@x", "password": "longenough"}),
        ),
    )
    .await;
    let (_, login) = call(
        &app,
        send(
            "POST",
            "/api/v1/login",
            None,
            json!({"name": "cara", "password": "longenough"}),
        ),
    )
    .await;
    let cara = serde_json::from_slice::<Value>(&login).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, body) = call(&app, upload_request(Some(&cara), &again, true)).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert!(String::from_utf8(body).unwrap().contains("not verified"));
    let bad = recipe_gzip("1");
    let (st, body) = call(&app, upload_request(Some(&ada), b"not-an-archive", true)).await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let _ = bad;
}
