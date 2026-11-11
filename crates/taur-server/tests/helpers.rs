//! Real `yay` and `paru` binaries against a temporary TAUR server.
//!
//! This is not an RPC emulation. Each helper is executed with `--aururl` and
//! `--aurrpcurl`, a private `HOME` / XDG tree, and a private pacman root.
//! `makepkg` and `sudo` are scripts that fail if a helper tries to build or
//! install. Set `TAUR_YAY` or `TAUR_PARU` to point at a binary. Set
//! `TAUR_REQUIRE_HELPERS=yay,paru` to fail when a named helper is absent.

use http_body_util::BodyExt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use taur_core::Store;

#[derive(Clone, Debug)]
struct Hit {
    method: String,
    uri: String,
    ua: String,
    body: String,
}

struct Srv {
    base: String,
    hits: Arc<Mutex<Vec<Hit>>>,
    // Runtime first so the listener releases the data dir before TempDir deletes it.
    _rt: tokio::runtime::Runtime,
    _home: tempfile::TempDir,
}

struct Iso {
    _root: tempfile::TempDir,
    base: String,
    pacman_conf: PathBuf,
    build: PathBuf,
    work: PathBuf,
    sudo: PathBuf,
    makepkg: PathBuf,
    sudo_log: PathBuf,
    makepkg_log: PathBuf,
    home: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    data: PathBuf,
    state: PathBuf,
    runtime: PathBuf,
}

#[test]
fn real_helpers_search_info_and_clone() {
    let before = user_state();
    let srv = start();
    clone_fixture(&srv, &format!("{}/hello-taur", srv.base));
    clone_fixture(&srv, &format!("{}/hello-taur.git", srv.base));

    for name in ["yay", "paru"] {
        match helper_bin(name) {
            Some(bin) => exercise(&srv, name, &bin),
            None if required(name) => panic!(
                "{name} is required (TAUR_REQUIRE_HELPERS) but was not found; set TAUR_{} or PATH",
                name.to_ascii_uppercase()
            ),
            None => eprintln!(
                "skipping {name}: not on PATH and TAUR_{} is unset",
                name.to_ascii_uppercase()
            ),
        }
    }
    assert_eq!(
        user_state(),
        before,
        "helper run changed a real config path or the system pacman lock"
    );
}

fn start() -> Srv {
    let home = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(Store::open(home.path()).unwrap());
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur");
    store.import_dir(&fx, None).unwrap();
    let hits = Arc::new(Mutex::new(Vec::new()));
    let logged = hits.clone();
    let app = taur_server::app(store).layer(axum::middleware::from_fn(
        move |req: axum::extract::Request, next: axum::middleware::Next| {
            let logged = logged.clone();
            async move {
                let method = req.method().as_str().to_string();
                let uri = req.uri().to_string();
                let ua = req
                    .headers()
                    .get(axum::http::header::USER_AGENT)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let (parts, body) = req.into_parts();
                let bytes = body.collect().await.expect("request body").to_bytes();
                let n = bytes.len().min(4096);
                let snippet = String::from_utf8_lossy(&bytes[..n]).into_owned();
                logged.lock().unwrap().push(Hit {
                    method,
                    uri,
                    ua,
                    body: snippet,
                });
                next.run(axum::extract::Request::from_parts(
                    parts,
                    axum::body::Body::from(bytes),
                ))
                .await
            }
        },
    ));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener =
        rt.block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap() });
    let base = format!("http://{}", listener.local_addr().unwrap());
    rt.spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Srv {
        base,
        hits,
        _rt: rt,
        _home: home,
    }
}

