use taur_core::{Error, SearchBy, Store};

fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur")
}

fn store() -> (tempfile::TempDir, Store) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    (d, s)
}

#[test]
fn moderator_can_reject_and_only_an_admin_can_reverse() {
    let (_d, s) = store();
    let admin = s
        .register("admin", "admin@example.com", "correct horse")
        .unwrap()
        .id;
    let modr = s
        .register("moddy", "mod@example.com", "correct horse")
        .unwrap()
        .id;
    let user = s
        .register("alice", "alice@example.com", "correct horse")
        .unwrap()
        .id;
    s.set_admin(admin, true).unwrap();
    s.mark_verified(admin).unwrap();
    s.mark_verified(modr).unwrap();
    s.mark_verified(user).unwrap();
    s.import_dir(&fixture(), Some("alice")).unwrap();
    assert!(matches!(
        s.reject_package(user, "hello-taur", "rogue"),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        s.set_moderator(modr, "alice", true),
        Err(Error::Forbidden)
    ));
    s.set_moderator(admin, "moddy", true).unwrap();
    assert!(matches!(
        s.reject_user(modr, "admin", "nope"),
        Err(Error::Forbidden)
    ));
    let event = s
        .reject_package(modr, "hello-taur", "not the upstream recipe")
        .unwrap();
    assert!(s.search(SearchBy::Name, "hello-taur").unwrap().is_empty());
    assert!(s.pkgbase_suspended("hello-taur").unwrap());
    assert!(matches!(
        s.reject_package(modr, "hello-taur", "again"),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(s.list_audit(modr), Err(Error::Forbidden)));
    assert!(matches!(
        s.reverse_audit(modr, event, "undo"),
        Err(Error::Forbidden)
    ));
    let rows = s.list_audit(admin).unwrap();
    assert!(
        rows.iter()
            .any(|e| e.id == event && e.open && e.reason == "not the upstream recipe")
    );
    assert!(matches!(
        s.accept_rejection(admin, event, "not an upload"),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        s.accept_rejection(modr, event, "not an upload"),
        Err(Error::Forbidden)
    ));
    assert!(!rows.iter().any(|e| e.reason.contains("correct horse")));
    s.reverse_audit(admin, event, "reviewed").unwrap();
    assert!(!s.pkgbase_suspended("hello-taur").unwrap());
    assert_eq!(s.search(SearchBy::Name, "hello").unwrap().len(), 2);
    assert!(matches!(
        s.reverse_audit(admin, event, "twice"),
        Err(Error::Conflict(_))
    ));

    let suspended = s.reject_user(modr, "alice", "rogue account").unwrap();
    match s.login("alice", "correct horse") {
        Err(err) => assert!(err.to_string().contains("suspended"), "{err}"),
        Ok(_) => panic!("suspended account signed in"),
    }
    s.reverse_audit(admin, suspended, "cleared").unwrap();
    assert!(s.login("alice", "correct horse").is_ok());
}

#[test]
fn request_rejection_is_audited_and_not_reopened() {
    let (_d, s) = store();
    let admin = s
        .register("admin", "admin@example.com", "correct horse")
        .unwrap()
        .id;
    let user = s
        .register("alice", "alice@example.com", "correct horse")
        .unwrap()
        .id;
    s.set_admin(admin, true).unwrap();
    s.mark_verified(admin).unwrap();
    s.mark_verified(user).unwrap();
    s.import_dir(&fixture(), Some("alice")).unwrap();
    let id = s.pkgbase_id("hello-taur").unwrap().unwrap();
    let req = s
        .file_request(user, "hello-taur", "deletion", None, "please")
        .unwrap();
    s.reject_request(admin, req.id, "no").unwrap();
    let rows = s.list_audit(admin).unwrap();
    let event = rows.iter().find(|e| e.target_kind == "request").unwrap();
    assert!(event.target.contains("hello-taur"));
    assert!(matches!(
        s.accept_rejection(admin, event.id, "not a registration"),
        Err(Error::Conflict(_))
    ));
    s.reverse_audit(admin, event.id, "retry").unwrap();
    assert_eq!(s.request(req.id).unwrap().status, "rejected");
    assert!(s.pkgbase_id("hello-taur").unwrap() == Some(id));
    let _ = req;
}
