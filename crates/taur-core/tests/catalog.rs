//! Public package index, organization directory, and recorded git revisions.

use std::fs;
use std::path::{Path, PathBuf};
use taur_core::{PACKAGE_INDEX_PER_PAGE, PackageSort, Store};

fn store() -> (tempfile::TempDir, Store) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    (d, s)
}

fn write_pkg(dir: &Path, name: &str, ver: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("PKGBUILD"),
        format!("pkgbase={name}\npkgname={name}\npkgver={ver}\npkgrel=1\narch=(any)\n"),
    )
    .unwrap();
    fs::write(
        dir.join(".SRCINFO"),
        format!(
            "pkgbase = {name}\n\tpkgdesc = {name} package\n\tpkgver = {ver}\n\tpkgrel = 1\n\tarch = any\n\npkgname = {name}\n"
        ),
    )
    .unwrap();
}

fn import_pkg(s: &Store, root: &Path, name: &str, ver: &str, maintainer: Option<&str>) {
    let dir = root.join(name);
    write_pkg(&dir, name, ver);
    s.import_dir(&dir, maintainer).unwrap();
}

fn set_modified(s: &Store, name: &str, ts: i64) {
    s.set_last_modified(name, ts).unwrap();
}

#[test]
fn package_index_is_paginated_and_sorts_with_stable_ties() {
    let (d, s) = store();
    let staging = d.path().join("src");
    // 25 packages, names a01..a25 so page 2 has a leftover of 5.
    for i in 1..=25 {
        let name = format!("pkg{i:02}");
        import_pkg(&s, &staging, &name, "1.0.0", None);
        set_modified(&s, &name, 1_700_000_000 + i);
    }
    // Identical update time for pkg01 and pkg02: name ASC must not skip or duplicate.
    set_modified(&s, "pkg01", 1_800_000_000);
    set_modified(&s, "pkg02", 1_800_000_000);

    let updated = s.package_index(PackageSort::Updated, 1).unwrap();
    assert_eq!(updated.per_page, PACKAGE_INDEX_PER_PAGE);
    assert_eq!(updated.total, 25);
    assert_eq!(updated.page, 1);
    assert_eq!(updated.packages.len(), 20);
    assert_eq!(updated.packages[0].name, "pkg01");
    assert_eq!(updated.packages[1].name, "pkg02");
    let names: Vec<_> = updated.packages.iter().map(|p| p.name.as_str()).collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "duplicates on page 1: {names:?}");

    let page2 = s.package_index(PackageSort::Updated, 2).unwrap();
    assert_eq!(page2.packages.len(), 5);
    let all: Vec<_> = names
        .into_iter()
        .chain(page2.packages.iter().map(|p| p.name.as_str()))
        .collect();
    let mut uniq = all.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 25, "skip or duplicate across pages: {all:?}");

    let empty = s.package_index(PackageSort::Updated, 99).unwrap();
    assert!(empty.packages.is_empty());
    assert_eq!(empty.total, 25);
    let clamped = s.package_index(PackageSort::Updated, 0).unwrap();
    assert_eq!(clamped.page, 1);
    assert_eq!(clamped.packages[0].name, "pkg01");

    assert_eq!(PackageSort::parse(Some("downloaded")), PackageSort::Updated);
    assert_eq!(PackageSort::parse(Some("nope")), PackageSort::Updated);
    assert_eq!(
        PackageSort::parse(Some("popularity")),
        PackageSort::Popularity
    );

    let alice = s.register("alice", "a@x", "correct horse").unwrap().id;
    let bob = s.register("bob", "b@x", "correct horse").unwrap().id;
    s.vote(alice, "pkg25").unwrap();
    s.vote(bob, "pkg25").unwrap();
    s.vote(alice, "pkg24").unwrap();
    let pop = s.package_index(PackageSort::Popularity, 1).unwrap();
    assert_eq!(pop.packages[0].name, "pkg25");
    assert_eq!(pop.packages[1].name, "pkg24");
    assert!(pop.packages[0].popularity > pop.packages[1].popularity);
}

