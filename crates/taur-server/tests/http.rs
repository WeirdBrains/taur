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
    assert!(String::from_utf8(b).unwrap().contains("refs/heads/master"));
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

async fn token(app: &axum::Router, name: &str) -> String {
    let reg = json!({"name": name, "email": format!("{name}@x"), "password": "longenough"});
    call(app, send("POST", "/api/v1/register", None, reg)).await;
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
    let (_d, app) = setup();
    let a = token(&app, "alice").await;
    let b = token(&app, "bob").await;
    let c = token(&app, "carol").await;
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
    // orphaned fixture: alice adopts, moves to org, carol can manage
    assert_eq!(
        call(
            &app,
            send(
                "POST",
                "/api/v1/packages/hello-taur/adopt",
                Some(&a),
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
