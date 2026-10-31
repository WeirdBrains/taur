//! Service configs under packaging/ exist and only use documented environment variables.

use std::path::PathBuf;
use taur_server::cli::ENV_VARS;

const FILES: &[&str] = &[
    "systemd/taur.service",
    "systemd/taur.sysusers",
    "systemd/taur.tmpfiles",
    "sysvinit/taur",
    "openrc/taur",
    "runit/taur/run",
    "runit/taur/log/run",
    "launchd/com.weirdbrains.taur.plist",
    "launchd/taur-launchd",
    "windows/taur-service.xml",
    "windows/install-service.ps1",
    "windows/uninstall-service.ps1",
    "taur.env.example",
];

fn vars(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while let Some(p) = text[i..].find("TAUR") {
        let start = i + p;
        let mut end = start;
        while end < b.len() && (b[end].is_ascii_uppercase() || b[end] == b'_') {
            end += 1;
        }
        let word = &text[start..end];
        // `TAUR_*` (a wildcard in prose) is not a variable name.
        let word = word.trim_end_matches('_');
        if word.starts_with("TAUR_") || word.starts_with("TAURCTL_") {
            out.push(word.to_string());
        }
        i = end.max(start + 4);
    }
    out
}

#[test]
fn service_configs_use_documented_env_vars() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging");
    let docs = std::fs::read_to_string(root.join("../docs/services.md")).unwrap();
    let known: Vec<&str> = ENV_VARS.iter().map(|v| v.0).collect();
    for f in FILES {
        let path = root.join(f);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing {f}"));
        for v in vars(&text) {
            assert!(known.contains(&v.as_str()), "{f} uses undocumented {v}");
        }
        assert!(docs.contains(f), "docs/services.md does not mention {f}");
        #[cfg(unix)]
        if text.starts_with("#!") {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "{f} is not executable");
        }
    }
    // Every service runs `taur serve`.
    for f in [
        "systemd/taur.service",
        "sysvinit/taur",
        "openrc/taur",
        "runit/taur/run",
        "launchd/taur-launchd",
        "windows/taur-service.xml",
    ] {
        assert!(
            std::fs::read_to_string(root.join(f))
                .unwrap()
                .contains("serve"),
            "{f}"
        );
    }
}
