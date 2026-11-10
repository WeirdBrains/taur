//! Git hooks receive `TAUR_DB` from the server and honor env/default precedence.
//! Does not touch a production database.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use taur_core::Store;
use taur_server::{AppState, app_with};

fn taur() -> Command {
    Command::new(env!("CARGO_BIN_EXE_taur"))
}

fn hook_pre_receive(
    repo: &Path,
    home: &Path,
    db: Option<&Path>,
    pusher: &str,
    stdin: &str,
) -> (i32, String) {
    let mut cmd = taur();
    cmd.args(["hook", "pre-receive"])
        .current_dir(repo)
        .env("TAUR_HOME", home)
        .env("TAUR_PUSHER", pusher)
        .env_remove("TAUR_DB")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(db) = db {
        cmd.env("TAUR_DB", db);
    }
    let mut child = cmd.spawn().unwrap();
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn register(store: &Store, name: &str) {
    let email = format!("{name}@x");
    store.register(name, &email, "longenough").unwrap();
    let token =
        taur_core::mail::FileMailer::find_token(&store.home().join("outbox"), &email, "verify")
            .unwrap();
    store.verify_email(&token).unwrap();
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

#[test]
fn hook_honors_taur_db_over_the_default_under_home() {
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let custom = other.path().join("elsewhere.db");
    let custom_store = Store::open_with_db(home.path(), &custom).unwrap();
    register(&custom_store, "alice");
    // A default database under home exists and does not have alice.
    let default_store = Store::open(home.path()).unwrap();
    assert!(default_store.user_by_name("alice").unwrap().is_none());
    drop(default_store);

    let repo = home.path().join("repos").join("hook-db.git");
    std::fs::create_dir_all(&repo).unwrap();
    // The hook looks up the pusher in the opened database before talking to git.
    let zeros = "0".repeat(40);
    let ones = "1".repeat(40);
    let stdin = format!("{zeros} {ones} refs/heads/master\n");

    let (code, out) = hook_pre_receive(&repo, home.path(), None, "alice", &stdin);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("unknown user alice"),
        "without TAUR_DB the hook must open <home>/taur.db, which has no alice: {out}"
    );

    let (code, out) = hook_pre_receive(&repo, home.path(), Some(&custom), "alice", &stdin);
    assert!(
        !out.contains("unknown user alice"),
        "with TAUR_DB the hook must open the custom database that has alice: {out}"
    );
    // The hook should get past the user lookup. A later git error is fine;
    // this stdin is not a real push.
    assert!(code != 0, "{out}");
}

#[test]
fn http_push_propagates_taur_db_and_writes_that_database() {
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let db = other.path().join("elsewhere.db");
    let store = Arc::new(Store::open_with_db(home.path(), &db).unwrap());
    store
        .configure_mail(None, "taur@test", "http://taur.test")
        .unwrap();
    register(&store, "alice");
    let token = store.login("alice", "longenough").unwrap().token;

    let env_log = home.path().join("hook.env");
    let wrapper = home.path().join("hook-wrapper");
    let real = env!("CARGO_BIN_EXE_taur");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n{{\n  printf 'TAUR_HOME=%s\\n' \"$TAUR_HOME\"\n  printf 'TAUR_DB=%s\\n' \"$TAUR_DB\"\n}} > '{}'\nexec '{}' \"$@\"\n",
            env_log.display(),
            Path::new(real).display()
        ),
    )
    .unwrap();
    let mut perm = std::fs::metadata(&wrapper).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&wrapper, perm).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener =
        rt.block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap() });
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = app_with(AppState {
        store: store.clone(),
        hook_bin: wrapper,
    });
    rt.spawn(async move { axum::serve(listener, app).await.unwrap() });

    let work = tempfile::tempdir().unwrap();
    let w = work.path().join("db-push");
    write_pkg(&w, "db-push", "1.0");
    assert!(git(&w, &["init", "-q"], &[]).0);
    git(&w, &["add", "-A"], &[]);
    git(&w, &["commit", "-qm", "init"], &[]);
    let host = url.trim_start_matches("http://");
    let (ok, out) = git(
        &w,
        &[
            "push",
            &format!("http://alice:{token}@{host}/db-push.git"),
            "master",
        ],
        &[],
    );
    assert!(ok, "{out}");

    let logged = std::fs::read_to_string(&env_log).unwrap();
    assert!(
        logged.contains(&format!("TAUR_HOME={}", store.home().display())),
        "{logged}"
    );
    assert!(
        logged.contains(&format!("TAUR_DB={}", store.db_path().display())),
        "{logged}"
    );

    let pkgs = store.info(&["db-push".into()]).unwrap();
    assert_eq!(
        pkgs.len(),
        1,
        "package must be recorded in the custom database"
    );
    assert_eq!(pkgs[0].maintainer.as_deref(), Some("alice"));

    let default = Store::open(home.path()).unwrap();
    assert!(
        default.info(&["db-push".into()]).unwrap().is_empty(),
        "the default <home>/taur.db must stay unused"
    );
}
