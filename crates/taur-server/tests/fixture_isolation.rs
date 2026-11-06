//! Build and install the repository fixture only inside bubblewrap.
//!
//! This runs `tests/fixtures/hello-taur`, not a package from the network.
//! `makepkg` and `pacman` execute in a user namespace with `/usr` read-only,
//! no home directory, and no network. The install root is a temp directory.
//! The test does not use host `sudo` and does not install into `/`.
//!
//! Limits: the sandbox still sees the host `/usr` and `/etc/makepkg.conf`.
//! `makepkg --nodeps` and `pacman -dd` skip real dependency installation.
//! Bubblewrap is not a virtual machine. Set `TAUR_REQUIRE_ISOLATION=1` to
//! fail when `bwrap` or `makepkg` is absent; otherwise the test skips.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn fixture_build_and_install_stay_inside_bwrap() {
    for bin in ["bwrap", "makepkg", "pacman"] {
        if Command::new(bin).arg("--version").output().is_err() {
            if std::env::var_os("TAUR_REQUIRE_ISOLATION").is_some() {
                panic!("{bin} is required for the fixture isolation test");
            }
            eprintln!("skipping fixture isolation test: {bin} is not installed");
            return;
        }
    }
    let host_marker = Path::new("/usr/share/hello-taur");
    assert!(
        !host_marker.exists(),
        "refusing to run: {host_marker:?} already exists on the host"
    );

    let work = tempfile::tempdir().unwrap();
    let pkg = work.path().join("pkg");
    fs::create_dir_all(&pkg).unwrap();
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur");
    fs::copy(fx.join("PKGBUILD"), pkg.join("PKGBUILD")).unwrap();
    fs::copy(fx.join(".SRCINFO"), pkg.join(".SRCINFO")).unwrap();
    for rel in ["root", "db/local", "db/sync", "cache", "gnupg", "hooks"] {
        fs::create_dir_all(work.path().join(rel)).unwrap();
    }
    fs::write(
        work.path().join("pacman.conf"),
        "[options]\nRootDir = /work/root\nDBPath = /work/db\nCacheDir = /work/cache\n\
         LogFile = /work/pacman.log\nGPGDir = /work/gnupg\nHookDir = /work/hooks\n\
         HoldPkg = pacman\nArchitecture = x86_64\nSigLevel = Never\nLocalFileSigLevel = Never\n",
    )
    .unwrap();

    let (ok, text) = bwrap(work.path(), false, &["makepkg", "--printsrcinfo"]);
    assert!(ok, "isolated printsrcinfo failed:\n{text}");
    assert!(text.contains("pkgbase = hello-taur"), "{text}");
    assert!(text.contains("pkgver = 1.2.3"), "{text}");
    assert!(text.contains("pkgname = hello-taur-docs"), "{text}");

    let (ok, text) = bwrap(
        work.path(),
        false,
        &["makepkg", "--noconfirm", "--noprogressbar", "--nodeps"],
    );
    assert!(ok, "isolated build failed:\n{text}");
    let built = built_packages(&pkg);
    assert_eq!(built.len(), 2, "built packages: {built:?}\n{text}");

    let mut install = vec![
        "pacman".into(),
        "--noconfirm".into(),
        "-dd".into(),
        "--config".into(),
        "/work/pacman.conf".into(),
        "-U".into(),
    ];
    for path in &built {
        let name = path.file_name().unwrap().to_string_lossy();
        install.push(format!("/work/pkg/{name}"));
    }
    let args: Vec<&str> = install.iter().map(String::as_str).collect();
    let (ok, text) = bwrap(work.path(), true, &args);
    assert!(ok, "isolated install failed:\n{text}");
    assert!(
        work.path()
            .join("root/usr/share/hello-taur/hello")
            .is_file(),
        "package files missing from the sandbox root"
    );
    assert!(
        work.path()
            .join("root/usr/share/doc/hello-taur/README")
            .is_file()
    );
    assert!(
        !host_marker.exists(),
        "fixture was installed on the host at {host_marker:?}"
    );
}

fn built_packages(pkg: &Path) -> Vec<PathBuf> {
    fs::read_dir(pkg)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(".pkg.tar."))
        })
        .collect()
}

fn bwrap(work: &Path, as_root: bool, cmd: &[&str]) -> (bool, String) {
    let mut args = vec![
        "--unshare-all",
        "--die-with-parent",
        "--ro-bind",
        "/usr",
        "/usr",
        "--symlink",
        "usr/bin",
        "/bin",
        "--symlink",
        "usr/lib",
        "/lib",
        "--symlink",
        "usr/lib64",
        "/lib64",
        "--ro-bind",
        "/etc/makepkg.conf",
        "/etc/makepkg.conf",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--tmpfs",
        "/run",
        "--tmpfs",
        "/home",
        "--bind",
    ];
    let work_s = work.to_str().unwrap().to_string();
    args.push(&work_s);
    args.extend([
        "/work",
        "--chdir",
        "/work/pkg",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "HOME",
        "/tmp",
    ]);
    if as_root {
        args.splice(0..0, ["--unshare-user", "--uid", "0", "--gid", "0"]);
        args.extend(["--setenv", "USER", "root"]);
    }
    args.push("--");
    args.extend(cmd);
    let mut child = Command::new("bwrap")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn bwrap");
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut v = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut v).unwrap();
        v
    });
    let err = std::thread::spawn(move || {
        let mut v = Vec::new();
        std::io::Read::read_to_end(&mut stderr, &mut v).unwrap();
        v
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("bwrap timed out");
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.join().unwrap()),
        String::from_utf8_lossy(&err.join().unwrap())
    );
    (status.success(), text)
}
