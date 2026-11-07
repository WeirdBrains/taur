//! Spins up the real server on a temp home and drives the `taurctl` binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use taur_core::Store;

struct Env {
    home: tempfile::TempDir,
    cfg: tempfile::TempDir,
    url: String,
    store: Arc<Store>,
    _rt: tokio::runtime::Runtime,
}

fn start() -> Env {
    let home = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(home.path()).unwrap());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = taur_server::app(store.clone());
    rt.spawn(async move { axum::serve(listener, app).await.unwrap() });
    Env {
        home,
        cfg: tempfile::tempdir().unwrap(),
        url,
        store,
        _rt: rt,
    }
}

impl Env {
    fn run_as(&self, cfg: &Path, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_taurctl"))
            .arg("--server")
            .arg(&self.url)
            .arg("--config-dir")
            .arg(cfg)
            .args(args)
            .env_remove("TAUR_SERVER")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    fn token(&self, email: &str, kind: &str) -> String {
        taur_core::mail::FileMailer::find_token(&self.home.path().join("outbox"), email, kind)
            .unwrap()
    }
    fn ok(&self, args: &[&str], stdin: &str) -> String {
        self.ok_as(self.cfg.path(), args, stdin)
    }
    fn ok_as(&self, cfg: &Path, args: &[&str], stdin: &str) -> String {
        let o = self.run_as(cfg, args, stdin);
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        assert!(
            o.status.success(),
            "{args:?}: {}{out}",
            String::from_utf8_lossy(&o.stderr)
        );
        out
    }
    fn fail(&self, cfg: &Path, args: &[&str], stdin: &str) -> (i32, String) {
        let o = self.run_as(cfg, args, stdin);
        assert!(!o.status.success(), "{args:?} should fail");
        (
            o.status.code().unwrap(),
            String::from_utf8_lossy(&o.stderr).to_string(),
        )
    }
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur")
}

#[test]
fn full_management_flow() {
    let e = start();
    let pw = "correct horse\n";
    e.ok(
        &["--password-stdin", "register", "alice", "--email", "a@x"],
        pw,
    );
    let (code, _) = e.fail(e.cfg.path(), &["whoami"], "");
    assert_eq!(code, 3);
    let out = e.ok(&["--password-stdin", "login", "alice"], pw);
    assert!(e.ok(&["whoami"], "").contains("(unverified)"));
    assert!(
        e.fail(e.cfg.path(), &["org", "create", "early"], "")
            .1
            .contains("not verified")
    );
    e.ok(&["resend-verification"], "");
    let vt = e.token("a@x", "verify");
    e.ok(&["verify", &vt], "");
    let token = std::fs::read_to_string(e.cfg.path().join("token")).unwrap();
    assert!(!out.contains(token.trim()), "token must not be printed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(e.cfg.path().join("token"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(e.ok(&["whoami"], "").contains("alice <a@x> (verified)"));

    e.store.import_dir(&fixture(), Some("alice")).unwrap();
    assert!(
        e.ok(&["search", "hello"], "")
            .contains("hello-taur-docs 1:1.2.3-1")
    );
    assert!(
        e.ok(&["search", "glibc", "--by", "depends"], "")
            .contains("hello-taur ")
    );
    let info = e.ok(&["--json", "info", "hello-taur"], "");
    let v: serde_json::Value = serde_json::from_str(&info).unwrap();
    assert_eq!(v["results"][0]["Maintainer"], "alice");
    assert!(
        e.fail(e.cfg.path(), &["search", "h"], "")
            .1
            .contains("Query arg too small")
    );

    // second user in its own config dir
    let bob = tempfile::tempdir().unwrap();
    e.ok_as(
        bob.path(),
        &["--password-stdin", "register", "bob", "--email", "b@x"],
        pw,
    );
    e.ok_as(bob.path(), &["--password-stdin", "login", "bob"], pw);
    e.ok_as(bob.path(), &["verify", &e.token("b@x", "verify")], "");

    // community commands
    e.ok_as(bob.path(), &["pkg", "vote", "hello-taur"], "");
    assert!(
        e.ok(&["info", "hello-taur"], "")
            .contains("NumVotes      : 1")
    );
    e.ok_as(
        bob.path(),
        &["pkg", "flag", "hello-taur", "--comment", "2.0 released"],
        "",
    );
    assert!(
        !e.ok(&["info", "hello-taur"], "")
            .contains("OutOfDate     : -")
    );
    e.ok(&["pkg", "unflag", "hello-taur"], "");
    e.ok_as(
        bob.path(),
        &["pkg", "comment", "hello-taur", "works for me"],
        "",
    );
    let comments = e.ok(&["pkg", "comments", "hello-taur"], "");
    assert!(comments.contains("bob") && comments.contains("works for me"));
    let cid = comments
        .trim_start_matches('#')
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    e.ok_as(bob.path(), &["comment", "edit", &cid, "works great"], "");
    e.ok(&["comment", "delete", &cid], "");
    e.ok(&["pkg", "comaintainers", "hello-taur", "bob"], "");
    assert!(
        e.ok(&["info", "hello-taur"], "")
            .contains("CoMaintainers : bob")
    );
    e.ok(&["pkg", "comaintainers", "hello-taur"], "");
    e.ok_as(bob.path(), &["pkg", "unvote", "hello-taur"], "");
    let filed = e.ok_as(
        bob.path(),
        &[
            "request",
            "file",
            "hello-taur",
            "--kind",
            "orphan",
            "--comment",
            "please",
        ],
        "",
    );
    let rid = filed
        .split('#')
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert!(
        e.ok(&["request", "list"], "")
            .contains("orphan hello-taur by bob [pending]")
    );
    assert!(
        e.fail(e.cfg.path(), &["request", "accept", &rid], "")
            .1
            .contains("403")
    );
    e.ok_as(bob.path(), &["request", "reject", &rid], "");
    assert!(
        e.ok(&["request", "show", &rid], "")
            .contains("status          : closed")
    );

    e.ok(&["org", "create", "brains"], "");
    e.ok(
        &["org", "add-member", "brains", "bob", "--role", "admin"],
        "",
    );
    assert!(
        e.ok_as(bob.path(), &["org", "list"], "")
            .contains("brains  admin")
    );
    let (code, err) = e.fail(
        bob.path(),
        &["org", "set-role", "brains", "alice", "contributor"],
        "",
    );
    assert_eq!(code, 1);
    assert!(err.contains("403"));
    e.ok(&["pkg", "transfer", "hello-taur", "--org", "brains"], "");
    assert!(
        e.ok_as(bob.path(), &["pkg", "list"], "")
            .contains("hello-taur  maintainer=alice org=brains")
    );
    e.ok_as(bob.path(), &["-y", "pkg", "disown", "hello-taur"], "");
    e.ok_as(bob.path(), &["pkg", "adopt", "hello-taur"], "");
    e.ok_as(
        bob.path(),
        &["pkg", "transfer", "hello-taur", "--user", "alice"],
        "",
    );
    assert!(
        e.ok(&["pkg", "list"], "")
            .contains("maintainer=alice org=-")
    );
    assert!(e.ok(&["org", "show", "brains"], "").contains("bob  admin"));
    e.ok(&["org", "set-role", "brains", "bob", "contributor"], "");
    e.ok(&["org", "remove-member", "brains", "bob"], "");
    // destructive commands need -y when not interactive
    assert!(
        e.fail(e.cfg.path(), &["org", "delete", "brains"], "")
            .1
            .contains("-y")
    );
    e.ok(&["-y", "org", "delete", "brains"], "");

    let pubkey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl test@host";
    assert!(
        e.ok_as(bob.path(), &["key", "add", "-"], pubkey)
            .contains("SHA256:")
    );
    let list = e.ok_as(bob.path(), &["key", "list"], "");
    assert!(list.contains("ssh-ed25519") && list.contains("test@host"));
    let id = list.split_whitespace().next().unwrap().to_string();
    e.ok_as(bob.path(), &["key", "remove", &id], "");
    assert!(e.ok_as(bob.path(), &["key", "list"], "").is_empty());
    e.ok_as(bob.path(), &["logout"], "");
    e.ok(&["reset-password", "request", "b@x"], "");
    let rt = e.token("b@x", "reset");
    e.ok_as(
        bob.path(),
        &[
            "--password-stdin",
            "reset-password",
            "confirm",
            "--token",
            &rt,
        ],
        "new password!\n",
    );
    e.ok_as(
        bob.path(),
        &["--password-stdin", "login", "bob"],
        "new password!\n",
    );
    assert!(bob.path().join("token").exists());

    assert!(
        e.fail(e.cfg.path(), &["--password-stdin", "unregister"], pw)
            .1
            .contains("-y")
    );
    e.ok(&["-y", "--password-stdin", "unregister"], pw);
    assert!(!e.cfg.path().join("token").exists());
    assert_eq!(
        e.fail(e.cfg.path(), &["--password-stdin", "login", "alice"], pw)
            .0,
        3
    );
}

#[test]
fn moderation_accept_is_admin_only_and_the_gate_reads_it() {
    let e = start();
    let pw = "correct horse\n";
    e.ok(
        &[
            "--password-stdin",
            "register",
            "admin",
            "--email",
            "admin@example.com",
        ],
        pw,
    );
    e.ok(&["verify", &e.token("admin@example.com", "verify")], "");
    e.ok(&["--password-stdin", "login", "admin"], pw);
    let admin = e.store.user_by_name("admin").unwrap().unwrap().id;
    e.store.set_admin(admin, true).unwrap();
    e.store.insert_registry("vendor", "enforce").unwrap();
    e.store
        .insert_identity("vendor", "carol", "carol@example.com")
        .unwrap();
    let mod_cfg = tempfile::tempdir().unwrap();
    e.ok_as(
        mod_cfg.path(),
        &[
            "--password-stdin",
            "register",
            "moddy",
            "--email",
            "mod@example.com",
        ],
        pw,
    );
    e.ok_as(
        mod_cfg.path(),
        &["verify", &e.token("mod@example.com", "verify")],
        "",
    );
    e.ok_as(mod_cfg.path(), &["--password-stdin", "login", "moddy"], pw);
    e.store
        .grant_moderator(e.store.user_by_name("moddy").unwrap().unwrap().id, true)
        .unwrap();
    let (_code, err) = e.fail(
        e.cfg.path(),
        &[
            "--password-stdin",
            "register",
            "carol",
            "--email",
            "other@example.com",
        ],
        pw,
    );
    assert!(err.contains("operator identity"), "{err}");
    let audit = e.ok(&["moderation", "audit"], "");
    assert!(audit.contains("registration"));
    assert!(!audit.contains("other@example.com"));
    assert!(!audit.contains("correct horse"));
    let id = audit.split_whitespace().next().unwrap().to_string();
    let (_code, err) = e.fail(mod_cfg.path(), &["moderation", "accept", &id, "no"], "");
    assert!(err.contains("forbidden"), "{err}");
    assert!(
        e.ok(&["moderation", "accept", &id, "this claim"], "")
            .contains("accepted")
    );
    let (_code, err) = e.fail(
        e.cfg.path(),
        &[
            "--password-stdin",
            "register",
            "carol",
            "--email",
            "third@example.com",
        ],
        pw,
    );
    assert!(err.contains("operator identity"), "{err}");
    e.ok(
        &[
            "--password-stdin",
            "register",
            "carol",
            "--email",
            "other@example.com",
        ],
        pw,
    );
    assert!(!e.store.user_by_name("carol").unwrap().unwrap().verified);
}

#[test]
fn config_server_setting() {
    let e = start();
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_taurctl"))
            .arg("--config-dir")
            .arg(dir.path())
            .args(args)
            .env_remove("TAUR_SERVER")
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).to_string()
    };
    assert!(run(&["config", "show"]).contains("server: http://127.0.0.1:8080"));
    run(&["config", "set-server", &e.url]);
    assert!(run(&["config", "show"]).contains(&format!("server: {}", e.url)));
    assert!(run(&["search", "nothing-here"]).is_empty());
}

#[test]
fn man_pages_up_to_date() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../man");
    let update = std::env::var_os("TAUR_UPDATE_MAN").is_some();
    for (name, roff) in taur_cli::man_pages() {
        let path = dir.join(&name);
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &roff).unwrap();
        } else {
            let current = std::fs::read(&path).unwrap_or_default();
            assert!(
                current == roff,
                "{name} is out of date: run TAUR_UPDATE_MAN=1 cargo test -p taur-cli"
            );
        }
    }
}

#[test]
fn mcp_stdio_proxy() {
    let e = start();
    e.store.import_dir(&fixture(), None).unwrap();
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"package_info","arguments":{"names":["hello-taur"]}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"whoami","arguments":{}}}"#,
    ]
    .join("\n");
    let out = e.ok(&["mcp"], &(input + "\n"));
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3, "{out}");
    assert_eq!(lines[0]["result"]["serverInfo"]["name"], "taur");
    assert_eq!(
        lines[1]["result"]["structuredContent"]["results"][0]["Name"],
        "hello-taur"
    );
    assert_eq!(lines[2]["result"]["isError"], true);
}
