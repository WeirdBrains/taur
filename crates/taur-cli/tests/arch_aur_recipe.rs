//! Public AUR recipe is self-contained and must not depend on a local payload.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn pkgbuild() -> String {
    fs::read_to_string(repo().join("packaging/arch/PKGBUILD")).unwrap()
}

fn srcinfo() -> String {
    fs::read_to_string(repo().join("packaging/arch/.SRCINFO")).unwrap()
}

#[test]
fn public_recipe_points_at_the_github_release_tarball() {
    let pkgbuild = pkgbuild();
    let srcinfo = srcinfo();
    let source_line = pkgbuild
        .lines()
        .find(|l| l.starts_with("source="))
        .expect("source=");
    assert!(
        !source_line.contains("payload.tar.gz"),
        "public PKGBUILD source must not be payload.tar.gz: {source_line}"
    );
    assert!(!srcinfo.contains("payload.tar.gz"), "{srcinfo}");
    assert!(source_line.contains("releases/download/"), "{source_line}");
    assert!(
        !pkgbuild.contains("sha256sums=('SKIP')"),
        "public PKGBUILD must not SKIP the tarball checksum"
    );
    let version = pkgbuild
        .lines()
        .find(|l| l.starts_with("pkgver="))
        .unwrap()
        .split_once('=')
        .unwrap()
        .1;
    let url = format!(
        "https://github.com/WeirdBrains/taur/releases/download/v{version}/taur-{version}-x86_64-unknown-linux-gnu.tar.gz"
    );
    assert!(
        pkgbuild.contains("v${pkgver}/taur-${pkgver}-x86_64-unknown-linux-gnu.tar.gz")
            || pkgbuild.contains(&url),
        "{pkgbuild}"
    );
    assert!(srcinfo.contains(&format!("source = {url}")), "{srcinfo}");
    let sum = srcinfo
        .lines()
        .find(|l| l.contains("sha256sums = "))
        .unwrap();
    let digest = sum.split_once(" = ").unwrap().1;
    assert!(
        digest == "UPSTREAM_TARBALL_SHA256"
            || (digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit())),
        "{sum}"
    );
}

#[test]
fn metadata_check_accepts_the_public_recipe() {
    let version = pkgbuild()
        .lines()
        .find(|l| l.starts_with("pkgver="))
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_string();
    let out = Command::new("python3")
        .args(["scripts/check-package-metadata.py", "check", &version])
        .current_dir(repo())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn metadata_check_rejects_a_payload_source_in_the_public_recipe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for dir in [
        "packaging/arch",
        "packaging/arch-internal",
        "packaging/debian",
        "packaging/rpm",
    ] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("packaging/arch/PKGBUILD"),
        "pkgname=taur\npkgver=1.6.0\npkgrel=1\npkgdesc='test'\narch=('x86_64')\nurl='https://example.com'\nlicense=('MIT')\ndepends=('git')\nsource=(payload.tar.gz)\nsha256sums=('SKIP')\npackage() { :; }\n",
    )
    .unwrap();
    let src = Command::new("makepkg")
        .arg("--printsrcinfo")
        .current_dir(root.join("packaging/arch"))
        .output()
        .unwrap();
    assert!(src.status.success());
    fs::write(root.join("packaging/arch/.SRCINFO"), src.stdout).unwrap();
    fs::write(
        root.join("packaging/arch-internal/PKGBUILD"),
        "pkgname=taur\nsource=(payload.tar.gz)\nsha256sums=('SKIP')\npackage() { :; }\n",
    )
    .unwrap();
    fs::write(
        root.join("packaging/debian/control.in"),
        "Package: taur\nVersion: @VERSION@-1\n",
    )
    .unwrap();
    fs::write(
        root.join("packaging/rpm/taur.spec"),
        "Name: taur\nVersion:        @VERSION@\nRelease:        1\n%changelog\n* date - @VERSION@-1\n",
    )
    .unwrap();
    let out = Command::new("python3")
        .args([
            "scripts/check-package-metadata.py",
            "check",
            "1.6.0",
            "--root",
        ])
        .arg(root)
        .current_dir(repo())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("payload.tar.gz") || err.contains("GitHub release tarball"),
        "{err}"
    );
}

#[test]
fn package_sh_builds_official_arch_packages_from_the_internal_recipe() {
    let script = fs::read_to_string(repo().join("scripts/package.sh")).unwrap();
    assert!(script.contains("packaging/arch-internal/PKGBUILD"));
    assert!(script.contains("set-tarball-sha256"));
    assert!(
        script.contains("taur-${version}-arch-aur.tar.gz") || script.contains("arch-aur.tar.gz")
    );
    let smoke = fs::read_to_string(repo().join("scripts/arch-aur-smoke.sh")).unwrap();
    assert!(smoke.contains("PKGBUILD"));
    assert!(smoke.contains(".SRCINFO"));
    assert!(smoke.contains("taur.install"));
    assert!(
        !smoke.contains("payload.tar.gz")
            || smoke.contains("must not")
            || smoke.contains("only files")
    );
    assert!(smoke.contains("/usr/bin/taur") && smoke.contains("/usr/bin/taurctl"));
    assert!(smoke.contains("command -v") && smoke.contains("--version"));
}

#[test]
fn internal_recipe_is_the_local_payload_wrapper() {
    let internal = fs::read_to_string(repo().join("packaging/arch-internal/PKGBUILD")).unwrap();
    assert!(internal.contains("source=(payload.tar.gz)"));
    assert!(internal.contains("scripts/package.sh"));
}
