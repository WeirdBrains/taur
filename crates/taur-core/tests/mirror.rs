//! Recipe mirrors against a loopback git daemon and a temporary home.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use taur_core::{Error, Store};

struct Daemon {
    child: std::process::Child,
    url: String,
    _root: tempfile::TempDir,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn recipe(dir: &Path, version: &str, extra: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(".SRCINFO"),
        format!("pkgbase = demo\npkgver = {version}\npkgrel = 1\n\npkgname = demo\n"),
    )
    .unwrap();
    fs::write(
        dir.join("PKGBUILD"),
        format!("pkgver={version}\npkgrel=1\npkgname=(demo)\n{extra}\n"),
    )
    .unwrap();
}

fn commit_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "master"]);
    git(dir, &["add", "."]);
    git(
        dir,
        &[
            "-c",
            "user.email=mirror@example.com",
            "-c",
            "user.name=mirror",
            "commit",
            "-q",
            "-m",
            "recipe",
        ],
    );
}

fn publish(work: &Path, name: &str, base: &Path) {
    let dest = base.join(format!("{name}.git"));
    let _ = fs::remove_dir_all(&dest);
    let out = Command::new("git")
        .args(["clone", "--bare", "-q"])
        .arg(work)
        .arg(&dest)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn serve(base: &Path) -> Daemon {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let child = Command::new("git")
        .args([
            "daemon",
            "--listen=127.0.0.1",
            &format!("--port={port}"),
            &format!("--base-path={}", base.display()),
            "--export-all",
            "--reuseaddr",
            "--informative-errors",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    Daemon {
        child,
        url: format!("git://127.0.0.1:{port}"),
        _root: tempfile::tempdir().unwrap(),
    }
}

fn head_file(store: &Store, pkg: &str, path: &str) -> String {
    let repo = store.repos_dir().join(format!("{pkg}.git"));
    let out = Command::new("git")
        .arg("--git-dir")
        .arg(&repo)
        .args(["show", &format!("HEAD:{path}")])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn commit_count(store: &Store, pkg: &str) -> usize {
    let repo = store.repos_dir().join(format!("{pkg}.git"));
    let out = Command::new("git")
        .arg("--git-dir")
        .arg(&repo)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[test]
fn a_mirror_copies_only_the_selected_recipe_and_stays_read_only() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("src");
    let marker = root.path().join("pwned");
    recipe(&work, "1.0.0", &format!("touch {}", marker.display()));
    commit_repo(&work);
    let base = root.path().join("pub");
    fs::create_dir_all(&base).unwrap();
    publish(&work, "demo", &base);
    let daemon = serve(&base);
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    store.add_mirror("lab", &daemon.url).unwrap();
    store.select_mirror_package("lab", "demo").unwrap();
    let rows = store.sync_mirror("lab").unwrap();
    let demo = rows.iter().find(|r| r.pkgbase == "demo").unwrap();
    assert_eq!(demo.status, "synced", "{:?}", demo.detail);
    assert!(demo.revision.as_deref().unwrap_or("").len() >= 7);
    assert!(!marker.exists(), "PKGBUILD was executed");
    assert_eq!(
        store.pkgbase_registry("demo").unwrap().as_deref(),
        Some("lab")
    );
    let registries = store.list_registries().unwrap();
    let lab = registries.iter().find(|r| r.name == "lab").unwrap();
    assert_eq!(lab.enforcement, "mirror");
    assert_eq!(lab.provenance, "mirror");
    assert!(head_file(&store, "demo", "TAUR-MIRROR").contains("not an operator signature"));
    assert!(head_file(&store, "demo", "PKGBUILD").contains("pkgver=1.0.0"));
    let again = store.sync_mirror("lab").unwrap();
    assert_eq!(again[0].status, "unchanged");
    assert_eq!(commit_count(&store, "demo"), 1);

    let other = root.path().join("import");
    recipe(&other, "9.0.0", "# local");
    let err = store.import_into(&other, None, Some("lab")).unwrap_err();
    assert!(err.to_string().contains("read-only"), "{err}");
    assert!(head_file(&store, "demo", "PKGBUILD").contains("pkgver=1.0.0"));

    let admin = store
        .register("admin", "admin@example.com", "correct horse")
        .unwrap()
        .id;
    store.set_admin(admin, true).unwrap();
    let rejection = store
        .reject_package(admin, "demo", "not the recipe it claims")
        .unwrap();
    assert!(matches!(
        store.accept_rejection(admin, rejection, "not a mirror bypass"),
        Err(Error::Conflict(_))
    ));
    recipe(&work, "2.0.0", "# next");
    git(&work, &["add", "."]);
    git(
        &work,
        &[
            "-c",
            "user.email=mirror@example.com",
            "-c",
            "user.name=mirror",
            "commit",
            "-q",
            "-m",
            "next",
        ],
    );
    publish(&work, "demo", &base);
    let blocked = store.sync_mirror("lab").unwrap();
    assert_eq!(blocked[0].status, "blocked", "{}", blocked[0].detail);
    assert!(head_file(&store, "demo", "PKGBUILD").contains("pkgver=1.0.0"));
    assert!(store.pkgbase_suspended("demo").unwrap());
    store.reverse_audit(admin, rejection, "cleared").unwrap();
    let updated = store.sync_mirror("lab").unwrap();
    assert_eq!(updated[0].status, "synced", "{}", updated[0].detail);
    assert!(head_file(&store, "demo", "PKGBUILD").contains("pkgver=2.0.0"));
    let _ = daemon;
}

#[test]
fn local_packages_collisions_and_bad_trees_are_not_copied() {
    let root = tempfile::tempdir().unwrap();
    let local = root.path().join("local");
    recipe(&local, "1.0.0", "# ours");
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    store.import_into(&local, None, None).unwrap();

    let work = root.path().join("src");
    recipe(&work, "3.0.0", "# theirs");
    commit_repo(&work);
    let sneaky = root.path().join("sneaky");
    recipe(&sneaky, "1.0.0", "# link");
    commit_repo(&sneaky);
    git(&sneaky, &["rm", "PKGBUILD"]);
    std::os::unix::fs::symlink("elsewhere", sneaky.join("PKGBUILD")).unwrap();
    git(&sneaky, &["add", "PKGBUILD"]);
    git(
        &sneaky,
        &[
            "-c",
            "user.email=mirror@example.com",
            "-c",
            "user.name=mirror",
            "commit",
            "-q",
            "-m",
            "link",
        ],
    );
    let base = root.path().join("pub");
    fs::create_dir_all(&base).unwrap();
    publish(&work, "demo", &base);
    publish(&sneaky, "linkpkg", &base);
    let daemon = serve(&base);
    store.add_mirror("lab", &daemon.url).unwrap();
    store.select_mirror_package("lab", "demo").unwrap();
    store.select_mirror_package("lab", "linkpkg").unwrap();
    store.select_mirror_package("lab", "missing").unwrap();
    let rows = store.sync_mirror("lab").unwrap();
    let demo = rows.iter().find(|r| r.pkgbase == "demo").unwrap();
    assert_eq!(demo.status, "blocked", "{}", demo.detail);
    assert!(head_file(&store, "demo", "PKGBUILD").contains("# ours"));
    assert_eq!(
        store.pkgbase_registry("demo").unwrap().as_deref(),
        Some("local")
    );
    let link = rows.iter().find(|r| r.pkgbase == "linkpkg").unwrap();
    assert_eq!(link.status, "failed", "{}", link.detail);
    assert!(store.pkgbase_id("linkpkg").unwrap().is_none());
    let missing = rows.iter().find(|r| r.pkgbase == "missing").unwrap();
    assert_eq!(missing.status, "failed", "{}", missing.detail);

    fs::remove_dir_all(base.join("demo.git")).unwrap();
    // The local package was never a mirror copy, so a later missing upstream stays blocked.
    let rows = store.sync_mirror("lab").unwrap();
    assert_eq!(
        rows.iter().find(|r| r.pkgbase == "demo").unwrap().status,
        "blocked"
    );
    assert!(head_file(&store, "demo", "PKGBUILD").contains("# ours"));

    store.unselect_mirror_package("lab", "missing").unwrap();
    assert!(store.remove_mirror("lab").is_ok());
    let _ = daemon;
}

#[test]
fn a_gone_upstream_keeps_the_last_copy_until_drop() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("src");
    recipe(&work, "1.0.0", "# kept");
    commit_repo(&work);
    let base = root.path().join("pub");
    fs::create_dir_all(&base).unwrap();
    publish(&work, "demo", &base);
    let daemon = serve(&base);
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    store.add_mirror("lab", &daemon.url).unwrap();
    store.select_mirror_package("lab", "demo").unwrap();
    assert_eq!(store.sync_mirror("lab").unwrap()[0].status, "synced");
    fs::remove_dir_all(base.join("demo.git")).unwrap();
    let gone = &store.sync_mirror("lab").unwrap()[0];
    assert_eq!(gone.status, "upstream-missing", "{}", gone.detail);
    assert!(head_file(&store, "demo", "PKGBUILD").contains("# kept"));
    store.drop_mirrored_package("lab", "demo").unwrap();
    assert!(store.pkgbase_id("demo").unwrap().is_none());
    store.remove_mirror("lab").unwrap();
    assert!(store.list_mirrors().unwrap().is_empty());
    let _ = daemon;
}
