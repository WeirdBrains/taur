//! TAUR core library: everything front ends (HTTP server, CLI, later MCP/gRPC) share.

pub mod accounts;
pub mod dump;
pub mod error;
pub mod keys;
pub mod mail;
pub mod orgs;
pub mod push;
pub mod repo;
pub mod roles;
pub mod rpc;
pub mod srcinfo;
pub mod store;

pub use error::{Error, Result};
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
pub const DEFAULT_BASE_URL: &str = "https://taur.weirdbrains.com";
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
