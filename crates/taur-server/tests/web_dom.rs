//! Runs the package-page render helpers against a real JS DOM (Node).
//! Complements the optional Chromium fixture in `web_headless.rs`.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn package_page_omits_literal_null_when_optional_banner_is_absent() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/web_dom.js");
    let out = Command::new("node")
        .arg(&script)
        .output()
        .expect("node is required to run web_dom.js");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "web_dom.js failed:\n{stdout}\n{stderr}"
    );
    assert!(stdout.contains("ok"), "{stdout}{stderr}");
}
