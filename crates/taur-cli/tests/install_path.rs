//! Documented system (`/usr/bin`) and user (`$HOME/.local/bin`) installs.
//! Isolated prefix/HOME fixtures only; does not touch the host profile or PATH.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn write_stub(path: &Path, name: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, format!("#!/bin/sh\necho \"{name} 1.5.0\"\n")).unwrap();
    let mut mode = fs::metadata(path).unwrap().permissions();
    mode.set_mode(0o755);
    fs::set_permissions(path, mode).unwrap();
}

fn run_in(dir: &Path, path: &str, home: &Path, script: &str) -> (bool, String) {
    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", path)
        .env("HOME", home)
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn check_script() -> &'static str {
    "command -v taur && command -v taurctl && taur --version && taurctl --version"
}

#[test]
fn system_install_puts_both_binaries_on_a_login_path() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    fs::create_dir_all(&src).unwrap();
    write_stub(&src.join("taur"), "taur");
    write_stub(&src.join("taurctl"), "taurctl");
    let prefix = root.path().join("sys");
    fs::create_dir_all(prefix.join("usr/bin")).unwrap();
    // Documented system install: install -m755 taur taurctl /usr/bin/
    let status = Command::new("install")
        .args(["-m755", "taur", "taurctl"])
        .arg(prefix.join("usr/bin"))
        .current_dir(&src)
        .status()
        .unwrap();
    assert!(status.success());

    let elsewhere = root.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let path = format!(
        "{}:/usr/sbin:/usr/bin:/sbin:/bin",
        prefix.join("usr/bin").display()
    );
    let (ok, text) = run_in(&elsewhere, &path, root.path(), check_script());
    assert!(ok, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], prefix.join("usr/bin/taur").to_str().unwrap());
    assert_eq!(lines[1], prefix.join("usr/bin/taurctl").to_str().unwrap());
    assert_eq!(lines[2], "taur 1.5.0");
    assert_eq!(lines[3], "taurctl 1.5.0");
}

#[test]
fn user_install_uses_local_bin_and_does_not_edit_the_profile() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    fs::create_dir_all(&src).unwrap();
    write_stub(&src.join("taur"), "taur");
    write_stub(&src.join("taurctl"), "taurctl");
    let home = root.path().join("home");
    let local_bin = home.join(".local/bin");
    // Documented user install.
    let status = Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"mkdir -p "$HOME/.local/bin" && install -m755 taur taurctl "$HOME/.local/bin/""#)
        .current_dir(&src)
        .env("HOME", &home)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(local_bin.join("taur").is_file());
    assert!(local_bin.join("taurctl").is_file());
    // No profile files were created or rewritten.
    for name in [".profile", ".bashrc", ".zshrc", ".config/environment.d"] {
        assert!(!home.join(name).exists(), "must not write {name}");
    }

    let elsewhere = root.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let with_local = format!("{}:/usr/bin:/bin", local_bin.display());
    let (ok, text) = run_in(&elsewhere, &with_local, &home, check_script());
    assert!(ok, "{text}");
    assert!(
        text.contains(&format!("{}/taur\n", local_bin.display())),
        "{text}"
    );
    assert!(
        text.contains(&format!("{}/taurctl\n", local_bin.display())),
        "{text}"
    );
    assert!(
        text.contains("taur 1.5.0") && text.contains("taurctl 1.5.0"),
        "{text}"
    );

    // Without ~/.local/bin on PATH, the binaries are not found; we do not rewrite the profile.
    // An empty prefix so a host /usr/bin/taur cannot leak into the fixture.
    let empty = root.path().join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    let (ok, text) = run_in(&elsewhere, empty.to_str().unwrap(), &home, check_script());
    assert!(
        !ok,
        "expected command -v to fail without .local/bin on PATH: {text}"
    );
}

#[test]
fn package_stage_and_smoke_script_use_usr_bin() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let smoke = fs::read_to_string(repo.join("scripts/package-smoke.sh")).unwrap();
    assert!(smoke.contains("command -v taur") && smoke.contains("command -v taurctl"));
    assert!(smoke.contains("taur --version") && smoke.contains("taurctl --version"));
    assert!(
        smoke.contains(r#"PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin""#)
    );
    assert!(smoke.contains(r#"[ "$taur_path" = /usr/bin/taur ]"#));
    assert!(smoke.contains(r#"[ "$ctl_path" = /usr/bin/taurctl ]"#));
    assert!(smoke.contains("cd /tmp"));
    assert!(smoke.contains("check_bins_on_path after reinstall"));
}