fn clone_fixture(srv: &Srv, url: &str) {
    srv.hits.lock().unwrap().clear();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("pkg");
    let mut cmd = Command::new("git");
    cmd.args(["clone", "--no-progress", url]);
    cmd.arg(&dest);
    cmd.env_clear();
    cmd.env("PATH", std::env::var_os("PATH").expect("PATH"));
    cmd.env("HOME", dir.path());
    git_env(&mut cmd);
    let (ok, text) = finish(&mut cmd);
    assert!(ok, "git clone {url} failed:\n{text}");
    assert_fixture(&dest);
    let hits = srv.hits.lock().unwrap();
    assert!(
        hits.iter().any(|h| {
            h.uri.contains("hello-taur")
                && h.uri.contains("info/refs")
                && h.uri.contains("service=git-upload-pack")
        }),
        "clone did not use smart HTTP: {hits:?}"
    );
}

fn exercise(srv: &Srv, name: &str, bin: &Path) {
    let iso = isolate(&srv.base);
    let ver = helper_version(bin, &iso);
    println!("{name} version: {ver}");
    assert!(
        ver.chars().any(|c| c.is_ascii_digit()),
        "{name} version looks empty: {ver}"
    );

    srv.hits.lock().unwrap().clear();
    let (ok, text) = helper_cmd(bin, &iso, name, &["-a", "--color", "never", "-Ss", "hello"]);
    assert!(ok, "{name} search failed:\n{text}");
    assert!(
        text.replace("hello-taur-docs", "").contains("hello-taur"),
        "{name} search:\n{text}"
    );
    assert!(text.contains("hello-taur-docs"), "{name} search:\n{text}");
    assert!(text.contains("1:1.2.3-1"), "{name} search:\n{text}");
    assert!(
        text.contains("A tiny test package for TAUR"),
        "{name} search:\n{text}"
    );
    assert_rpc(name, &srv.hits.lock().unwrap(), "search", "arg", "hello");
    assert!(
        srv.hits.lock().unwrap().iter().any(|h| ua_ok(name, &h.ua)),
        "{name} search user-agent: {:?}",
        srv.hits.lock().unwrap()
    );

    for (pkg, desc) in [
        ("hello-taur", "A tiny test package for TAUR"),
        ("hello-taur-docs", "Docs for hello-taur"),
    ] {
        srv.hits.lock().unwrap().clear();
        let (ok, text) = helper_cmd(bin, &iso, name, &["-a", "--color", "never", "-Sii", pkg]);
        assert!(ok, "{name} info {pkg} failed:\n{text}");
        assert!(text.contains(pkg), "{name} info:\n{text}");
        assert!(text.contains("1:1.2.3-1"), "{name} info:\n{text}");
        assert!(text.contains(desc), "{name} info:\n{text}");
        assert!(
            text.contains("https://example.com/hello-taur"),
            "{name} info:\n{text}"
        );
        assert_rpc(name, &srv.hits.lock().unwrap(), "info", "arg[]", pkg);
    }

    srv.hits.lock().unwrap().clear();
    let (ok, text) = helper_cmd(
        bin,
        &iso,
        name,
        &["-a", "--color", "never", "-G", "hello-taur"],
    );
    assert!(ok, "{name} clone failed:\n{text}");
    assert_fixture(&iso.work.join("hello-taur"));
    let hits = srv.hits.lock().unwrap();
    assert!(
        hits.iter().any(|h| {
            h.uri.contains("hello-taur")
                && h.uri.contains("info/refs")
                && h.uri.contains("service=git-upload-pack")
        }),
        "{name} clone requests: {hits:?}\n{text}"
    );
    drop(hits);
    assert!(
        !iso.sudo_log.exists(),
        "{name} invoked sudo: {}",
        fs::read_to_string(&iso.sudo_log).unwrap_or_default()
    );
    assert!(
        !iso.makepkg_log.exists(),
        "{name} invoked makepkg: {}",
        fs::read_to_string(&iso.makepkg_log).unwrap_or_default()
    );
}

fn assert_fixture(dir: &Path) {
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur");
    assert_eq!(
        fs::read(dir.join("PKGBUILD")).expect("PKGBUILD"),
        fs::read(fx.join("PKGBUILD")).unwrap()
    );
    assert_eq!(
        fs::read(dir.join(".SRCINFO")).expect(".SRCINFO"),
        fs::read(fx.join(".SRCINFO")).unwrap()
    );
}

