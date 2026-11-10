//! Anonymous HTTP access for the public org directory, package index, and revisions.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use taur_core::Store;
use tower::ServiceExt;

fn write_pkg(dir: &Path, name: &str, ver: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("PKGBUILD"),
        format!("pkgbase={name}\npkgname={name}\npkgver={ver}\npkgrel=1\narch=(any)\n"),
    )
    .unwrap();
    fs::write(
        dir.join(".SRCINFO"),
        format!(
            "pkgbase = {name}\n\tpkgdesc = {name} package\n\tpkgver = {ver}\n\tpkgrel = 1\n\tarch = any\n\npkgname = {name}\n"
        ),
    )
    .unwrap();
}

fn import_pkg(s: &Store, root: &Path, name: &str, ver: &str, maintainer: Option<&str>) {
    let dir = root.join(name);
    write_pkg(&dir, name, ver);
    s.import_dir(&dir, maintainer).unwrap();
}

fn set_modified(s: &Store, name: &str, ts: i64) {
    s.set_last_modified(name, ts).unwrap();
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

async fn get_ok(app: &axum::Router, uri: &str) -> Value {
    let (st, b) = get(app, uri).await;
    assert_eq!(st, StatusCode::OK, "{uri}: {}", String::from_utf8_lossy(&b));
    serde_json::from_slice(&b).unwrap()
}

fn dircopy(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for ent in fs::read_dir(src).unwrap() {
        let ent = ent.unwrap();
        let to = dest.join(ent.file_name());
        if ent.file_type().unwrap().is_dir() {
            dircopy(&ent.path(), &to);
        } else {
            fs::copy(ent.path(), to).unwrap();
        }
    }
}

#[tokio::test]
async fn public_orgs_are_anonymous_and_scoped() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    let staging = d.path().join("src");
    let alice = s.register("alice", "a@x", "correct horse").unwrap().id;
    let org = s.create_org(alice, "WeirdBrains").unwrap();
    let other = s.create_org(alice, "other-org").unwrap();
    import_pkg(&s, &staging, "org-pkg", "1.0.0", Some("alice"));
    import_pkg(&s, &staging, "other-pkg", "1.0.0", Some("alice"));
    import_pkg(&s, &staging, "personal", "1.0.0", Some("alice"));
    s.set_pkgbase_org(alice, s.pkgbase_id("org-pkg").unwrap().unwrap(), Some(org))
        .unwrap();
    s.set_pkgbase_org(
        alice,
        s.pkgbase_id("other-pkg").unwrap().unwrap(),
        Some(other),
    )
    .unwrap();
    let app = taur_server::app(Arc::new(s));

    // Membership-gated list still requires auth.
    let (st, _) = get(&app, "/api/v1/orgs").await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    let v = get_ok(&app, "/api/v1/organizations").await;
    let names: Vec<&str> = v["organizations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"WeirdBrains"), "{names:?}");
    assert!(names.contains(&"other-org"), "{names:?}");

    let filtered = get_ok(&app, "/api/v1/organizations?q=weirdbrains").await;
    assert_eq!(filtered["organizations"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["organizations"][0]["name"], "WeirdBrains");
    let empty = get_ok(&app, "/api/v1/organizations?q=no-such").await;
    assert!(empty["organizations"].as_array().unwrap().is_empty());

    let profile = get_ok(&app, "/api/v1/organizations/weirdbrains").await;
    assert_eq!(profile["name"], "WeirdBrains");
    let pkgs: Vec<&str> = profile["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["Name"].as_str().unwrap())
        .collect();
    assert_eq!(pkgs, vec!["org-pkg"]);
    assert!(!pkgs.contains(&"other-pkg"));
    assert!(!pkgs.contains(&"personal"));

    let (st, _) = get(&app, "/api/v1/organizations/missing-org").await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // SPA routes are public shells.
    for path in ["/orgs", "/orgs/WeirdBrains"] {
        let (st, _) = get(&app, path).await;
        assert_eq!(st, StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn package_index_paginates_and_keeps_sort() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    let staging = d.path().join("src");
    for i in 1..=25 {
        let name = format!("pkg{i:02}");
        import_pkg(&s, &staging, &name, "1.0.0", None);
        set_modified(&s, &name, 1_700_000_000 + i);
    }
    set_modified(&s, "pkg01", 1_800_000_000);
    set_modified(&s, "pkg02", 1_800_000_000);
    let app = taur_server::app(Arc::new(s));

    let p1 = get_ok(&app, "/api/v1/packages/index?sort=updated&page=1").await;
    assert_eq!(p1["per_page"], 20);
    assert_eq!(p1["total"], 25);
    assert_eq!(p1["page"], 1);
    assert_eq!(p1["sort"], "updated");
    let names1: Vec<&str> = p1["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["Name"].as_str().unwrap())
        .collect();
    assert_eq!(names1.len(), 20);
    assert_eq!(names1[0], "pkg01");
    assert_eq!(names1[1], "pkg02");

    let p2 = get_ok(&app, "/api/v1/packages/index?sort=updated&page=2").await;
    assert_eq!(p2["sort"], "updated");
    let names2: Vec<&str> = p2["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["Name"].as_str().unwrap())
        .collect();
    assert_eq!(names2.len(), 5);
    let mut all = names1.clone();
    all.extend(&names2);
    let mut uniq = all.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 25, "skip or duplicate across pages: {all:?}");

    let empty = get_ok(&app, "/api/v1/packages/index?sort=updated&page=99").await;
    assert!(empty["packages"].as_array().unwrap().is_empty());
    assert_eq!(empty["total"], 25);

    let invalid = get_ok(&app, "/api/v1/packages/index?page=nope&sort=updated").await;
    assert_eq!(invalid["page"], 1);
    assert_eq!(invalid["packages"][0]["Name"], "pkg01");

    // No download metric is stored; the API falls back to updated and says so via sort.
    let fallback = get_ok(&app, "/api/v1/packages/index?sort=downloaded&page=2").await;
    assert_eq!(fallback["sort"], "updated");
    assert_eq!(fallback["packages"][0]["Name"], names2[0]);

    let pop = get_ok(&app, "/api/v1/packages/index?sort=popularity&page=1").await;
    assert_eq!(pop["sort"], "popularity");
    assert_eq!(pop["page"], 1);
}

#[tokio::test]
async fn historic_revision_snapshot_is_immutable() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    let fx = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/hello-taur");
    s.import_dir(&fx, None).unwrap();
    let next = d.path().join("v2");
    dircopy(&fx, &next);
    for name in [".SRCINFO", "PKGBUILD"] {
        let path = next.join(name);
        let t = fs::read_to_string(&path).unwrap().replace("1.2.3", "2.0.0");
        fs::write(path, t).unwrap();
    }
    s.import_dir(&next, None).unwrap();
    let app = taur_server::app(Arc::new(s));

    let revs = get_ok(&app, "/api/v1/packages/hello-taur/revisions").await;
    let revs = revs.as_array().unwrap();
    assert!(revs.len() >= 2, "{revs:?}");
    assert_eq!(revs[0]["current"], true);
    assert_eq!(revs[0]["version"], "1:2.0.0-1");
    assert_eq!(revs[1]["current"], false);
    assert_eq!(revs[1]["version"], "1:1.2.3-1");
    let old_id = revs[1]["id"].as_str().unwrap();
    let new_id = revs[0]["id"].as_str().unwrap();
    let old = get_ok(
        &app,
        &format!("/api/v1/packages/hello-taur/revisions/{old_id}"),
    )
    .await;
    assert_eq!(old["version"], "1:1.2.3-1");
    assert_eq!(old["id"], old_id);

    let (st_old, old_snap) = get(
        &app,
        &format!("/cgit/aur.git/snapshot/hello-taur.tar.gz?id={old_id}"),
    )
    .await;
    let (st_new, new_snap) = get(
        &app,
        &format!("/cgit/aur.git/snapshot/hello-taur.tar.gz?id={new_id}"),
    )
    .await;
    assert_eq!(st_old, StatusCode::OK);
    assert_eq!(st_new, StatusCode::OK);
    assert_ne!(old_snap, new_snap);
    assert_eq!(&old_snap[..2], &[0x1f, 0x8b]);

    let (st, _) = get(
        &app,
        "/api/v1/packages/hello-taur/revisions/deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = get(&app, "/api/v1/packages/hello-taur/revisions/not..valid").await;
    assert!(
        st == StatusCode::NOT_FOUND || st == StatusCode::BAD_REQUEST,
        "invalid rev status {st}"
    );

    let (st, body) = get(&app, "/packages/hello-taur/revisions/deadbeef").await;
    assert_eq!(st, StatusCode::OK, "SPA shell for a revision path");
    assert!(!body.is_empty());
}
