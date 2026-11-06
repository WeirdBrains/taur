//! Renders web UI pages in headless Chromium against a temp server.
//! Ignored by default (needs local `chromium` and `python3`):
//! `cargo test -p taur-server --test web_headless -- --ignored`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use taur_core::Store;

fn dump(url: &str, profile: &std::path::Path, marker: &str) -> String {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/chromium_dump.py");
    let dir = profile.join(format!(
        "p{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let o = Command::new("python3")
        .arg(script)
        .arg(url)
        .arg(&dir)
        .arg(marker)
        .output()
        .expect("python3");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(o.status.success(), "{text}");
    text
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
    let page = dump(
        &format!("{url}/packages/hello-taur"),
        profile.path(),
        "hello-taur 1:1.2.3-1",
    );
    assert!(page.contains("hello-taur 1:1.2.3-1"), "{page}");
    assert!(page.contains("Make depends"), "{page}");
    assert!(
        page.contains("rust&gt;=1.80") || page.contains("rust>=1.80"),
        "{page}"
    );
    let search = dump(
        &format!("{url}/?q=hello&by=name"),
        profile.path(),
        "2 packages found",
    );
    assert!(search.contains("2 packages found"), "{search}");
    let verify = dump(
        &format!("{url}/verify?token={token}"),
        profile.path(),
        "Email verified",
    );
    assert!(verify.contains("Email verified"), "{verify}");
    assert!(store.user_by_name("alice").unwrap().unwrap().verified);
    let reset = dump(&format!("{url}/reset"), profile.path(), "Reset password");
    assert!(reset.contains("Reset password"), "{reset}");
}
