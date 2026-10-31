//! Renders web UI pages in headless Chromium against a temp server.
//! Ignored by default (needs a local `chromium`): `cargo test -p taur-server -- --ignored`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use taur_core::Store;

fn dump(url: &str, profile: &std::path::Path) -> String {
    let o = Command::new("chromium")
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            "--virtual-time-budget=5000",
            "--dump-dom",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(url)
        .output()
        .expect("chromium");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
#[ignore]
fn pages_render_in_chromium() {
    let home = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(home.path()).unwrap());
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur");
    store.import_dir(&fx, None).unwrap();
    store.register("alice", "a@x", "longenough").unwrap();
    let token =
        taur_core::mail::FileMailer::find_token(&home.path().join("outbox"), "a@x", "verify")
            .unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let l = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let app = taur_server::app(store.clone());
    rt.spawn(async move { axum::serve(l, app).await.unwrap() });
    let profile = tempfile::tempdir().unwrap();
    let page = dump(&format!("{url}/packages/hello-taur"), profile.path());
    assert!(page.contains("<h1>hello-taur 1:1.2.3-1</h1>"), "{page}");
    assert!(page.contains("<dt>Make depends</dt><dd>rust&gt;=1.80"));
    let search = dump(&format!("{url}/?q=hello&by=name"), profile.path());
    assert!(search.contains("2 packages found"));
    let verify = dump(&format!("{url}/verify?token={token}"), profile.path());
    assert!(verify.contains("Email verified"), "{verify}");
    assert!(store.user_by_name("alice").unwrap().unwrap().verified);
    assert!(dump(&format!("{url}/reset"), profile.path()).contains("Reset password"));
}
