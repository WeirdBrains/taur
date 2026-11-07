use std::fs;
use std::path::Path;
use taur_core::registry::{self, PROVENANCE_OPERATOR, UploadDecision};
use taur_core::{Error, Store};

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

fn recipe(ver: &str) -> (String, String) {
    let srcinfo = format!("pkgbase = demo\npkgver = {ver}\npkgrel = 1\n\npkgname = demo\n");
    let pkgbuild = format!("pkgver={ver}\npkgrel=1\npkgname=(demo)\n");
    (srcinfo, pkgbuild)
}

fn write_upload(
    dir: &Path,
    registry: &str,
    repository: &str,
    srcinfo: &str,
    pkgbuild: &str,
    signature: Option<&[u8]>,
) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(".SRCINFO"), srcinfo).unwrap();
    fs::write(dir.join("PKGBUILD"), pkgbuild).unwrap();
    fs::write(dir.join("TAUR-REGISTRY"), format!("{registry}\n")).unwrap();
    fs::write(dir.join("TAUR-REPOSITORY"), format!("{repository}\n")).unwrap();
    if let Some(sig) = signature {
        fs::write(dir.join("TAUR-RECIPE.sig"), hex(sig)).unwrap();
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn legacy_registry_keeps_existing_upload_rules() {
    let (dir, store) = open();
    let (src, pkg) = recipe("1.0.0");
    write_upload(&dir.path().join("pkg"), "local", "extra", &src, &pkg, None);
    store
        .import_into(&dir.path().join("pkg"), None, None)
        .unwrap();
    let rows = store.list_registries().unwrap();
    assert_eq!(rows[0].name, "local");
    assert_eq!(rows[0].enforcement, "exempt");
    assert_eq!(rows[0].provenance, PROVENANCE_OPERATOR);
    assert_eq!(
        store.pkgbase_registry("demo").unwrap().as_deref(),
        Some("local")
    );
}

#[test]
fn enforcing_registry_verifies_the_recipe_and_rejects_without_storing_it() {
    let (root, store) = open();
    let admin = store
        .register("admin", "admin@example.com", "correct horse")
        .unwrap()
        .id;
    let alice = store
        .register("alice", "alice@example.com", "correct horse")
        .unwrap()
        .id;
    let bob = store
        .register("bob", "bob@example.com", "correct horse")
        .unwrap()
        .id;
    store.set_admin(admin, true).unwrap();
    store.mark_verified(admin).unwrap();
    store.mark_verified(alice).unwrap();
    let modr = store
        .register("moddy", "mod@example.com", "correct horse")
        .unwrap()
        .id;
    store.grant_moderator(modr, true).unwrap();
    assert!(matches!(
        store.create_registry(modr, "vendor"),
        Err(Error::Forbidden)
    ));
    store.create_registry(admin, "vendor").unwrap();

    let (signing, verifying) = registry::generate_key();
    let fp = store
        .add_registry_key(admin, "vendor", "lab", verifying.as_bytes())
        .unwrap();
    store
        .bind_identity(admin, "vendor", "alice", "alice@example.com")
        .unwrap();
    let (src, pkg) = recipe("1.0.0");
    let sig = registry::sign_recipe(
        signing.as_bytes(),
        "vendor",
        "extra",
        "demo",
        "1.0.0-1",
        src.as_bytes(),
        pkg.as_bytes(),
    );
    store
        .expect_package(
            admin, "vendor", "demo", "extra", "1.0.0-1", &sig, &fp, "alice",
        )
        .unwrap();

    let good = root.path().join("good");
    write_upload(&good, "vendor", "extra", &src, &pkg, Some(&sig));
    store
        .import_into(&good, Some("alice"), Some("vendor"))
        .unwrap();
    assert_eq!(
        store.pkgbase_registry("demo").unwrap().as_deref(),
        Some("vendor")
    );

    let (src2, pkg2) = recipe("1.0.0");
    let mut tampered = pkg2.clone();
    tampered.push_str("# tampered\n");
    let bad = root.path().join("bad");
    write_upload(&bad, "vendor", "extra", &src2, &tampered, Some(&sig));
    let err = store
        .import_into(&bad, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
    assert!(store.pkgbase_id("other").unwrap().is_none());

    let other_sig = registry::sign_recipe(
        signing.as_bytes(),
        "vendor",
        "core",
        "demo",
        "1.0.0-1",
        src.as_bytes(),
        pkg.as_bytes(),
    );
    let swapped = root.path().join("repo");
    write_upload(&swapped, "vendor", "core", &src, &pkg, Some(&other_sig));
    let err = store
        .import_into(&swapped, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("repository"), "{err}");

    let (src_new, pkg_new) = recipe("2.0.0");
    let new_sig = registry::sign_recipe(
        signing.as_bytes(),
        "vendor",
        "extra",
        "demo",
        "2.0.0-1",
        src_new.as_bytes(),
        pkg_new.as_bytes(),
    );
    let newer = root.path().join("new");
    write_upload(
        &newer,
        "vendor",
        "extra",
        &src_new,
        &pkg_new,
        Some(&new_sig),
    );
    let err = store
        .import_into(&newer, Some("bob"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("nick and email"), "{err}");
    assert_eq!(
        store
            .check_upload(&taur_core::registry::RecipeUpload {
                registry: "vendor",
                repository: Some("extra"),
                pkgbase: "demo",
                version: "2.0.0-1",
                srcinfo: src_new.as_bytes(),
                pkgbuild: pkg_new.as_bytes(),
                signature: Some(&new_sig),
                actor: Some(alice),
            })
            .unwrap(),
        UploadDecision::NewVersion
    );
    store
        .import_into(&newer, Some("alice"), Some("vendor"))
        .unwrap();

    let (src_old, pkg_old) = recipe("1.5.0");
    let old_sig = registry::sign_recipe(
        signing.as_bytes(),
        "vendor",
        "extra",
        "demo",
        "1.5.0-1",
        src_old.as_bytes(),
        pkg_old.as_bytes(),
    );
    let down = root.path().join("down");
    write_upload(&down, "vendor", "extra", &src_old, &pkg_old, Some(&old_sig));
    let err = store
        .import_into(&down, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("not newer"), "{err}");

    let (_other, other_key) = registry::generate_key();
    let foreign = registry::sign_recipe(
        other_key.as_bytes(),
        "vendor",
        "extra",
        "demo",
        "3.0.0-1",
        src_new.as_bytes(),
        pkg_new.as_bytes(),
    );
    let untrusted = root.path().join("untrusted");
    let (src3, pkg3) = recipe("3.0.0");
    write_upload(&untrusted, "vendor", "extra", &src3, &pkg3, Some(&foreign));
    let err = store
        .import_into(&untrusted, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("trusted key"), "{err}");

    let moved = root.path().join("moved");
    write_upload(&moved, "local", "extra", &src_new, &pkg_new, None);
    let err = store
        .import_into(&moved, Some("alice"), Some("local"))
        .unwrap_err();
    assert!(err.to_string().contains("cannot be moved"), "{err}");
    let _ = bob;
}

#[test]
fn registration_rejects_a_conflicting_operator_identity_before_creating_an_account() {
    let (_d, store) = open();
    let admin = store
        .register("admin", "admin@example.com", "correct horse")
        .unwrap()
        .id;
    store.set_admin(admin, true).unwrap();
    store.insert_registry("vendor", "enforce").unwrap();
    store
        .insert_identity("vendor", "carol", "carol@example.com")
        .unwrap();
    let before = store.user_by_name("carol").unwrap();
    let err = store
        .register("carol", "other@example.com", "correct horse")
        .unwrap_err();
    assert!(err.to_string().contains("operator identity"), "{err}");
    assert!(store.user_by_name("carol").unwrap().is_none());
    assert!(before.is_none());
    let carol = store
        .register("carol", "carol@example.com", "correct horse")
        .unwrap();
    assert!(!carol.email.is_empty());
    let rows = store.list_audit(admin).unwrap();
    let event = rows
        .iter()
        .find(|e| e.target_kind == "registration")
        .unwrap();
    assert!(!event.reason.contains("correct horse"));
    assert!(!event.reason.contains("other@example.com"));
    store.reverse_audit(admin, event.id, "reviewed").unwrap();
    assert!(store.user_by_name("intruder").unwrap().is_none());
    let again = store.register("intruder", "other@example.com", "correct horse");
    assert!(again.is_ok(), "a nick with no identity is not a mismatch");
    let err = store
        .register("carol", "third@example.com", "correct horse")
        .unwrap_err();
    assert!(
        err.to_string().contains("already registered")
            || err.to_string().contains("operator identity"),
        "{err}"
    );
}

#[test]
fn admin_acceptance_permits_only_the_same_attempt() {
    let (root, store) = open();
    let admin = store
        .register("admin", "admin@example.com", "correct horse")
        .unwrap()
        .id;
    let alice = store
        .register("alice", "alice@example.com", "correct horse")
        .unwrap()
        .id;
    let bob = store
        .register("bob", "bob@example.com", "correct horse")
        .unwrap()
        .id;
    store.set_admin(admin, true).unwrap();
    let modr = store
        .register("moddy", "mod@example.com", "correct horse")
        .unwrap()
        .id;
    store.grant_moderator(modr, true).unwrap();
    store.insert_registry("vendor", "enforce").unwrap();
    let (signing, verifying) = registry::generate_key();
    let fp = store
        .insert_registry_key("vendor", "lab", verifying.as_bytes())
        .unwrap();
    store
        .insert_identity("vendor", "alice", "alice@example.com")
        .unwrap();
    let (src, pkg) = recipe("1.0.0");
    let sig = registry::sign_recipe(
        signing.as_bytes(),
        "vendor",
        "extra",
        "demo",
        "1.0.0-1",
        src.as_bytes(),
        pkg.as_bytes(),
    );
    store
        .insert_expectation("vendor", "demo", "extra", "1.0.0-1", &sig, &fp, "alice")
        .unwrap();
    let mut bad_pkg = pkg.clone();
    bad_pkg.push_str("# changed\n");
    let bad = root.path().join("bad");
    write_upload(&bad, "vendor", "extra", &src, &bad_pkg, Some(&sig));
    let err = store
        .import_into(&bad, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
    assert!(store.pkgbase_id("demo").unwrap().is_none());
    let rows = store.list_audit(admin).unwrap();
    let rejection = rows
        .iter()
        .find(|e| e.target_kind == "registry_upload" && e.open)
        .unwrap();
    assert!(!rejection.reason.contains("# changed"));
    assert_eq!(store.user_by_id(alice).unwrap().name, "alice");
    store
        .reverse_audit(admin, rejection.id, "reviewed only")
        .unwrap();
    let err = store
        .import_into(&bad, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(
        err.to_string().contains("rejected"),
        "reversing the audit row does not permit the retry: {err}"
    );
    let rejection = store
        .list_audit(admin)
        .unwrap()
        .into_iter()
        .find(|e| e.target_kind == "registry_upload" && e.open)
        .unwrap();
    assert!(matches!(
        store.accept_rejection(modr, rejection.id, "allow this file"),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        store.accept_rejection(bob, rejection.id, "allow this file"),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        store.accept_rejection(admin, rejection.id, "   "),
        Err(Error::Invalid(_))
    ));
    let accepted = store
        .accept_rejection(admin, rejection.id, "allow this file")
        .unwrap();
    assert_eq!(
        store
            .accept_rejection(admin, rejection.id, "allow this file")
            .unwrap(),
        accepted
    );
    let history = store.list_audit(admin).unwrap();
    let accept_row = history.iter().find(|e| e.id == accepted).unwrap();
    assert_eq!(accept_row.action, "accept");
    assert_eq!(accept_row.actor.as_deref(), Some("admin"));
    assert_eq!(accept_row.reason, "allow this file");
    assert_eq!(accept_row.reverses, Some(rejection.id));
    assert!(accept_row.at > 0);
    assert!(
        history
            .iter()
            .any(|e| e.action == "reject" && e.target_kind == "registry_upload")
    );
    assert!(matches!(
        store.revoke_acceptance(modr, accepted, "no"),
        Err(Error::Forbidden)
    ));
    let junk = root.path().join("junk");
    fs::create_dir_all(&junk).unwrap();
    fs::write(junk.join(".SRCINFO"), "this is not srcinfo\n").unwrap();
    fs::write(junk.join("PKGBUILD"), "pkgver=1\n").unwrap();
    let err = store
        .import_into(&junk, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(
        !err.to_string().contains("allow this file"),
        "a malformed recipe is not accepted: {err}"
    );
    let other_repo = root.path().join("other-repo");
    write_upload(&other_repo, "vendor", "core", &src, &bad_pkg, Some(&sig));
    let err = store
        .import_into(&other_repo, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
    let (src_new, pkg_new) = recipe("2.0.0");
    let mut bad_new = pkg_new.clone();
    bad_new.push_str("# changed\n");
    let other_ver = root.path().join("other-ver");
    write_upload(
        &other_ver,
        "vendor",
        "extra",
        &src_new,
        &bad_new,
        Some(&sig),
    );
    let err = store
        .import_into(&other_ver, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
    let new_sig = registry::sign_recipe(
        signing.as_bytes(),
        "vendor",
        "extra",
        "demo",
        "2.0.0-1",
        src_new.as_bytes(),
        pkg_new.as_bytes(),
    );
    let err = store
        .check_upload(&taur_core::registry::RecipeUpload {
            registry: "vendor",
            repository: Some("extra"),
            pkgbase: "demo",
            version: "2.0.0-1",
            srcinfo: src_new.as_bytes(),
            pkgbuild: pkg_new.as_bytes(),
            signature: Some(&new_sig),
            actor: Some(bob),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("nick and email"),
        "a valid newer signature by someone else is still rejected: {err}"
    );
    assert_eq!(
        store
            .check_upload(&taur_core::registry::RecipeUpload {
                registry: "vendor",
                repository: Some("extra"),
                pkgbase: "demo",
                version: "2.0.0-1",
                srcinfo: src_new.as_bytes(),
                pkgbuild: pkg_new.as_bytes(),
                signature: Some(&new_sig),
                actor: Some(alice),
            })
            .unwrap(),
        UploadDecision::NewVersion
    );
    let suspended = store
        .reject_user(admin, "alice", "account is suspended")
        .unwrap();
    let err = store
        .import_into(&bad, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(
        err.to_string().contains("suspended"),
        "acceptance does not unsuspend the applicant: {err}"
    );
    store.reverse_audit(admin, suspended, "cleared").unwrap();
    let store = std::sync::Arc::new(store);
    let gate = store.clone();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let gate = gate.clone();
            let srcinfo = src.clone();
            let pkgbuild = bad_pkg.clone();
            let signature = sig;
            scope.spawn(move || {
                assert_eq!(
                    gate.check_upload(&taur_core::registry::RecipeUpload {
                        registry: "vendor",
                        repository: Some("extra"),
                        pkgbase: "demo",
                        version: "1.0.0-1",
                        srcinfo: srcinfo.as_bytes(),
                        pkgbuild: pkgbuild.as_bytes(),
                        signature: Some(&signature),
                        actor: Some(alice),
                    })
                    .unwrap(),
                    UploadDecision::Overridden
                );
            });
        }
    });
    let again_accept = store.clone();
    std::thread::scope(|scope| {
        let mut joins = Vec::new();
        for _ in 0..4 {
            let again_accept = again_accept.clone();
            joins.push(scope.spawn(move || {
                again_accept
                    .accept_rejection(admin, rejection.id, "allow this file")
                    .unwrap()
            }));
        }
        for join in joins {
            assert_eq!(join.join().unwrap(), accepted);
        }
    });
    store
        .import_into(&bad, Some("alice"), Some("vendor"))
        .unwrap();
    store
        .import_into(&bad, Some("alice"), Some("vendor"))
        .unwrap();
    let again = root.path().join("bob");
    write_upload(&again, "vendor", "extra", &src, &bad_pkg, Some(&sig));
    let err = store
        .import_into(&again, Some("bob"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
    let mut other = bad_pkg.clone();
    other.push_str("# more\n");
    let changed = root.path().join("changed");
    write_upload(&changed, "vendor", "extra", &src, &other, Some(&sig));
    let err = store
        .import_into(&changed, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
    store
        .revoke_acceptance(admin, accepted, "withdraw")
        .unwrap();
    let original = root.path().join("original-again");
    write_upload(&original, "vendor", "extra", &src, &bad_pkg, Some(&sig));
    let err = store
        .import_into(&original, Some("alice"), Some("vendor"))
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
    assert!(matches!(
        store.accept_rejection(admin, rejection.id, "stale"),
        Err(Error::Conflict(_))
    ));
    let latest = store
        .list_audit(admin)
        .unwrap()
        .into_iter()
        .rfind(|e| e.target_kind == "registry_upload" && e.action == "reject" && e.open)
        .unwrap();
    store
        .accept_rejection(admin, latest.id, "allow the same file again")
        .unwrap();
    store
        .import_into(&original, Some("alice"), Some("vendor"))
        .unwrap();

    assert_eq!(
        store
            .check_upload(&taur_core::registry::RecipeUpload {
                registry: "vendor",
                repository: Some("extra"),
                pkgbase: "demo",
                version: "1.0.0-1",
                srcinfo: src.as_bytes(),
                pkgbuild: pkg.as_bytes(),
                signature: Some(&sig),
                actor: Some(alice),
            })
            .unwrap(),
        UploadDecision::Exact,
        "accepting a bad recipe does not replace the operator expectation"
    );
    store
        .insert_identity("vendor", "carol", "carol@example.com")
        .unwrap();
    store.insert_registry("other", "enforce").unwrap();
    store
        .insert_identity("other", "carol", "kept@example.com")
        .unwrap();
    let err = store
        .register("carol", "other@example.com", "correct horse")
        .unwrap_err();
    assert!(err.to_string().contains("operator identity"), "{err}");
    assert!(store.user_by_name("carol").unwrap().is_none());
    let reg = store
        .list_audit(admin)
        .unwrap()
        .into_iter()
        .rfind(|e| e.target_kind == "registration" && e.action == "reject" && e.open)
        .unwrap();
    assert!(!reg.reason.contains("correct horse"));
    assert!(!reg.reason.contains("other@example.com"));
    let db = fs::read(root.path().join("taur.db")).unwrap();
    let db = String::from_utf8_lossy(&db);
    assert!(!db.contains("correct horse"));
    assert!(!db.contains("other@example.com"));
    store
        .accept_rejection(admin, reg.id, "this claim is allowed")
        .unwrap();
    let err = store
        .register("carol", "third@example.com", "correct horse")
        .unwrap_err();
    assert!(err.to_string().contains("operator identity"), "{err}");
    let err = store
        .register("carol", "other@example.com", "correct horse")
        .unwrap_err();
    assert!(
        err.to_string().contains("operator identity"),
        "the other registry's identity is a separate claim: {err}"
    );
    let second = store
        .list_audit(admin)
        .unwrap()
        .into_iter()
        .rfind(|e| e.target_kind == "registration" && e.action == "reject" && e.open)
        .unwrap();
    assert_ne!(second.id, reg.id);
    store
        .accept_rejection(admin, second.id, "the other registry too")
        .unwrap();
    let pending = store.clone();
    let joined: Vec<_> = std::thread::scope(|scope| {
        let mut joins = Vec::new();
        for _ in 0..2 {
            let pending = pending.clone();
            joins
                .push(scope.spawn(move || {
                    pending.register("carol", "other@example.com", "correct horse")
                }));
        }
        joins.into_iter().map(|j| j.join().unwrap()).collect()
    });
    assert_eq!(joined.iter().filter(|r| r.is_ok()).count(), 1);
    for result in &joined {
        if let Err(err) = result {
            let msg = err.to_string();
            assert!(!msg.contains("operator identity"), "{msg}");
            assert!(!msg.contains("forbidden"), "{msg}");
        }
    }
    let carol = store.user_by_name("carol").unwrap().unwrap();
    assert_eq!(carol.name, "carol");
    assert!(!carol.verified);
}

#[test]
fn an_unknown_name_on_an_enforcing_registry_uses_existing_rules() {
    let (dir, store) = open();
    store.insert_registry("vendor", "enforce").unwrap();
    let (src, pkg) = recipe("1.0.0");
    write_upload(&dir.path().join("pkg"), "vendor", "extra", &src, &pkg, None);
    store
        .import_into(&dir.path().join("pkg"), None, Some("vendor"))
        .unwrap();
    assert_eq!(
        store.pkgbase_registry("demo").unwrap().as_deref(),
        Some("vendor")
    );
}

fn git_commit(dir: &Path) -> String {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?} {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "-q", "-b", "master"]);
    run(&["add", "."]);
    run(&[
        "-c",
        "user.email=t@example.com",
        "-c",
        "user.name=t",
        "commit",
        "-q",
        "-m",
        "recipe",
    ]);
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn push_of(sha: &str) -> taur_core::push::RefUpdate {
    taur_core::push::RefUpdate {
        old: "0".repeat(40),
        new: sha.to_string(),
        refname: "refs/heads/master".into(),
    }
}

#[test]
fn git_pre_receive_honors_the_same_acceptance() {
    let (root, store) = open();
    let admin = store
        .register("admin", "admin@example.com", "correct horse")
        .unwrap()
        .id;
    let alice = store
        .register("alice", "alice@example.com", "correct horse")
        .unwrap()
        .id;
    store.set_admin(admin, true).unwrap();
    store.insert_registry("vendor", "enforce").unwrap();
    let (signing, verifying) = registry::generate_key();
    let fp = store
        .insert_registry_key("vendor", "lab", verifying.as_bytes())
        .unwrap();
    store
        .insert_identity("vendor", "alice", "alice@example.com")
        .unwrap();
    let (src, pkg) = recipe("1.0.0");
    let sig = registry::sign_recipe(
        signing.as_bytes(),
        "vendor",
        "extra",
        "demo",
        "1.0.0-1",
        src.as_bytes(),
        pkg.as_bytes(),
    );
    store
        .insert_expectation("vendor", "demo", "extra", "1.0.0-1", &sig, &fp, "alice")
        .unwrap();
    let mut bad_pkg = pkg.clone();
    bad_pkg.push_str("# changed\n");
    let dir = root.path().join("push");
    write_upload(&dir, "vendor", "extra", &src, &bad_pkg, Some(&sig));
    let sha = git_commit(&dir);
    let git_dir = dir.join(".git");
    let update = push_of(&sha);
    assert!(matches!(
        store.pre_receive(&git_dir, "demo", "alice", std::slice::from_ref(&update)),
        Err(Error::Unverified)
    ));
    store.mark_verified(alice).unwrap();
    let err = store
        .pre_receive(&git_dir, "demo", "alice", std::slice::from_ref(&update))
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
    assert!(store.pkgbase_id("demo").unwrap().is_none());
    let rejection = store
        .list_audit(admin)
        .unwrap()
        .into_iter()
        .find(|e| e.target_kind == "registry_upload" && e.open)
        .unwrap();
    let accepted = store
        .accept_rejection(admin, rejection.id, "same tree")
        .unwrap();
    let suspended = store.reject_user(admin, "alice", "paused").unwrap();
    let err = store
        .pre_receive(&git_dir, "demo", "alice", std::slice::from_ref(&update))
        .unwrap_err();
    assert!(err.to_string().contains("suspended"), "{err}");
    store.reverse_audit(admin, suspended, "cleared").unwrap();
    store
        .pre_receive(&git_dir, "demo", "alice", std::slice::from_ref(&update))
        .unwrap();
    assert!(
        store.pkgbase_id("demo").unwrap().is_none(),
        "acceptance permits the push check and does not publish the package"
    );
    let moved = root.path().join("moved");
    write_upload(&moved, "vendor", "core", &src, &bad_pkg, Some(&sig));
    let other = git_commit(&moved);
    let err = store
        .pre_receive(&moved.join(".git"), "demo", "alice", &[push_of(&other)])
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
    store
        .revoke_acceptance(admin, accepted, "withdraw")
        .unwrap();
    let err = store
        .pre_receive(&git_dir, "demo", "alice", &[update])
        .unwrap_err();
    assert!(err.to_string().contains("rejected"), "{err}");
}
