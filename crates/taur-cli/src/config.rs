//! CLI configuration directory: `config.json` (server URL) and `token` (0600).

use crate::CliError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const DEFAULT_SERVER: &str = "http://127.0.0.1:8080";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub server: Option<String>,
}

/// `--config-dir`, then `TAURCTL_CONFIG_DIR`, then `$XDG_CONFIG_HOME/taurctl`, then
/// `~/.config/taurctl`.
pub fn resolve_dir(flag: Option<PathBuf>) -> PathBuf {
    if let Some(p) = flag.or_else(|| std::env::var_os("TAURCTL_CONFIG_DIR").map(PathBuf::from)) {
        return p;
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".config")
        });
    base.join("taurctl")
}

pub fn load(dir: &Path) -> Result<Config, CliError> {
    match fs::read_to_string(dir.join("config.json")) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| CliError::Config(e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(CliError::Config(e.to_string())),
    }
}

pub fn save(dir: &Path, cfg: &Config) -> Result<(), CliError> {
    fs::create_dir_all(dir).map_err(|e| CliError::Config(e.to_string()))?;
    let s = serde_json::to_string_pretty(cfg).map_err(|e| CliError::Config(e.to_string()))?;
    fs::write(dir.join("config.json"), s + "\n").map_err(|e| CliError::Config(e.to_string()))
}

pub fn read_token(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join("token"))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Write the session token with mode 0600 (directory 0700).
pub fn write_token(dir: &Path, token: &str) -> Result<(), CliError> {
    let err = |e: std::io::Error| CliError::Config(e.to_string());
    fs::create_dir_all(dir).map_err(err)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(err)?;
    let path = dir.join("token");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(err)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(err)?;
    f.write_all(token.as_bytes()).map_err(err)
}

pub fn clear_token(dir: &Path) {
    let _ = fs::remove_file(dir.join("token"));
}
