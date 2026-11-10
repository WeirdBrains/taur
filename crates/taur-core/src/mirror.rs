//! Recipe mirrors.
//!
//! An administrator copies selected package bases from a Git source that uses
//! the same layout as the AUR and as TAUR: `{base}/{pkgbase}.git` with a
//! `master` branch that contains `.SRCINFO` and `PKGBUILD`. A pacman binary
//! repository is not that layout and is refused.
//!
//! Mirrored packages live in their own registry with enforcement `mirror`.
//! That registry is read-only to uploaders. It is not an operator signature
//! and it is not an Arch or AUR verification. A suspended package is not
//! updated. A package already kept in another registry is not overwritten.

use crate::registry::MIRROR_ENFORCEMENT;
use crate::srcinfo::{self, literal_mismatches};
use crate::{Error, Result, Store, now, repo};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn unique_temp(prefix: &str, pkgbase: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "{prefix}-{pkgbase}-{}-{nanos}-{n}",
        std::process::id()
    ))
}

/// Shown on a mirror registry. Not [`crate::registry::PROVENANCE_OPERATOR`].
pub const PROVENANCE_MIRROR: &str = "mirror";
/// A selected package whose last successful copy is older than this is stale.
/// The copy is kept.
pub const STALE_AFTER_SECS: i64 = 7 * 24 * 60 * 60;
const MAX_SELECTED: i64 = 200;
const MAX_FILES: usize = 64;
const MAX_TREE_BYTES: u64 = 1024 * 1024;
const FETCH_LIMIT: Duration = Duration::from_secs(60);

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS mirrors (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  base_url TEXT NOT NULL,
  created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS mirror_packages (
  id INTEGER PRIMARY KEY,
  mirror_id INTEGER NOT NULL REFERENCES mirrors(id) ON DELETE CASCADE,
  pkgbase TEXT NOT NULL,
  revision TEXT,
  synced_at INTEGER,
  status TEXT NOT NULL,
  detail TEXT NOT NULL DEFAULT '',
  UNIQUE (mirror_id, pkgbase));
"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MirrorInfo {
    pub name: String,
    pub base_url: String,
    pub registry: String,
    pub provenance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MirrorPackage {
    pub pkgbase: String,
    pub status: String,
    pub revision: Option<String>,
    pub synced_at: Option<i64>,
    pub stale: bool,
    pub detail: String,
}

pub fn migrate(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

/// `true` when a successful copy is older than [`STALE_AFTER_SECS`].
pub fn is_stale(synced_at: i64, now_secs: i64) -> bool {
    now_secs.saturating_sub(synced_at) > STALE_AFTER_SECS
}

impl Store {
    /// Record a recipe Git source and its read-only registry. Does not fetch.
    pub fn add_mirror(&self, name: &str, base_url: &str) -> Result<()> {
        let name = clean_mirror_name(name)?;
        let base_url = validate_base_url(base_url)?;
        if name == crate::registry::LEGACY_REGISTRY {
            return Err(Error::Conflict(format!(
                "{name} is the existing package registry and cannot be a mirror"
            )));
        }
        let exists: Option<i64> = self
            .lock()
            .query_row("SELECT id FROM registries WHERE name = ?1", [&name], |r| {
                r.get(0)
            })
            .optional()?;
        if exists.is_some() {
            return Err(Error::Conflict(format!("registry {name} already exists")));
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO registries (name, enforcement, created_at) VALUES (?1, ?2, ?3)",
            params![name, MIRROR_ENFORCEMENT, now()],
        )?;
        tx.execute(
            "INSERT INTO mirrors (name, base_url, created_at) VALUES (?1, ?2, ?3)",
            params![name, base_url, now()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_mirrors(&self) -> Result<Vec<MirrorInfo>> {
        let conn = self.lock();
        let mut st = conn.prepare("SELECT name, base_url FROM mirrors ORDER BY name")?;
        let rows = st.query_map([], |r| {
            let name: String = r.get(0)?;
            Ok(MirrorInfo {
                registry: name.clone(),
                name,
                base_url: r.get(1)?,
                provenance: PROVENANCE_MIRROR.to_string(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Remember a package base to copy. Does not fetch it.
    pub fn select_mirror_package(&self, mirror: &str, pkgbase: &str) -> Result<()> {
        srcinfo::validate_name(pkgbase)?;
        let id = self.mirror_id(mirror)?;
        let count: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM mirror_packages WHERE mirror_id = ?1 AND status != 'unselected'",
            [id],
            |r| r.get(0),
        )?;
        if count >= MAX_SELECTED {
            return Err(Error::Invalid(format!(
                "a mirror can select at most {MAX_SELECTED} packages"
            )));
        }
        self.lock().execute(
            "INSERT INTO mirror_packages (mirror_id, pkgbase, status, detail) VALUES (?1, ?2, 'selected', '')
             ON CONFLICT(mirror_id, pkgbase) DO UPDATE SET status = CASE WHEN status = 'unselected' THEN 'selected' ELSE status END",
            params![id, pkgbase],
        )?;
        Ok(())
    }

    /// Stop copying this package. The copied recipe stays until [`Store::drop_mirrored_package`].
    pub fn unselect_mirror_package(&self, mirror: &str, pkgbase: &str) -> Result<()> {
        let id = self.mirror_id(mirror)?;
        let n = self.lock().execute(
            "UPDATE mirror_packages SET status = 'unselected', detail = 'not selected; the last copy is kept' WHERE mirror_id = ?1 AND pkgbase = ?2",
            params![id, pkgbase],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!(
                "{pkgbase} is not selected on {mirror}"
            )));
        }
        Ok(())
    }

    pub fn mirror_status(&self, mirror: &str) -> Result<Vec<MirrorPackage>> {
        let id = self.mirror_id(mirror)?;
        let now = now();
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT pkgbase, status, revision, synced_at, detail FROM mirror_packages WHERE mirror_id = ?1 ORDER BY pkgbase",
        )?;
        let rows = st.query_map([id], |r| {
            let synced_at: Option<i64> = r.get(3)?;
            let status: String = r.get(1)?;
            let stale = synced_at
                .is_some_and(|t| (status == "synced" || status == "unchanged") && is_stale(t, now));
            Ok(MirrorPackage {
                pkgbase: r.get(0)?,
                stale,
                status,
                revision: r.get(2)?,
                synced_at,
                detail: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Fetch each selected package. One failure does not roll back the others.
    /// The same upstream revision is a no-op. Missing or failing upstreams keep
    /// the last good copy.
    pub fn sync_mirror(&self, mirror: &str) -> Result<Vec<MirrorPackage>> {
        let (id, base_url) = self.mirror_row(mirror)?;
        let selected: Vec<(String, Option<String>)> = {
            let conn = self.lock();
            let mut st = conn.prepare(
                "SELECT pkgbase, revision FROM mirror_packages WHERE mirror_id = ?1 AND status != 'unselected' ORDER BY pkgbase",
            )?;
            let rows = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (pkgbase, previous) in selected {
            let outcome = self.sync_one(mirror, &base_url, &pkgbase, previous.as_deref());
            self.record_mirror_result(id, &pkgbase, &outcome)?;
        }
        self.mirror_status(mirror)
    }

    /// Delete one copied package that belongs to this mirror. A local package
    /// with the same name is left alone.
    pub fn drop_mirrored_package(&self, mirror: &str, pkgbase: &str) -> Result<()> {
        srcinfo::validate_name(pkgbase)?;
        let id = self.mirror_id(mirror)?;
        match self.pkgbase_registry(pkgbase)? {
            Some(current) if current == mirror => {}
            Some(current) => {
                return Err(Error::Conflict(format!(
                    "{pkgbase} is in registry {current} and is not dropped from mirror {mirror}"
                )));
            }
            None => {}
        }
        if let Some(pb) = self.pkgbase_id(pkgbase)?
            && self.pkgbase_registry(pkgbase)?.as_deref() == Some(mirror)
        {
            self.delete_mirrored_base(pb, pkgbase)?;
        }
        self.lock().execute(
            "DELETE FROM mirror_packages WHERE mirror_id = ?1 AND pkgbase = ?2",
            params![id, pkgbase],
        )?;
        Ok(())
    }

    /// Remove the source. Refuses while any of its packages are still stored.
    pub fn remove_mirror(&self, mirror: &str) -> Result<()> {
        let id = self.mirror_id(mirror)?;
        let held: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM pkgbases b JOIN registries r ON r.id = b.registry_id WHERE r.name = ?1",
            [mirror],
            |r| r.get(0),
        )?;
        if held > 0 {
            return Err(Error::Conflict(format!(
                "mirror {mirror} still has {held} package(s); drop them before removing the source"
            )));
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM mirrors WHERE id = ?1", [id])?;
        tx.execute(
            "DELETE FROM registries WHERE name = ?1 AND enforcement = ?2",
            params![mirror, MIRROR_ENFORCEMENT],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn sync_one(
        &self,
        mirror: &str,
        base_url: &str,
        pkgbase: &str,
        previous: Option<&str>,
    ) -> MirrorPackage {
        let fail = |status: &str, detail: String| MirrorPackage {
            pkgbase: pkgbase.to_string(),
            status: status.to_string(),
            revision: previous.map(str::to_string),
            synced_at: None,
            stale: false,
            detail,
        };
        if let Err(e) = srcinfo::validate_name(pkgbase) {
            return fail("failed", e.to_string());
        }
        if self.pkgbase_suspended(pkgbase).unwrap_or(false) {
            return fail(
                "blocked",
                "package is rejected; the mirror did not update it or clear the rejection".into(),
            );
        }
        match self.pkgbase_registry(pkgbase) {
            Ok(Some(current)) if current != mirror => {
                return fail(
                    "blocked",
                    format!(
                        "{pkgbase} is in registry {current} and was not overwritten; a mirror is not an operator attestation"
                    ),
                );
            }
            Err(e) => return fail("failed", e.to_string()),
            _ => {}
        }
        let url = match clone_url(base_url, pkgbase) {
            Ok(url) => url,
            Err(e) => return fail("failed", e.to_string()),
        };
        let fetched = match fetch_recipe(&url, pkgbase) {
            Ok(tree) => tree,
            Err(e) => {
                let message = clip(&e.to_string());
                let status = if previous.is_some() && upstream_missing(&message) {
                    "upstream-missing"
                } else {
                    "failed"
                };
                let detail = if status == "upstream-missing" {
                    "upstream repository is missing; the last copy is kept".to_string()
                } else if previous.is_some() {
                    format!("fetch failed; the last copy is kept: {message}")
                } else {
                    format!("fetch failed; nothing was stored: {message}")
                };
                return fail(status, detail);
            }
        };
        if previous == Some(fetched.revision.as_str())
            && self.pkgbase_id(pkgbase).ok().flatten().is_some()
        {
            return MirrorPackage {
                pkgbase: pkgbase.to_string(),
                status: "unchanged".into(),
                revision: Some(fetched.revision),
                synced_at: Some(now()),
                stale: false,
                detail: format!("upstream {url} revision unchanged"),
            };
        }
        if let Err(e) = self.store_tree(mirror, pkgbase, &url, &fetched) {
            return fail("failed", format!("copy refused: {e}"));
        }
        MirrorPackage {
            pkgbase: pkgbase.to_string(),
            status: "synced".into(),
            revision: Some(fetched.revision),
            synced_at: Some(now()),
            stale: false,
            detail: format!("mirrored from {url}; not an operator signature"),
        }
    }

    fn store_tree(&self, mirror: &str, pkgbase: &str, url: &str, tree: &Fetched) -> Result<()> {
        let dir = unique_temp(&format!("taur-mirror-{mirror}"), pkgbase);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let result = (|| {
            for (name, bytes) in &tree.files {
                let path = dir.join(name);
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                file.write_all(bytes)?;
            }
            let note = format!(
                "mirror: {mirror}\nsource: {url}\nrevision: {}\nnote: mirrored recipe; not an operator signature or an Arch or AUR verification\n",
                tree.revision
            );
            std::fs::write(dir.join("TAUR-MIRROR"), note)?;
            repo::commit_dir(
                &self.repos_dir(),
                pkgbase,
                &dir,
                &format!("mirror {mirror} {}", tree.revision),
            )?;
            self.import_srcinfo_in(&tree.info, None, mirror)?;
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    fn record_mirror_result(
        &self,
        mirror_id: i64,
        pkgbase: &str,
        row: &MirrorPackage,
    ) -> Result<()> {
        self.lock().execute(
            "UPDATE mirror_packages SET revision = ?1, synced_at = COALESCE(?2, synced_at), status = ?3, detail = ?4 WHERE mirror_id = ?5 AND pkgbase = ?6",
            params![row.revision, row.synced_at, row.status, row.detail, mirror_id, pkgbase],
        )?;
        Ok(())
    }

    fn delete_mirrored_base(&self, pb: i64, name: &str) -> Result<()> {
        let repo = self.repos_dir().join(format!("{name}.git"));
        if repo.exists() {
            let trash = self.repos_dir().join(".deleted");
            std::fs::create_dir_all(&trash)?;
            let dest_name = format!("{name}-{pb}-{}.git", now());
            std::fs::rename(&repo, trash.join(&dest_name))?;
            self.lock().execute(
                "INSERT INTO deleted_pkgbases (name, repo_rel, maintainer_id, deleted_at) VALUES (?1, ?2, NULL, ?3)",
                params![name, format!(".deleted/{dest_name}"), now()],
            )?;
        }
        self.lock()
            .execute("DELETE FROM pkgbases WHERE id = ?1", [pb])?;
        Ok(())
    }

    fn mirror_id(&self, name: &str) -> Result<i64> {
        self.lock()
            .query_row("SELECT id FROM mirrors WHERE name = ?1", [name], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("mirror {name}")))
    }

    fn mirror_row(&self, name: &str) -> Result<(i64, String)> {
        self.lock()
            .query_row(
                "SELECT id, base_url FROM mirrors WHERE name = ?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("mirror {name}")))
    }
}

struct Fetched {
    revision: String,
    info: srcinfo::SrcInfo,
    files: Vec<(String, Vec<u8>)>,
}

fn fetch_recipe(url: &str, pkgbase: &str) -> Result<Fetched> {
    let dest = unique_temp("taur-mirror-fetch", pkgbase);
    let _ = std::fs::remove_dir_all(&dest);
    let mut cmd = mirror_git();
    cmd.args([
        "clone",
        "--bare",
        "--depth",
        "1",
        "--branch",
        "master",
        "--single-branch",
        "--",
        url,
    ])
    .arg(&dest);
    let fetched = run_timeout(cmd, FETCH_LIMIT).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&dest);
    });
    fetched?;
    let read = read_tree(&dest, pkgbase);
    let _ = std::fs::remove_dir_all(&dest);
    read
}

fn read_tree(git_dir: &Path, pkgbase: &str) -> Result<Fetched> {
    let revision = git_text(git_dir, &["rev-parse", "HEAD"])?;
    if revision.len() < 7 || !revision.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Invalid(
            "upstream revision is not a commit id".into(),
        ));
    }
    let listing = git_text(git_dir, &["ls-tree", "-l", "HEAD"])?;
    let mut files = Vec::new();
    let mut total = 0u64;
    for line in listing.lines() {
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let fields: Vec<&str> = meta.split_whitespace().collect();
        if fields.len() < 4 {
            return Err(Error::Invalid("upstream tree is not a flat recipe".into()));
        }
        let (mode, kind, blob, size) = (fields[0], fields[1], fields[2], fields[3]);
        if kind != "blob" || mode != "100644" && mode != "100755" {
            return Err(Error::Invalid(format!(
                "upstream path {path} is not a regular file; symlinks, directories, and submodules are refused"
            )));
        }
        let name = safe_file_name(path)?;
        let bytes_len: u64 = size.parse().unwrap_or(u64::MAX);
        if bytes_len > crate::push::MAX_BLOB_SIZE {
            return Err(Error::Invalid(format!(
                "{name} exceeds the {} byte size limit",
                crate::push::MAX_BLOB_SIZE
            )));
        }
        total = total.saturating_add(bytes_len);
        if total > MAX_TREE_BYTES || files.len() >= MAX_FILES {
            return Err(Error::Invalid(
                "upstream recipe is larger than the mirror limit".into(),
            ));
        }
        let bytes = git_bytes(git_dir, &["cat-file", "blob", blob])?;
        if bytes.len() as u64 != bytes_len && size != "-" {
            return Err(Error::Invalid(format!(
                "{name} size does not match the tree"
            )));
        }
        files.push((name, bytes));
    }
    let srcinfo = file_named(&files, ".SRCINFO")
        .ok_or_else(|| Error::Invalid("upstream tree has no .SRCINFO".into()))?;
    let pkgbuild = file_named(&files, "PKGBUILD")
        .ok_or_else(|| Error::Invalid("upstream tree has no PKGBUILD".into()))?;
    let text = String::from_utf8(srcinfo.clone())
        .map_err(|_| Error::Invalid("upstream .SRCINFO is not UTF-8".into()))?;
    let info = srcinfo::parse(&text)?;
    if info.pkgbase != pkgbase {
        return Err(Error::Invalid(format!(
            "upstream .SRCINFO pkgbase {} does not match {pkgbase}",
            info.pkgbase
        )));
    }
    let pkgbuild_text = String::from_utf8_lossy(pkgbuild);
    let mismatches = literal_mismatches(&info, &pkgbuild_text);
    if !mismatches.is_empty() {
        return Err(Error::Invalid(format!(
            "upstream recipe mismatch: {}",
            mismatches.join("; ")
        )));
    }
    let _ = pkgbuild;
    Ok(Fetched {
        revision,
        info,
        files,
    })
}

fn file_named<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a Vec<u8>> {
    files.iter().find(|(n, _)| n == name).map(|(_, b)| b)
}

fn safe_file_name(path: &str) -> Result<String> {
    if path.is_empty()
        || path == "."
        || path == ".."
        || path.contains('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.starts_with('.') && path != ".SRCINFO"
        || !path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
    {
        return Err(Error::Invalid(format!(
            "upstream path {path:?} is not a single recipe file name"
        )));
    }
    Ok(path.to_string())
}

fn mirror_git() -> Command {
    let mut cmd = repo::git();
    cmd.args([
        "-c",
        "protocol.file.allow=never",
        "-c",
        "protocol.ext.allow=never",
        "-c",
        "credential.helper=",
        "-c",
        "http.followRedirects=false",
        "-c",
        "http.extraHeader=",
        "-c",
        "core.hooksPath=/dev/null",
    ])
    .env("GIT_ASKPASS", "/bin/false")
    .env("GIT_TERMINAL_PROMPT", "0")
    .env_remove("SSH_AUTH_SOCK");
    cmd
}

fn git_text(git_dir: &Path, args: &[&str]) -> Result<String> {
    let bytes = git_bytes(git_dir, args)?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

fn git_bytes(git_dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut cmd = mirror_git();
    cmd.arg("--git-dir").arg(git_dir).args(args);
    run_timeout(cmd, Duration::from_secs(20))
}

fn run_timeout(mut cmd: Command, limit: Duration) -> Result<Vec<u8>> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Git(format!("cannot run git: {e}")))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_end(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_end(&mut stderr);
                }
                if status.success() {
                    return Ok(stdout);
                }
                return Err(Error::Git(clip(&String::from_utf8_lossy(&stderr))));
            }
            Ok(None) => {
                if start.elapsed() > limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Git("mirror fetch timed out".into()));
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            Err(e) => return Err(Error::Git(format!("cannot wait for git: {e}"))),
        }
    }
}

pub fn validate_base_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Invalid(
            "mirror URL must be one https or loopback Git address without spaces".into(),
        ));
    }
    if raw.contains("://") && raw.chars().any(|c| "\"'\\$`;&|<>".contains(c)) {
        return Err(Error::Invalid(
            "mirror URL contains a character that is not part of an address".into(),
        ));
    }
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Err(Error::Invalid(
            "mirror URL needs a scheme; a pacman Server= line is not a recipe source".into(),
        ));
    };
    let scheme = scheme.to_ascii_lowercase();
    if rest.contains('@') {
        return Err(Error::Invalid(
            "mirror URL must not include a username or password".into(),
        ));
    }
    if rest.contains('#') || rest.contains('?') {
        return Err(Error::Invalid(
            "mirror URL must not include a query or fragment".into(),
        ));
    }
    let (hostport, path) = match rest.split_once('/') {
        Some((host, path)) => (host, path.trim_end_matches('/')),
        None => (rest, ""),
    };
    if hostport.is_empty() || path.split('/').any(|p| p == ".." || p == ".") {
        return Err(Error::Invalid("mirror URL path is not usable".into()));
    }
    let host = hostport
        .rsplit_once(':')
        .map(|(h, port)| {
            if port.chars().all(|c| c.is_ascii_digit()) {
                h
            } else {
                hostport
            }
        })
        .unwrap_or(hostport);
    let host = host.trim_matches(|c| c == '[' || c == ']');
    let loopback = is_loopback(host);
    match scheme.as_str() {
        "https" => {}
        "http" | "git" if loopback => {}
        "http" | "git" => {
            return Err(Error::Invalid(
                "only https is allowed for a public mirror; http and git:// stay on loopback for a local fixture".into(),
            ));
        }
        _ => {
            return Err(Error::Invalid(format!(
                "scheme {scheme} is not a recipe mirror; file, ssh, and pacman database URLs are refused"
            )));
        }
    }
    let lower = raw.to_ascii_lowercase();
    for suffix in [
        ".db",
        ".db.tar.gz",
        ".db.tar.zst",
        ".files",
        ".files.tar.gz",
        ".files.tar.zst",
        ".pkg.tar.zst",
        ".pkg.tar.xz",
    ] {
        if lower.ends_with(suffix) {
            return Err(Error::Invalid(
                "that URL is a pacman binary repository or package archive, not a recipe Git source"
                    .into(),
            ));
        }
    }
    if lower.ends_with(".git") {
        return Err(Error::Invalid(
            "give the source base, not one package's .git URL; TAUR appends /{pkgbase}.git".into(),
        ));
    }
    Ok(raw.trim_end_matches('/').to_string())
}

fn clone_url(base: &str, pkgbase: &str) -> Result<String> {
    srcinfo::validate_name(pkgbase)?;
    let base = validate_base_url(base)?;
    Ok(format!("{base}/{pkgbase}.git"))
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn upstream_missing(message: &str) -> bool {
    let l = message.to_ascii_lowercase();
    l.contains("not found")
        || l.contains("no such repository")
        || l.contains("couldn't find remote ref")
        || l.contains("does not appear to be a git")
        || l.contains("repository not found")
}

fn clip(text: &str) -> String {
    let mut out: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(240)
        .collect();
    if text.chars().count() > 240 {
        out.push('…');
    }
    out
}

fn clean_mirror_name(name: &str) -> Result<String> {
    let name = name.trim().to_ascii_lowercase();
    let ok = (1..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(name)
    } else {
        Err(Error::Invalid(
            "mirror name must be 1-32 characters, start with a letter, and use a-z 0-9 _ -".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_reject_pacman_credentials_and_odd_schemes() {
        assert!(validate_base_url("https://aur.archlinux.org").is_ok());
        assert!(validate_base_url("http://127.0.0.1:9/recipes").is_ok());
        assert!(validate_base_url("git://localhost/aur").is_ok());
        for bad in [
            "file:///srv/aur",
            "ssh://git@example.com/aur",
            "https://user:secret@aur.archlinux.org",
            "http://aur.archlinux.org",
            "git://aur.archlinux.org",
            "https://example.com/core.db",
            "https://example.com/core.db.tar.gz",
            "https://example.com/pkg.pkg.tar.zst",
            "https://example.com/yay.git",
            "https://example.com/a/../../b",
            "ext::sh -c evil",
        ] {
            assert!(validate_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn staleness_is_age_not_deletion() {
        assert!(!is_stale(1_000, 1_000 + STALE_AFTER_SECS));
        assert!(is_stale(1_000, 1_001 + STALE_AFTER_SECS));
    }
}
