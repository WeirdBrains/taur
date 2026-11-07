//! The release script against temporary git repositories. Never the live origin.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fix {
    _bare: tempfile::TempDir,
    work: tempfile::TempDir,
    log: tempfile::NamedTempFile,
    stub: tempfile::TempDir,
}

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/release.sh")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_repo(dir: &Path, version: &str) {
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[workspace]\nresolver = \"3\"\nmembers = []\n\n[workspace.package]\nversion = \"{version}\"\nedition = \"2024\"\n"
        ),
    )
    .unwrap();
    fs::write(
        dir.join("Cargo.lock"),
        format!(
            "[[package]]\nname = \"taur-cli\"\nversion = \"{version}\"\n\n[[package]]\nname = \"taur-core\"\nversion = \"{version}\"\n\n[[package]]\nname = \"taur-server\"\nversion = \"{version}\"\n"
        ),
    )
    .unwrap();
    fs::write(
        dir.join("CHANGELOG.md"),
        "# Changelog\n\n## [Unreleased]\n### Added\n- Pending.\n\n## [0.1.0] - 2026-01-01\n### Added\n- Start.\n",
    )
    .unwrap();
}

fn fixture(version: &str) -> Fix {
    let bare = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    git(bare.path(), &["init", "--bare", "-b", "main"]);
    git(work.path(), &["clone", bare.path().to_str().unwrap(), "."]);
    write_repo(work.path(), version);
    git(
        work.path(),
        &["config", "user.email", "release-test@example.com"],
    );
    git(work.path(), &["config", "user.name", "Release Test"]);
    git(
        work.path(),
        &["add", "Cargo.toml", "Cargo.lock", "CHANGELOG.md"],
    );
    git(work.path(), &["commit", "-m", "start"]);
    git(work.path(), &["push", "-u", "origin", "main"]);
    let log = tempfile::NamedTempFile::new().unwrap();
    let stub = tempfile::tempdir().unwrap();
    let cargo = stub.path().join("cargo");
    fs::write(
        &cargo,
        "#!/bin/sh\necho \"$(git -C \"$RELEASE_WORK\" rev-parse HEAD) $*\" >> \"$CARGO_LOG\"\nexit 0\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&cargo).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&cargo, perms).unwrap();
    Fix {
        _bare: bare,
        work,
        log,
        stub,
    }
}

fn run(fix: &Fix, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("bash");
    cmd.arg(script())
        .args(args)
        .current_dir(fix.work.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                fix.stub.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("CARGO_LOG", fix.log.path())
        .env("RELEASE_WORK", fix.work.path())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    cmd.output().unwrap()
}

fn ok(fix: &Fix, args: &[&str]) -> String {
    let out = run(fix, args);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{args:?}\n{text}");
    text
}

fn fail(fix: &Fix, args: &[&str]) -> String {
    let out = run(fix, args);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "{args:?} should fail\n{text}");
    text
}

fn version_of(dir: &Path) -> String {
    fs::read_to_string(dir.join("Cargo.toml"))
        .unwrap()
        .lines()
        .find(|l| l.starts_with("version = "))
        .unwrap()
        .split('"')
        .nth(1)
        .unwrap()
        .to_string()
}

fn remote_has_tag(bare: &Path, tag: &str) -> bool {
    let out = Command::new("git")
        .args(["--git-dir", bare.to_str().unwrap(), "show-ref", "--tags"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&format!("refs/tags/{tag}"))
}

fn checks_ran_before_commit(fix: &Fix, before: &str) {
    let log = fs::read_to_string(fix.log.path()).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert!(lines.len() >= 4, "{log}");
    for line in &lines {
        assert!(line.starts_with(before), "cargo ran after commit: {line}");
    }
    assert!(lines.iter().any(|l| l.contains("fmt")));
    assert!(lines.iter().any(|l| l.contains("clippy")));
    assert!(lines.iter().any(|l| l.contains("test --offline")));
    assert!(
        lines
            .iter()
            .any(|l| l.contains("build --release --offline"))
    );
}

#[test]
fn default_minor_bump_commits_tags_and_pushes_only_that_tag() {
    let fix = fixture("1.2.0");
    let before = git(fix.work.path(), &["rev-parse", "HEAD"]);
    git(fix.work.path(), &["tag", "-a", "v0.0.9", "-m", "unrelated"]);
    let text = ok(&fix, &[]);
    assert!(text.contains("pushed v1.3.0"), "{text}");
    assert_eq!(version_of(fix.work.path()), "1.3.0");
    assert!(
        fs::read_to_string(fix.work.path().join("Cargo.lock"))
            .unwrap()
            .contains("version = \"1.3.0\"")
    );
    let changelog = fs::read_to_string(fix.work.path().join("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("## [1.3.0] - "), "{changelog}");
    assert!(!changelog.contains("[Unreleased]"));
    assert_eq!(
        git(fix.work.path(), &["log", "-1", "--format=%s"]),
        "release: v1.3.0"
    );
    assert_eq!(
        git(fix.work.path(), &["rev-parse", "v1.3.0^{}"]),
        git(fix.work.path(), &["rev-parse", "HEAD"])
    );
    assert_eq!(
        git(fix.work.path(), &["rev-parse", "HEAD"]),
        git(fix.work.path(), &["rev-parse", "origin/main"])
    );
    assert!(remote_has_tag(fix._bare.path(), "v1.3.0"));
    assert!(!remote_has_tag(fix._bare.path(), "v0.0.9"));
    checks_ran_before_commit(&fix, &before);
}

#[test]
fn patch_major_and_explicit_versions_follow_the_reset_rules() {
    let patch = fixture("1.2.3");
    ok(&patch, &["--patch"]);
    assert_eq!(version_of(patch.work.path()), "1.2.4");
    assert!(remote_has_tag(patch._bare.path(), "v1.2.4"));

    let major = fixture("1.2.3");
    ok(&major, &["--major"]);
    assert_eq!(version_of(major.work.path()), "2.0.0");
    assert!(remote_has_tag(major._bare.path(), "v2.0.0"));

    let explicit = fixture("0.9.1");
    ok(&explicit, &["--version", "1.0.0"]);
    assert_eq!(version_of(explicit.work.path()), "1.0.0");
    assert!(remote_has_tag(explicit._bare.path(), "v1.0.0"));
}

#[test]
fn dry_run_prints_the_plan_and_changes_nothing() {
    let fix = fixture("1.2.0");
    let before = git(fix.work.path(), &["rev-parse", "HEAD"]);
    let text = ok(&fix, &["--dry-run", "--patch"]);
    assert!(text.contains("new:      1.2.1"), "{text}");
    assert!(text.contains("tag:      v1.2.1"), "{text}");
    assert!(text.contains("dry-run"), "{text}");
    assert!(text.contains("does not create a GitHub release"), "{text}");
    assert_eq!(git(fix.work.path(), &["rev-parse", "HEAD"]), before);
    assert_eq!(version_of(fix.work.path()), "1.2.0");
    assert!(git(fix.work.path(), &["status", "--porcelain"]).is_empty());
    assert!(!remote_has_tag(fix._bare.path(), "v1.2.1"));
    assert!(fs::read_to_string(fix.log.path()).unwrap().is_empty());
}

#[test]
fn invalid_and_conflicting_versions_are_rejected() {
    let cases = [
        vec!["--major", "--patch"],
        vec!["--version", "1.3.0", "--patch"],
        vec!["--major", "--version", "2.0.0"],
        vec!["--version", "1.2"],
        vec!["--version", "01.2.0"],
        vec!["--version", "1.02.0"],
        vec!["--version", "1.2.03"],
        vec!["--version", "1.2.3-rc.1"],
        vec!["--version", "1.2.3+build.1"],
        vec!["--version", "v1.3.0"],
        vec!["--version", "1.2.0"],
        vec!["--version", "1.1.9"],
        vec!["--nope"],
    ];
    for args in cases {
        let fix = fixture("1.2.0");
        let err = fail(&fix, &args);
        assert!(
            err.contains("release:"),
            "{args:?} stderr did not explain the rejection:\n{err}"
        );
        assert_eq!(version_of(fix.work.path()), "1.2.0", "{args:?}");
        assert!(git(fix.work.path(), &["status", "--porcelain"]).is_empty());
        assert!(!remote_has_tag(fix._bare.path(), "v1.3.0"));
    }
}

#[test]
fn dirty_tree_wrong_branch_and_divergence_are_refused() {
    let dirty = fixture("1.2.0");
    fs::write(dirty.work.path().join("notes.txt"), "x").unwrap();
    assert!(fail(&dirty, &[]).contains("not clean"));

    let staged = fixture("1.2.0");
    let mut toml = fs::read_to_string(staged.work.path().join("Cargo.toml")).unwrap();
    toml.push_str("\n# staged\n");
    fs::write(staged.work.path().join("Cargo.toml"), toml).unwrap();
    git(staged.work.path(), &["add", "Cargo.toml"]);
    assert!(
        fail(&staged, &["--patch"]).contains("not clean")
            || fail(&staged, &["--patch"]).contains("staged")
    );

    let branch = fixture("1.2.0");
    git(branch.work.path(), &["checkout", "-b", "dev"]);
    assert!(fail(&branch, &[]).contains("branch"));

    let ahead = fixture("1.2.0");
    fs::write(ahead.work.path().join("extra.txt"), "x").unwrap();
    git(ahead.work.path(), &["add", "extra.txt"]);
    git(ahead.work.path(), &["commit", "-m", "local only"]);
    assert!(fail(&ahead, &[]).contains("not up to date"));

    let behind = fixture("1.2.0");
    let other = tempfile::tempdir().unwrap();
    git(
        other.path(),
        &[
            "clone",
            behind._bare.path().to_str().unwrap(),
            other.path().to_str().unwrap(),
        ],
    );
    git(
        other.path(),
        &["config", "user.email", "release-test@example.com"],
    );
    git(other.path(), &["config", "user.name", "Release Test"]);
    fs::write(other.path().join("other.txt"), "x").unwrap();
    git(other.path(), &["add", "other.txt"]);
    git(other.path(), &["commit", "-m", "elsewhere"]);
    git(other.path(), &["push", "origin", "main"]);
    assert!(fail(&behind, &[]).contains("not up to date"));
}

#[test]
fn existing_local_or_remote_tag_is_not_overwritten() {
    let local = fixture("1.2.0");
    git(local.work.path(), &["tag", "-a", "v1.3.0", "-m", "taken"]);
    let err = fail(&local, &[]);
    assert!(
        err.contains("local tag") || err.contains("already exists"),
        "{err}"
    );
    assert_eq!(
        git(local.work.path(), &["rev-parse", "v1.3.0^{}"]),
        git(local.work.path(), &["rev-parse", "HEAD"])
    );

    let remote = fixture("1.2.0");
    git(remote.work.path(), &["tag", "-a", "v1.3.0", "-m", "taken"]);
    git(remote.work.path(), &["push", "origin", "v1.3.0"]);
    git(remote.work.path(), &["tag", "-d", "v1.3.0"]);
    let err = fail(&remote, &[]);
    assert!(err.contains("origin already has"), "{err}");
    assert!(remote_has_tag(remote._bare.path(), "v1.3.0"));
    assert_eq!(version_of(remote.work.path()), "1.2.0");
}

#[test]
fn a_rejected_push_does_not_publish_the_branch_or_the_tag() {
    let fix = fixture("1.2.0");
    let before = git(fix.work.path(), &["rev-parse", "HEAD"]);
    let hook = fix._bare.path().join("hooks/update");
    fs::write(
        &hook,
        "#!/bin/sh\ncase \"$1\" in refs/tags/*) exit 1 ;; esac\nexit 0\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&hook).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&hook, perms).unwrap();
    let err = fail(&fix, &["--patch"]);
    assert!(
        err.contains("atomic") || err.contains("rejected") || err.contains("hook"),
        "{err}"
    );
    assert_eq!(
        git(fix._bare.path(), &["rev-parse", "main"]),
        before,
        "origin/main moved despite a rejected tag"
    );
    assert!(!remote_has_tag(fix._bare.path(), "v1.2.1"));
}