fn assert_rpc(name: &str, hits: &[Hit], kind: &str, arg_key: &str, arg_val: &str) {
    let method = if name == "paru" { "POST" } else { "GET" };
    let ok = hits.iter().any(|h| {
        if h.method != method {
            return false;
        }
        let blob = format!("{}&{}", query_of(&h.uri), h.body);
        fields(&blob, "type").iter().any(|t| t == kind)
            && fields(&blob, arg_key).iter().any(|a| a == arg_val)
    });
    assert!(
        ok,
        "{name} missing {method} type={kind} {arg_key}={arg_val}: {hits:?}"
    );
}

fn query_of(uri: &str) -> &str {
    uri.split_once('?').map(|(_, q)| q).unwrap_or("")
}

fn fields(blob: &str, key: &str) -> Vec<String> {
    blob.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (decode(k) == key).then(|| decode(v))
        })
        .collect()
}

fn decode(s: &str) -> String {
    let s = s.replace('+', " ");
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn ua_ok(name: &str, ua: &str) -> bool {
    match name {
        "yay" => ua.starts_with("Yay/"),
        "paru" => ua.to_ascii_lowercase().starts_with("paru/"),
        _ => false,
    }
}

fn helper_cmd(bin: &Path, iso: &Iso, name: &str, args: &[&str]) -> (bool, String) {
    let mut cmd = Command::new(bin);
    isolate_env(&mut cmd, iso);
    cmd.current_dir(&iso.work);
    let clone_flag = if name == "paru" {
        "--clonedir"
    } else {
        "--builddir"
    };
    // yay's official default RPC URL ends in `?` (GET query concat).
    // paru POSTs the form to the URL as-is and defaults to `{aururl}/rpc` with no `?`.
    let rpc = if name == "paru" {
        format!("{}/rpc", iso.base)
    } else {
        format!("{}/rpc?", iso.base)
    };
    cmd.args([
        "--noconfirm",
        "--aururl",
        iso.base.as_str(),
        "--aurrpcurl",
        rpc.as_str(),
        clone_flag,
        iso.build.to_str().unwrap(),
        "--config",
        iso.pacman_conf.to_str().unwrap(),
        "--sudo",
        iso.sudo.to_str().unwrap(),
        "--makepkg",
        iso.makepkg.to_str().unwrap(),
    ]);
    cmd.args(args);
    finish(&mut cmd)
}

fn helper_version(bin: &Path, iso: &Iso) -> String {
    let mut cmd = Command::new(bin);
    isolate_env(&mut cmd, iso);
    cmd.arg("--version");
    let (ok, text) = finish(&mut cmd);
    assert!(ok, "version command failed for {}:\n{text}", bin.display());
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn isolate(base: &str) -> Iso {
    let root = tempfile::tempdir().unwrap();
    let p = |rel: &str| root.path().join(rel);
    for rel in [
        "home", "config", "cache", "data", "state", "runtime", "build", "work", "db/local",
        "db/sync", "pkgroot", "pkgcache", "gpg", "hooks",
    ] {
        fs::create_dir_all(p(rel)).unwrap();
    }
    let sudo_log = p("sudo.log");
    let makepkg_log = p("makepkg.log");
    let sudo = p("sudo");
    let makepkg = p("makepkg");
    write_script(
        &sudo,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexit 99\n",
            sh(&sudo_log)
        ),
    );
    write_script(
        &makepkg,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexit 99\n",
            sh(&makepkg_log)
        ),
    );
    let pacman_conf = p("pacman.conf");
    let db = p("db");
    let pkgroot = p("pkgroot");
    let pkgcache = p("pkgcache");
    let gpg = p("gpg");
    let hooks = p("hooks");
    let log = p("pacman.log");
    fs::write(
        &pacman_conf,
        format!(
            "[options]\nHoldPkg = pacman\nArchitecture = x86_64\nSigLevel = Never\nLocalFileSigLevel = Never\nRootDir = {}\nDBPath = {}\nCacheDir = {}\nLogFile = {}\nGPGDir = {}\nHookDir = {}\n",
            pkgroot.display(),
            db.display(),
            pkgcache.display(),
            log.display(),
            gpg.display(),
            hooks.display()
        ),
    )
    .unwrap();
    Iso {
        base: base.to_string(),
        pacman_conf,
        build: p("build"),
        work: p("work"),
        sudo,
        makepkg,
        sudo_log,
        makepkg_log,
        home: p("home"),
        config: p("config"),
        cache: p("cache"),
        data: p("data"),
        state: p("state"),
        runtime: p("runtime"),
        _root: root,
    }
}

