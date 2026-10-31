//! End-to-end pushes with real git over smart HTTP and SSH against a temp server.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use taur_core::Store;
use taur_server::{AppState, app_with};

struct Srv {
    _home: tempfile::TempDir,
    work: tempfile::TempDir,
    store: Arc<Store>,
    http: String,
    ssh_port: u16,
    _rt: tokio::runtime::Runtime,
}

fn start() -> Srv {
    let home = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(home.path()).unwrap());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let hook_bin = PathBuf::from(env!("CARGO_BIN_EXE_taur"));
    let (http, ssh_port) = rt.block_on(async {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let s = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        (l, s)
    });
    let url = format!("http://{}", http.local_addr().unwrap());
    let port = ssh_port.local_addr().unwrap().port();
    let app = app_with(AppState {
        store: store.clone(),
        hook_bin: hook_bin.clone(),
    });
    rt.spawn(async move { axum::serve(http, app).await.unwrap() });
    rt.spawn(taur_server::ssh::serve(ssh_port, store.clone(), hook_bin));
    Srv {
        _home: home,
        work: tempfile::tempdir().unwrap(),
        store,
        http: url,
        ssh_port: port,
        _rt: rt,
    }
}

fn git(dir: &Path, args: &[&str], envs: &[(&str, String)]) -> (bool, String) {
    let o = Command::new("git")
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@x",
            "-c",
            "init.defaultBranch=master",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(envs.iter().map(|(k, v)| (*k, v.as_str())))
        .output()
        .unwrap();
    (
        o.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
    )
}

fn write_pkg(dir: &Path, pkgbase: &str, ver: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("PKGBUILD"),
        format!("pkgname={pkgbase}\npkgver={ver}\npkgrel=1\narch=(any)\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join(".SRCINFO"),
        format!("pkgbase = {pkgbase}\n\tpkgdesc = pushed package\n\tpkgver = {ver}\n\tpkgrel = 1\n\tarch = any\n\npkgname = {pkgbase}\n"),
    )
    .unwrap();
}

fn user(s: &Srv, name: &str) -> String {
    let email = format!("{name}@x");
    s.store.register(name, &email, "longenough").unwrap();
    let outbox = s.store.home().join("outbox");
    let t = taur_core::mail::FileMailer::find_token(&outbox, &email, "verify").unwrap();
    s.store.verify_email(&t).unwrap();
    s.store.login(name, "longenough").unwrap().token
}

#[test]
fn push_over_http_and_ssh() {
    let s = start();
    let alice = user(&s, "alice");
    let bob = user(&s, "bob");
    let host = s.http.trim_start_matches("http://");
    let w = s.work.path().join("push-me");
    write_pkg(&w, "push-me", "1.0");
    assert!(git(&w, &["init", "-q"], &[]).0);
    git(&w, &["add", "-A"], &[]);
    git(&w, &["commit", "-qm", "init"], &[]);

    // unverified accounts can't push
    s.store.register("eve", "eve@x", "longenough").unwrap();
    let eve = s.store.login("eve", "longenough").unwrap().token;
    let (ok, out) = git(
        &w,
        &[
            "push",
            &format!(
                "http://eve:{eve}@{}/push-me.git",
                s.http.trim_start_matches("http://")
            ),
            "master",
        ],
        &[],
    );
    assert!(!ok && out.contains("not verified"), "{out}");
    // anonymous push is refused
    let (ok, _) = git(
        &w,
        &["push", &format!("{}/push-me.git", s.http), "master"],
        &[],
    );
    assert!(!ok);
    // authenticated push creates the package base
    let alice_url = format!("http://alice:{alice}@{host}/push-me.git");
    let (ok, out) = git(&w, &["push", &alice_url, "master"], &[]);
    assert!(ok, "{out}");
    let p = &s.store.info(&["push-me".into()]).unwrap()[0];
    assert_eq!(
        (p.version.as_str(), p.maintainer.as_deref()),
        ("1.0-1", Some("alice"))
    );

    // smart-HTTP clone
    let c = s.work.path().join("clone");
    let (ok, out) = git(
        s.work.path(),
        &["clone", "-q", &format!("{}/push-me.git", s.http), "clone"],
        &[],
    );
    assert!(ok, "{out}");
    assert!(c.join("PKGBUILD").exists());

    // bob may not push to alice's package
    write_pkg(&w, "push-me", "1.1");
    git(&w, &["commit", "-qam", "bump"], &[]);
    let (ok, out) = git(
        &w,
        &[
            "push",
            &format!("http://bob:{bob}@{host}/push-me.git"),
            "master",
        ],
        &[],
    );
    assert!(!ok && out.contains("permission denied"), "{out}");

    // validation: missing .SRCINFO, wrong pkgbase, subdirectory, wrong branch
    let bad = s.work.path().join("bad");
    write_pkg(&bad, "other-name", "1");
    git(&bad, &["init", "-q"], &[]);
    git(&bad, &["add", "-A"], &[]);
    git(&bad, &["commit", "-qm", "x"], &[]);
    let (ok, out) = git(
        &bad,
        &[
            "push",
            &format!("http://alice:{alice}@{host}/bad.git"),
            "master",
        ],
        &[],
    );
    assert!(!ok && out.contains("does not match"), "{out}");
    std::fs::remove_file(bad.join(".SRCINFO")).unwrap();
    git(&bad, &["commit", "-qam", "rm"], &[]);
    let (ok, out) = git(
        &bad,
        &[
            "push",
            &format!("http://alice:{alice}@{host}/other-name.git"),
            "master",
        ],
        &[],
    );
    assert!(!ok && out.contains("missing .SRCINFO"), "{out}");
    let (ok, out) = git(&w, &["push", &alice_url, "master:refs/heads/dev"], &[]);
    assert!(!ok && out.contains("other than master"), "{out}");

    // SSH: register a key, push the bump
    let key = s.work.path().join("id_ed25519");
    let st = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
        .arg(&key)
        .status()
        .unwrap();
    assert!(st.success());
    let pubkey = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let uid = s.store.user_by_name("alice").unwrap().unwrap().id;
    s.store.add_ssh_key(uid, &pubkey).unwrap();
    let ssh_cmd = format!(
        "ssh -F /dev/null -i {} -p {} -o IdentitiesOnly=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR",
        key.display(),
        s.ssh_port
    );
    let env = [("GIT_SSH_COMMAND", ssh_cmd)];
    let (ok, out) = git(
        &w,
        &["push", "ssh://taur@127.0.0.1/push-me.git", "master"],
        &env,
    );
    assert!(ok, "{out}");
    assert_eq!(
        s.store.info(&["push-me".into()]).unwrap()[0].version,
        "1.1-1"
    );
    // non-fast-forward is denied
    git(&w, &["commit", "-q", "--amend", "-m", "rewrite"], &[]);
    let (ok, _) = git(
        &w,
        &["push", "-f", "ssh://taur@127.0.0.1/push-me.git", "master"],
        &env,
    );
    assert!(!ok);
    // SSH clone
    let (ok, out) = git(
        s.work.path(),
        &[
            "clone",
            "-q",
            "ssh://taur@127.0.0.1/push-me.git",
            "sshclone",
        ],
        &env,
    );
    assert!(ok, "{out}");
    // unknown key is rejected
    s.store
        .remove_ssh_key(uid, s.store.list_ssh_keys(uid).unwrap()[0].id)
        .unwrap();
    let (ok, _) = git(
        s.work.path(),
        &["clone", "-q", "ssh://taur@127.0.0.1/push-me.git", "nokey"],
        &env,
    );
    assert!(!ok);
}
