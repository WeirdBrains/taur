//! `taur ssh-auth` and `taur git-serve`: OpenSSH AuthorizedKeysCommand and the
//! aurweb-style SSH command set (list, vote, keywords, disown, adopt).

use std::path::Path;
use std::process::Command;
use taur_core::Store;

fn taur(home: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_taur"))
        .arg("--home")
        .arg(home)
        .args(args)
        .envs(env.iter().copied())
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn ed25519(dir: &Path) -> (String, String) {
    let path = dir.join("id_ed25519");
    let out = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&path)
        .output()
        .expect("ssh-keygen");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line = std::fs::read_to_string(path.with_extension("pub")).unwrap();
    let mut parts = line.split_whitespace();
    (
        parts.next().unwrap().to_string(),
        parts.next().unwrap().to_string(),
    )
}

#[test]
fn ssh_auth_and_git_serve_commands() {
    let home = tempfile::tempdir().unwrap();
    let keys = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    let alice = store.register("alice", "a@x", "longenough").unwrap();
    store.mark_verified(alice.id).unwrap();
    let bob = store.register("bob", "b@x", "longenough").unwrap();
    store.mark_verified(bob.id).unwrap();
    let carol = store.register("carol", "c@x", "longenough").unwrap();
    let fx = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/hello-taur");
    store.import_dir(&fx, Some("alice")).unwrap();
    let (ty, b64) = ed25519(keys.path());
    let offered = format!("{ty} {b64}");
    store.add_ssh_key(alice.id, &offered).unwrap();
    drop(store);

    let (code, stdout, stderr) = taur(home.path(), &["ssh-auth", &ty, &b64], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("git-serve --user alice"), "{stdout}");
    assert!(stdout.contains(&b64), "{stdout}");
    assert!(stdout.starts_with("restrict,command="), "{stdout}");

    let (code, stdout, _) = taur(home.path(), &["ssh-auth", "ssh-ed25519", "not-a-key"], &[]);
    assert_eq!(code, 0);
    assert!(stdout.is_empty(), "{stdout}");

    let (code, stdout, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "alice"],
        &[("SSH_ORIGINAL_COMMAND", "list-repos")],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("hello-taur"), "{stdout}");

    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "alice"],
        &[("SSH_ORIGINAL_COMMAND", "help")],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("set-comaintainers"), "{stderr}");

    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "alice"],
        &[("SSH_ORIGINAL_COMMAND", "vote hello-taur")],
    );
    assert_eq!(code, 0, "{stderr}");
    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "alice"],
        &[("SSH_ORIGINAL_COMMAND", "vote hello-taur")],
    );
    assert_ne!(code, 0);
    assert!(stderr.contains("already voted"), "{stderr}");

    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "alice"],
        &[("SSH_ORIGINAL_COMMAND", "set-keywords hello-taur demo")],
    );
    assert_eq!(code, 0, "{stderr}");
    let store = Store::open(home.path()).unwrap();
    let info = store.info(&["hello-taur".into()]).unwrap();
    assert_eq!(info[0].keywords, vec!["demo".to_string()]);
    assert!(store.has_voted(alice.id, "hello-taur").unwrap());
    drop(store);

    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "bob"],
        &[("SSH_ORIGINAL_COMMAND", "disown hello-taur")],
    );
    assert_ne!(code, 0);
    assert!(stderr.contains("permission denied"), "{stderr}");

    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "alice"],
        &[("SSH_ORIGINAL_COMMAND", "flag hello-taur no")],
    );
    assert_ne!(code, 0, "{stderr}");

    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "bob"],
        &[("SSH_ORIGINAL_COMMAND", "flag hello-taur 'upstream moved'")],
    );
    assert_eq!(code, 0, "{stderr}");

    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "alice"],
        &[("SSH_ORIGINAL_COMMAND", "disown hello-taur")],
    );
    assert_eq!(code, 0, "{stderr}");

    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "bob"],
        &[("SSH_ORIGINAL_COMMAND", "adopt hello-taur")],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("adoption request"), "{stderr}");
    let store = Store::open(home.path()).unwrap();
    assert_eq!(
        store.info(&["hello-taur".into()]).unwrap()[0]
            .maintainer
            .as_deref(),
        None
    );
    drop(store);

    // Unverified accounts may list repos but not vote.
    let _ = carol;
    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "carol"],
        &[("SSH_ORIGINAL_COMMAND", "list-repos")],
    );
    assert_eq!(code, 0, "{stderr}");
    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "carol"],
        &[("SSH_ORIGINAL_COMMAND", "vote hello-taur")],
    );
    assert_ne!(code, 0);
    assert!(stderr.contains("not verified"), "{stderr}");

    let (code, _, stderr) = taur(
        home.path(),
        &["git-serve", "--user", "alice"],
        &[("SSH_ORIGINAL_COMMAND", "")],
    );
    assert_ne!(code, 0);
    assert!(stderr.contains("Interactive shell is disabled"), "{stderr}");
}
