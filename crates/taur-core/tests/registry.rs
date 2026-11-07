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
