use std::path::PathBuf;
use taur_core::{Error, Role, SearchBy, Store, rpc};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur")
}

fn store() -> (tempfile::TempDir, Store) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    (d, s)
}

#[test]
fn import_and_query() {
    let (_d, s) = store();
    s.register("alice", "a@example.com", "correct horse")
        .unwrap();
    let info = s.import_dir(&fixture(), Some("alice")).unwrap();
    assert_eq!(info.pkgbase, "hello-taur");
    let r = s.info(&["hello-taur".into(), "missing".into()]).unwrap();
    assert_eq!(r.len(), 1);
    let p = &r[0];
    assert_eq!(p.version, "1:1.2.3-1");
    assert_eq!(p.maintainer.as_deref(), Some("alice"));
    assert_eq!(p.relations["depends"], vec!["glibc", "lib32-glibc"]);
    assert_eq!(p.relations["license"], vec!["MIT"]);
    assert_eq!(s.search(SearchBy::NameDesc, "tiny").unwrap().len(), 1);
    assert_eq!(s.search(SearchBy::Name, "hello").unwrap().len(), 2);
    assert_eq!(s.search(SearchBy::Depends, "glibc").unwrap().len(), 1);
    assert_eq!(s.search(SearchBy::MakeDepends, "rust").unwrap().len(), 2);
    assert_eq!(s.search(SearchBy::OptDepends, "bash").unwrap().len(), 2);
    assert_eq!(s.search(SearchBy::Maintainer, "alice").unwrap().len(), 2);
    assert_eq!(s.search(SearchBy::Name, "%").unwrap().len(), 0);
    s.set_keywords("hello-taur", &["demo".into()]).unwrap();
    assert_eq!(s.search(SearchBy::Keywords, "demo").unwrap().len(), 2);
    assert_eq!(s.suggest("hello-taur-").unwrap(), vec!["hello-taur-docs"]);
    assert_eq!(s.suggest_pkgbase("hel").unwrap(), vec!["hello-taur"]);
    // re-import is idempotent
    s.import_dir(&fixture(), None).unwrap();
    assert_eq!(s.all_packages().unwrap().len(), 2);
    let tgz = s.snapshot("hello-taur").unwrap();
    assert_eq!(&tgz[..2], &[0x1f, 0x8b]);
    assert!(s.git_file("/hello-taur.git/info/refs").is_some());
    assert!(s.git_file("/hello-taur.git/HEAD").is_some());
}

#[test]
fn rpc_shapes() {
    let (_d, s) = store();
    s.import_dir(&fixture(), None).unwrap();
    let v = rpc::handle(&s, Some("info"), &["hello-taur".into()], None);
    assert_eq!(v["type"], "multiinfo");
    assert_eq!(v["version"], 5);
    assert_eq!(v["resultcount"], 1);
    let p = &v["results"][0];
    assert_eq!(p["Maintainer"], serde_json::Value::Null);
    assert_eq!(p["URLPath"], "/cgit/aur.git/snapshot/hello-taur.tar.gz");
    assert_eq!(p["MakeDepends"][0], "rust>=1.80");
    assert!(p.get("Conflicts").is_none());
    assert_eq!(p["Keywords"], serde_json::json!([]));
    let v = rpc::handle(&s, Some("search"), &["h".into()], None);
    assert_eq!(v["error"], "Query arg too small.");
    let v = rpc::handle(&s, Some("search"), &["hello".into()], Some("bogus"));
    assert_eq!(v["error"], "Incorrect by field specified.");
    let v = rpc::handle(&s, Some("search"), &["hello".into()], Some("name"));
    assert_eq!(v["resultcount"], 2);
    assert!(v["results"][0].get("Depends").is_none());
    let v = rpc::handle(&s, Some("msearch"), &["".into()], None);
    assert_eq!(v["resultcount"], 2);
    assert_eq!(rpc::handle(&s, Some("nope"), &[], None)["type"], "error");
    assert_eq!(rpc::handle(&s, None, &[], None)["type"], "error");
    assert_eq!(
        rpc::handle(&s, Some("suggest"), &["he".into()], None)[0],
        "hello-taur"
    );
}

