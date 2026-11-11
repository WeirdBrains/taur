//! Renders web UI pages in headless Chromium against a temp server.
//! Ignored by default (needs local `chromium` and `python3`):
//! `cargo test -p taur-server --test web_headless -- --ignored`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use taur_core::Store;

fn webup_gzip() -> Vec<u8> {
    let pkg = b"pkgbase=webup\npkgname=(webup)\npkgver=1.0.0\npkgrel=1\narch=(any)\n";
    let src = b"pkgbase = webup\n\tpkgdesc = Uploaded\n\tpkgver = 1.0.0\n\tpkgrel = 1\n\tarch = any\n\npkgname = webup\n";
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
    raw.extend(ustar("PKGBUILD", pkg));
    raw.extend(ustar(".SRCINFO", src));
    raw.extend(vec![0u8; 1024]);
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    use std::io::Write;
    enc.write_all(&raw).unwrap();
    enc.finish().unwrap()
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let mut n = (chunk[0] as u32) << 16;
        if chunk.len() > 1 {
            n |= (chunk[1] as u32) << 8;
        }
        if chunk.len() > 2 {
            n |= chunk[2] as u32;
        }
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn dump(url: &str, profile: &std::path::Path, marker: &str) -> String {
    dump_hook(url, profile, marker, None)
}

fn dump_shot(url: &str, profile: &std::path::Path, marker: &str, shot: &std::path::Path) -> String {
    // One-shot capture for the home header. The shared hook path strips the env.
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
        .env("TAUR_WEB_SCREENSHOT", shot)
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

fn dump_hook(url: &str, profile: &std::path::Path, marker: &str, hook: Option<&str>) -> String {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/chromium_dump.py");
    let dir = profile.join(format!(
        "p{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cmd = Command::new("python3");
    cmd.arg(script).arg(url).arg(&dir).arg(marker);
    if let Some(hook) = hook {
        cmd.arg(hook);
    }
    // Do not inherit a process-wide screenshot path; later pages would overwrite it.
    cmd.env_remove("TAUR_WEB_SCREENSHOT");
    let o = cmd.output().expect("python3");
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
    let home = if let Some(shot) = std::env::var_os("TAUR_WEB_SCREENSHOT") {
        dump_shot(
            &format!("{url}/"),
            profile.path(),
            "Packages",
            std::path::Path::new(&shot),
        )
    } else {
        dump(&format!("{url}/"), profile.path(), "Packages")
    };
    assert!(
        home.contains("/static/logo-dark.svg"),
        "home header should include the TAUR mark: {home}"
    );
    assert!(
        home.contains("TAUR mark: an angular tau, not the Arch Linux logo"),
        "{home}"
    );
    assert!(
        home.contains(r#"href="https://github.com/WeirdBrains/taur""#),
        "menubar should link to the project GitHub: {home}"
    );
    assert!(
        home.contains(">GitHub</a>"),
        "GitHub should be a visible nav item: {home}"
    );
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
    let docs = dump(&format!("{url}/docs"), profile.path(), "Publish a package");
    assert!(docs.contains("Publish a package"), "{docs}");
    assert!(!docs.contains("Operating the server"), "{docs}");
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
    store
        .create_org(store.user_by_name("alice").unwrap().unwrap().id, "weborg")
        .unwrap();
    let reset = dump(&format!("{url}/reset"), profile.path(), "Reset password");
    assert!(reset.contains("Reset password"), "{reset}");

    store
        .set_admin(store.user_by_name("alice").unwrap().unwrap().id, true)
        .unwrap();
    let session = store.login("alice", "longenough").unwrap().token;
    let filter_hook = format!(
        "(() => {{ localStorage.setItem('taur.token', {session:?}); if (!window.__taurBoot) {{ window.__taurBoot = 1; loadMe().then(route); return; }} if (window.__taurFilter) return; const toggle = document.getElementById('upload-owner-toggle'); const hidden = document.querySelector('input[name=owner]'); const filter = document.querySelector('input.combo-filter'); if (!toggle || !hidden || !filter) return; if (hidden.value !== '') return; toggle.click(); const menu = document.querySelector('.combo-menu'); if (!menu || menu.hidden || menu.firstElementChild !== filter) return; filter.value = 'zzz-no-such'; filter.dispatchEvent(new Event('input', {{ bubbles: true }})); if (![...document.querySelectorAll('.combo-empty')].some((el) => el.textContent === 'No matches')) return; document.body.append('no-owner-matches'); filter.value = 'web'; filter.dispatchEvent(new Event('input', {{ bubbles: true }})); const opt = [...document.querySelectorAll('.combo-option')].find((o) => o.textContent === 'weborg'); if (!opt || document.querySelectorAll('.combo-option').length !== 1) return; opt.click(); if (hidden.value !== 'weborg') return; window.__taurFilter = 1; document.body.append('owner-filter-ok'); }})()"
    );
    let filtered = dump_hook(
        &format!("{url}/upload"),
        profile.path(),
        "owner-filter-ok",
        Some(&filter_hook),
    );
    assert!(filtered.contains("owner-filter-ok"), "{filtered}");
    assert!(filtered.contains("no-owner-matches"), "{filtered}");
    assert!(filtered.contains("weborg"), "{filtered}");
    let upload_hook = format!(
        "(() => {{ localStorage.setItem('taur.token', {session:?}); if (!window.__taurBoot) {{ window.__taurBoot = 1; loadMe().then(route); return; }} const input = document.querySelector('input[name=archive]'); const hidden = document.querySelector('input[name=owner]'); const toggle = document.getElementById('upload-owner-toggle'); if (input && hidden && toggle && !window.__taurSent) {{ toggle.click(); const opt = [...document.querySelectorAll('.combo-option')].find((o) => o.textContent === 'weborg'); if (!opt) return; window.__taurSent = 1; opt.click(); if (hidden.value !== 'weborg') {{ window.__taurSent = 0; return; }} const bin = Uint8Array.from(atob({b64:?}), c => c.charCodeAt(0)); const file = new File([bin], 'webup.tar.gz', {{type:'application/gzip'}}); const dt = new DataTransfer(); dt.items.add(file); input.files = dt.files; input.form.requestSubmit(); }} }})()",
        b64 = b64(&webup_gzip())
    );
    let uploaded = dump_hook(
        &format!("{url}/upload"),
        profile.path(),
        "webup 1.0.0-1",
        Some(&upload_hook),
    );
    assert!(uploaded.contains("webup 1.0.0-1"), "{uploaded}");
    assert!(uploaded.contains("Published webup"), "{uploaded}");
    assert!(uploaded.contains("weborg"), "{uploaded}");
    let cycle = format!(
        "(() => {{ localStorage.setItem('taur.token', {session:?}); window.prompt = () => 'upstream 2.0'; window.confirm = () => true; if (!window.__taurBoot) {{ window.__taurBoot = 1; loadMe().then(route); return; }} const click = (label) => [...document.querySelectorAll('button')].find((b) => b.textContent === label); if (!window.__step) {{ const b = click('Mark out of date'); if (b) {{ window.__step = 1; b.click(); }} return; }} if (window.__step === 1) {{ const b = click('Clear out-of-date flag'); if (b) {{ window.__step = 2; b.click(); }} return; }} if (window.__step === 2) {{ const b = click('Archive'); if (b) {{ window.__step = 3; b.click(); }} return; }} if (window.__step === 3 && document.body.innerText.includes('Archived')) {{ const b = click('Unarchive'); if (b) {{ window.__step = 4; b.click(); }} }} }})()"
    );
    let restored = dump_hook(
        &format!("{url}/packages/webup"),
        profile.path(),
        "Package restored.",
        Some(&cycle),
    );
    assert!(restored.contains("Package restored."), "{restored}");
    assert!(!restored.contains("Archived since"), "{restored}");
    store.insert_registry("vendor", "enforce").unwrap();
    store
        .insert_identity("vendor", "carol", "carol@example.com")
        .unwrap();
    assert!(
        store
            .register("carol", "other@example.com", "longenough")
            .unwrap_err()
            .to_string()
            .contains("operator identity")
    );
    let hook = format!(
        "(() => {{ localStorage.setItem('taur.token', {session:?}); if (typeof loadMe !== 'function') return; if (!window.__taurBoot) {{ window.__taurBoot = 1; loadMe().then(route); return; }} window.prompt = () => 'allow this claim'; const b = [...document.querySelectorAll('button')].find((x) => x.textContent === 'Accept'); if (b && !window.__taurClicked) {{ window.__taurClicked = 1; b.click(); }} }})()"
    );
    let audit = dump_hook(
        &format!("{url}/audit"),
        profile.path(),
        "Accepted.",
        Some(&hook),
    );
    assert!(audit.contains("Accepted."), "{audit}");
    assert!(audit.contains("registration"), "{audit}");
    assert!(!audit.contains("other@example.com"), "{audit}");
    assert!(!audit.contains("longenough"), "{audit}");
    store
        .register("carol", "other@example.com", "longenough")
        .unwrap();
    assert!(store.user_by_name("carol").unwrap().is_some());

    let walk = format!(
        "(() => {{ localStorage.setItem('taur.token', {session:?}); if (!window.__taurBoot) {{ window.__taurBoot = 1; loadMe().then(route); }} }})()"
    );
    for (path, marker) in [
        ("/admin", "Users"),
        ("/registries", "Registries"),
        ("/mirrors", "Mirrors"),
        ("/account", "Sessions"),
    ] {
        let page = dump_hook(&format!("{url}{path}"), profile.path(), marker, Some(&walk));
        assert!(page.contains(marker), "{path}: {page}");
    }
}
