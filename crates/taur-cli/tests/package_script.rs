//! `scripts/package.sh` stages a package tree without compiling or tagging.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/package.sh")
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new("bash")
        .arg(script())
        .args(args)
        .current_dir(repo())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap()
}

#[test]
fn help_and_missing_version() {
    let help = run(&["--help"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    assert!(help.status.success(), "{text}");
    assert!(text.contains("glibc 2.31"), "{text}");
    assert!(text.contains("does not enable"), "{text}");

    let missing = run(&[]);
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&missing.stdout),
        String::from_utf8_lossy(&missing.stderr)
    );
    assert!(!missing.status.success(), "{err}");
    assert!(err.contains("VERSION is required"), "{err}");
}

#[test]
fn stage_only_uses_the_tag_tree_and_checkout_packaging() {
    let out = tempfile::tempdir().unwrap();
    let bins = tempfile::tempdir().unwrap();
    for name in ["taur", "taurctl"] {
        let path = bins.path().join(name);
        fs::write(&path, "#!/bin/sh\necho test-binary\n").unwrap();
        let mut mode = fs::metadata(&path).unwrap().permissions();
        mode.set_mode(0o755);
        fs::set_permissions(&path, mode).unwrap();
    }
    let result = run(&[
        "--stage-only",
        "--bin-dir",
        bins.path().to_str().unwrap(),
        "--output",
        out.path().to_str().unwrap(),
        "1.3.1",
    ]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.status.success(), "{text}");
    let stage = out.path().join("stage");
    assert!(stage.join("usr/bin/taur").is_file());
    assert!(stage.join("usr/bin/taurctl").is_file());
    fn no_srcinfo(dir: &std::path::Path) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                no_srcinfo(&path);
            } else {
                assert_ne!(
                    path.file_name().and_then(|n| n.to_str()),
                    Some(".SRCINFO"),
                    ".SRCINFO must not be installed as a runtime file: {}",
                    path.display()
                );
            }
        }
    }
    no_srcinfo(&stage);
    assert!(stage.join("usr/lib/systemd/system/taur.service").is_file());
    assert!(stage.join("usr/lib/sysusers.d/taur.conf").is_file());
    assert!(stage.join("usr/lib/tmpfiles.d/taur.conf").is_file());
    assert!(stage.join("usr/lib/taur/ensure-account").is_file());
    assert!(stage.join("usr/share/man/man1/taur.1.gz").is_file());
    assert!(stage.join("usr/share/man/man1/taurctl.1.gz").is_file());
    assert!(stage.join("usr/share/doc/taur/docs/services.md").is_file());
    let env = fs::read_to_string(stage.join("etc/taur/taur.env")).unwrap();
    assert!(env.contains("TAUR_HOME=/var/lib/taur"), "{env}");
    let mode = fs::metadata(stage.join("etc/taur/taur.env"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640, "env mode is {mode:o}");
    let sysusers = fs::read_to_string(stage.join("usr/lib/sysusers.d/taur.conf")).unwrap();
    assert!(
        sysusers.contains("/usr/sbin/nologin"),
        "checkout sysusers was not used: {sysusers}"
    );
    assert!(!out.path().join("SHA256SUMS").exists());
}
