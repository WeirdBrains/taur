//! TAUR core library: everything front ends (HTTP server, CLI, later MCP/gRPC) share.

pub mod accounts;
pub mod community;
pub mod dump;
pub mod error;
pub mod gitserve;
pub mod keys;
pub mod mail;
pub mod mirror;
pub mod moderation;
pub mod orgs;
pub mod push;
pub mod ratelimit;
pub mod registry;
pub mod repo;
pub mod roles;
pub mod rpc;
pub mod srcinfo;
pub mod store;
pub mod upload;

pub use error::{Error, Result};

/// Null device for `GIT_CONFIG_GLOBAL` (ignore the service user's git config).
pub const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// Set Unix permission bits (no-op on other platforms, where ACLs of the data
/// directory apply instead).
pub fn set_mode(path: &std::path::Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// Variables a child process needs besides `PATH` after `env_clear()` (Windows needs
/// a few system variables for git to start).
pub fn base_child_env() -> Vec<(String, std::ffi::OsString)> {
    let keys: &[&str] = if cfg!(windows) {
        &[
            "PATH",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "COMSPEC",
            "PATHEXT",
        ]
    } else {
        &["PATH"]
    };
    keys.iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (k.to_string(), v)))
        .collect()
}
pub use ratelimit::{LimitClass, RatePolicy};
pub use roles::{Action, Role};
pub use store::{PkgInfo, SearchBy, Store};

use std::path::PathBuf;

/// Resolve the data directory: explicit flag, then `TAUR_HOME`, then `~/.local/share/taur`.
pub fn resolve_home(flag: Option<PathBuf>) -> PathBuf {
    if let Some(p) = flag {
        return p;
    }
    if let Some(p) = std::env::var_os("TAUR_HOME") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".local/share/taur")
}

/// Default public base URL used in emails.
pub const DEFAULT_BASE_URL: &str = "https://packages.taur.weirdbrains.com";
pub const DEFAULT_MAIL_FROM: &str = "TAUR <noreply@taur.weirdbrains.com>";

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