fn sh(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn write_script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, body).unwrap();
    let mut perm = fs::metadata(path).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(path, perm).unwrap();
}

fn isolate_env(cmd: &mut Command, iso: &Iso) {
    let path = std::env::var_os("PATH").expect("PATH");
    cmd.env_clear();
    cmd.env("PATH", path);
    cmd.env("HOME", &iso.home);
    cmd.env("XDG_CONFIG_HOME", &iso.config);
    cmd.env("XDG_CACHE_HOME", &iso.cache);
    cmd.env("XDG_DATA_HOME", &iso.data);
    cmd.env("XDG_STATE_HOME", &iso.state);
    cmd.env("XDG_RUNTIME_DIR", &iso.runtime);
    cmd.env("LANG", "C");
    cmd.env("LC_ALL", "C");
    cmd.env("PAGER", "cat");
    git_env(cmd);
}

fn git_env(cmd: &mut Command) {
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_SYSTEM", "/dev/null");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_PAGER", "cat");
}

fn finish(cmd: &mut Command) -> (bool, String) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn");
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut v).unwrap();
        v
    });
    let err_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        std::io::Read::read_to_end(&mut stderr, &mut v).unwrap();
        v
    });
    let limit = Duration::from_secs(90);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > limit {
            let id = child.id();
            let _ = Command::new("kill")
                .args(["-s", "KILL", &format!("-{id}")])
                .status();
            let _ = child.kill();
            let _ = child.wait();
            let stdout = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned();
            let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();
            panic!("helper timed out after {limit:?}\n{stdout}{stderr}");
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let stdout = String::from_utf8_lossy(&out_t.join().unwrap()).into_owned();
    let stderr = String::from_utf8_lossy(&err_t.join().unwrap()).into_owned();
    (status.success(), format!("{stdout}{stderr}"))
}

fn helper_bin(name: &str) -> Option<PathBuf> {
    let key = format!("TAUR_{}", name.to_ascii_uppercase());
    if let Some(v) = std::env::var_os(&key) {
        let p = PathBuf::from(&v);
        assert!(p.is_file(), "{key} is not a file: {}", p.display());
        return Some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

fn required(name: &str) -> bool {
    std::env::var("TAUR_REQUIRE_HELPERS")
        .unwrap_or_default()
        .split(',')
        .any(|s| s.trim() == name)
}

fn user_state() -> Vec<(String, String)> {
    let mut paths = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for rel in [
            ".config/yay",
            ".config/yay/config.json",
            ".config/paru",
            ".config/paru/paru.conf",
            ".gitconfig",
            ".config/git/config",
        ] {
            paths.push(home.join(rel));
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        let xdg = PathBuf::from(xdg);
        paths.push(xdg.join("yay"));
        paths.push(xdg.join("yay/config.json"));
        paths.push(xdg.join("paru"));
        paths.push(xdg.join("paru/paru.conf"));
    }
    paths.push(PathBuf::from("/var/lib/pacman/db.lck"));
    paths
        .into_iter()
        .map(|p| {
            let desc = match fs::symlink_metadata(&p) {
                Ok(m) => format!(
                    "dir={} len={} mtime={:?}",
                    m.is_dir(),
                    m.len(),
                    m.modified().unwrap_or(SystemTime::UNIX_EPOCH)
                ),
                Err(_) => "absent".into(),
            };
            (p.display().to_string(), desc)
        })
        .collect()
}