#[test]
fn public_orgs_scope_packages_and_filter_names() {
    let (d, s) = store();
    let staging = d.path().join("src");
    let alice = s.register("alice", "a@x", "correct horse").unwrap().id;
    let org = s.create_org(alice, "WeirdBrains").unwrap();
    let other = s.create_org(alice, "other-org").unwrap();
    import_pkg(&s, &staging, "org-pkg", "1.0.0", Some("alice"));
    import_pkg(&s, &staging, "other-pkg", "1.0.0", Some("alice"));
    import_pkg(&s, &staging, "personal", "1.0.0", Some("alice"));
    let org_pb = s.pkgbase_id("org-pkg").unwrap().unwrap();
    let other_pb = s.pkgbase_id("other-pkg").unwrap().unwrap();
    s.set_pkgbase_org(alice, org_pb, Some(org)).unwrap();
    s.set_pkgbase_org(alice, other_pb, Some(other)).unwrap();

    let dir = s.public_orgs(None).unwrap();
    let names: Vec<_> = dir.iter().map(|o| o.name.as_str()).collect();
    assert!(names.contains(&"WeirdBrains"), "{names:?}");
    assert!(names.contains(&"other-org"), "{names:?}");
    let wb = dir.iter().find(|o| o.name == "WeirdBrains").unwrap();
    assert_eq!(wb.package_count, 1);

    let filtered = s.public_orgs(Some("weirdbrains")).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "WeirdBrains");
    assert!(s.public_orgs(Some("no-such")).unwrap().is_empty());

    let profile = s.public_org("weirdbrains").unwrap();
    assert_eq!(profile.name, "WeirdBrains");
    let pkgs: Vec<_> = profile.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(pkgs, vec!["org-pkg"]);
    assert!(!pkgs.contains(&"other-pkg"));
    assert!(!pkgs.contains(&"personal"));

    match s.public_org("missing-org") {
        Err(taur_core::Error::NotFound(_)) => {}
        other => panic!("expected not found, got {other:?}"),
    }
}

#[test]
fn package_revisions_are_immutable_and_distinguish_recipe_revisions() {
    let (d, s) = store();
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur");
    s.import_dir(&fx, None).unwrap();
    let first = s.package_revisions("hello-taur").unwrap();
    assert_eq!(first.len(), 1);
    assert!(first[0].current);
    assert_eq!(first[0].version.as_deref(), Some("1:1.2.3-1"));
    let head_snap = s.snapshot_at("hello-taur", &first[0].id).unwrap();

    let next = d.path().join("v2");
    dircopy(&fx, &next);
    replace_version(&next, "1.2.3", "2.0.0");
    s.import_dir(&next, None).unwrap();

    let revs = s.package_revisions("hello-taur").unwrap();
    assert_eq!(revs.len(), 2);
    assert!(revs[0].current);
    assert_eq!(revs[0].version.as_deref(), Some("1:2.0.0-1"));
    assert!(!revs[1].current);
    assert_eq!(revs[1].version.as_deref(), Some("1:1.2.3-1"));
    assert_ne!(revs[0].id, revs[1].id);
    let old = s.package_revision("hello-taur", &revs[1].id).unwrap();
    assert_eq!(old.version.as_deref(), Some("1:1.2.3-1"));
    let old_snap = s.snapshot_at("hello-taur", &revs[1].id).unwrap();
    let new_snap = s.snapshot_at("hello-taur", &revs[0].id).unwrap();
    assert_ne!(old_snap, new_snap);
    assert_eq!(old_snap, head_snap);

    // Same pkgver, later recipe revision (pkgrel bump) is a distinct recorded commit.
    let third = d.path().join("v2b");
    dircopy(&next, &third);
    replace_rel(&third, "1", "2");
    s.import_dir(&third, None).unwrap();
    let revs = s.package_revisions("hello-taur").unwrap();
    assert_eq!(revs.len(), 3);
    assert_eq!(revs[0].pkgver.as_deref(), revs[1].pkgver.as_deref());
    assert_ne!(revs[0].id, revs[1].id);
    assert_ne!(revs[0].version.as_deref(), revs[1].version.as_deref());

    match s.package_revision("hello-taur", "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef") {
        Err(taur_core::Error::NotFound(_)) => {}
        other => panic!("expected not found, got {other:?}"),
    }
    match s.package_revision("hello-taur", "not..valid") {
        Err(taur_core::Error::Invalid(_)) => {}
        other => panic!("expected invalid, got {other:?}"),
    }
}

fn dircopy(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for ent in fs::read_dir(src).unwrap() {
        let ent = ent.unwrap();
        let to = dest.join(ent.file_name());
        if ent.file_type().unwrap().is_dir() {
            dircopy(&ent.path(), &to);
        } else {
            fs::copy(ent.path(), to).unwrap();
        }
    }
}

fn replace_version(dir: &Path, old: &str, new: &str) {
    for name in [".SRCINFO", "PKGBUILD"] {
        let path = dir.join(name);
        let t = fs::read_to_string(&path).unwrap().replace(old, new);
        fs::write(path, t).unwrap();
    }
}

fn replace_rel(dir: &Path, old: &str, new: &str) {
    for name in [".SRCINFO", "PKGBUILD"] {
        let path = dir.join(name);
        let t = fs::read_to_string(&path)
            .unwrap()
            .replace(&format!("pkgrel = {old}"), &format!("pkgrel = {new}"))
            .replace(&format!("pkgrel={old}"), &format!("pkgrel={new}"));
        fs::write(path, t).unwrap();
    }
}