#[test]
fn accounts_lifecycle() {
    let (_d, s) = store();
    assert!(matches!(
        s.register("ab", "a@x", "longenough"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        s.register("alice", "nope", "longenough"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        s.register("alice", "a@x", "short"),
        Err(Error::Invalid(_))
    ));
    let u = s.register("alice", "a@x", "longenough").unwrap();
    assert!(matches!(
        s.register("Alice", "b@x", "longenough"),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        s.login("alice", "wrong-pass"),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        s.login("bob", "longenough"),
        Err(Error::Unauthorized)
    ));
    let sess = s.login("alice", "longenough").unwrap();
    assert_eq!(sess.token.len(), 64);
    assert_eq!(s.user_for_token(&sess.token).unwrap(), u);
    s.logout(&sess.token).unwrap();
    assert!(matches!(
        s.user_for_token(&sess.token),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(s.logout(&sess.token), Err(Error::Unauthorized)));

    s.import_dir(&fixture(), Some("alice")).unwrap();
    let sess = s.login("alice", "longenough").unwrap();
    assert!(matches!(
        s.unregister(&sess.token, "bad"),
        Err(Error::Unauthorized)
    ));
    s.unregister(&sess.token, "longenough").unwrap();
    assert!(s.user_by_name("alice").unwrap().is_none());
    assert!(matches!(
        s.user_for_token(&sess.token),
        Err(Error::Unauthorized)
    ));
    let p = &s.info(&["hello-taur".into()]).unwrap()[0];
    assert_eq!(p.maintainer, None, "packages are orphaned");
    // name is free again
    s.register("alice", "a@x", "longenough").unwrap();
}

#[test]
fn orgs_and_roles() {
    let (_d, s) = store();
    let owner = s.register("owner", "o@x", "longenough").unwrap().id;
    let admin = s.register("admin", "ad@x", "longenough").unwrap().id;
    let contrib = s.register("contrib", "c@x", "longenough").unwrap().id;
    let outsider = s.register("outsider", "x@x", "longenough").unwrap().id;
    let org = s.create_org(owner, "weirdbrains").unwrap();
    assert_eq!(s.role_in(org, owner).unwrap(), Some(Role::Owner));
    s.set_member(owner, org, admin, Role::Admin).unwrap();
    s.set_member(admin, org, contrib, Role::Contributor)
        .unwrap();
    assert!(matches!(
        s.set_member(admin, org, outsider, Role::Owner),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        s.set_member(admin, org, owner, Role::Admin),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        s.set_member(contrib, org, outsider, Role::Contributor),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        s.set_member(owner, org, owner, Role::Admin),
        Err(Error::Conflict(_))
    ));

    let pb = s
        .import_srcinfo(
            &taur_core::srcinfo::parse(include_str!("../../../tests/fixtures/hello-taur/.SRCINFO"))
                .unwrap(),
            Some("owner"),
        )
        .unwrap();
    assert!(!s.can_manage_pkgbase(contrib, pb).unwrap());
    s.set_pkgbase_org(owner, pb, Some(org)).unwrap();
    assert!(s.can_manage_pkgbase(contrib, pb).unwrap());
    assert!(s.can_manage_pkgbase(admin, pb).unwrap());
    assert!(!s.can_manage_pkgbase(outsider, pb).unwrap());
    s.remove_member(admin, org, contrib).unwrap();
    assert!(!s.can_manage_pkgbase(contrib, pb).unwrap());

    // sole owner cannot unregister
    let t = s.login("owner", "longenough").unwrap().token;
    assert!(matches!(
        s.unregister(&t, "longenough"),
        Err(Error::Conflict(_))
    ));
    s.set_member(owner, org, admin, Role::Owner).unwrap();
    s.unregister(&t, "longenough").unwrap();
}

#[test]
fn org_and_package_management() {
    let (_d, s) = store();
    let a = s.register("alice", "a@x", "longenough").unwrap().id;
    let b = s.register("bob", "b@x", "longenough").unwrap().id;
    let org = s.create_org(a, "brains").unwrap();
    assert_eq!(s.org_id("brains").unwrap(), org);
    assert!(matches!(s.org_detail(b, "brains"), Err(Error::Forbidden)));
    s.set_member(a, org, b, Role::Contributor).unwrap();
    let d = s.org_detail(b, "brains").unwrap();
    assert_eq!(d.members.len(), 2);
    assert_eq!(s.orgs_for(b).unwrap()[0].role, Role::Contributor);
    let pb = s
        .import_dir(&fixture(), Some("alice"))
        .map(|_| s.pkgbase_id("hello-taur").unwrap().unwrap())
        .unwrap();
    assert_eq!(s.packages_for(a).unwrap().len(), 1);
    assert!(s.packages_for(b).unwrap().is_empty());
    assert!(matches!(s.disown(b, pb), Err(Error::Forbidden)));
    s.set_pkgbase_org(a, pb, Some(org)).unwrap();
    assert_eq!(s.packages_for(b).unwrap()[0].org.as_deref(), Some("brains"));
    s.disown(b, pb).unwrap();
    assert!(matches!(
        s.adopt(s.register("carol", "c@x", "longenough").unwrap().id, pb),
        Err(Error::Forbidden)
    ));
    s.adopt(b, pb).unwrap();
    assert!(matches!(s.adopt(a, pb), Err(Error::Conflict(_))));
    s.transfer_to_user(b, pb, a).unwrap();
    let p = &s.packages_for(a).unwrap()[0];
    assert_eq!(
        (p.maintainer.as_deref(), p.org.as_deref()),
        (Some("alice"), None)
    );
    assert!(matches!(s.delete_org(b, org), Err(Error::Forbidden)));
    s.delete_org(a, org).unwrap();
    assert!(s.orgs_for(b).unwrap().is_empty());
}

#[test]
fn email_verification_and_reset() {
    use taur_core::mail::FileMailer;
    let (d, s) = store();
    let outbox = d.path().join("outbox");
    let u = s.register("erin", "e@x", "longenough").unwrap();
    assert!(!u.verified);
    assert!(matches!(s.require_verified(&u), Err(Error::Unverified)));
    let t = FileMailer::find_token(&outbox, "e@x", "verify").unwrap();
    assert!(matches!(s.verify_email("deadbeef"), Err(Error::Invalid(_))));
    let v = s.verify_email(&t).unwrap();
    assert!(v.verified);
    assert!(s.verify_email(&t).is_err(), "single use");
    assert!(matches!(s.resend_verification(&v), Err(Error::Conflict(_))));
    let sess = s.login("erin", "longenough").unwrap();
    s.request_password_reset("nobody@x").unwrap();
    s.request_password_reset("e@x").unwrap();
    let rt = FileMailer::find_token(&outbox, "e@x", "reset").unwrap();
    assert!(matches!(
        s.reset_password(&rt, "short"),
        Err(Error::Invalid(_))
    ));
    s.reset_password(&rt, "another password").unwrap();
    assert!(s.reset_password(&rt, "another password").is_err());
    assert!(s.user_for_token(&sess.token).is_err(), "sessions revoked");
    assert!(s.login("erin", "longenough").is_err());
    s.login("erin", "another password").unwrap();
    // outbox files are private and never contain the password
    for f in std::fs::read_dir(&outbox).unwrap().flatten() {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(f.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert!(
            !std::fs::read_to_string(f.path())
                .unwrap()
                .contains("another password")
        );
    }
}
