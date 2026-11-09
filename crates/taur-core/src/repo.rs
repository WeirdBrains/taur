//! Per-pkgbase bare git repositories (`<home>/repos/<pkgbase>.git`), served over git's
//! "dumb" HTTP protocol, plus snapshot tarballs. Uses the system `git` binary with
//! system/global config disabled so the user's settings never leak in.

use crate::srcinfo::validate_name;
use crate::{Error, Result, Store};
use std::path::Path;
use std::process::Command;

pub(crate) fn git() -> Command {
    let mut c = Command::new("git");
    c.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", crate::NULL_DEVICE)
        .env("GIT_TERMINAL_PROMPT", "0");
    c
}

pub(crate) fn run(mut c: Command) -> Result<Vec<u8>> {
    let out = c
        .output()
        .map_err(|e| Error::Git(format!("cannot run git: {e}")))?;
    if !out.status.success() {
        return Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(out.stdout)
}

/// Commit the contents of `dir` as the new tip of `master` in the pkgbase repo.
pub fn commit_dir(repos: &Path, pkgbase: &str, dir: &Path, message: &str) -> Result<()> {
    validate_name(pkgbase)?;
    let dir = &dir.canonicalize()?;
    std::fs::create_dir_all(repos)?;
    let repo = ensure_repo(repos, pkgbase)?;
    let with_tree = || {
        let mut c = git();
        c.arg("--git-dir")
            .arg(&repo)
            .arg("--work-tree")
            .arg(dir)
            .current_dir(dir);
        c
    };
    let mut c = with_tree();
    c.args(["add", "-A", "."]);
    run(c)?;
    let mut c = with_tree();
    c.args([
        "-c",
        "user.name=TAUR",
        "-c",
        "user.email=taur@localhost",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        message,
    ]);
    run(c)?;
    let mut c = git();
    c.arg("--git-dir").arg(&repo).arg("update-server-info");
    run(c)?;
    Ok(())
}

const HOOK: &str = "#!/bin/sh\n# Installed by TAUR. Pushes are only accepted through the TAUR server.\n[ -n \"$TAUR_HOOK_BIN\" ] || { echo 'error: pushes must go through the TAUR server' >&2; exit 1; }\nexec \"$TAUR_HOOK_BIN\" hook NAME\n";

/// Create the bare repo for `pkgbase` if needed and (re)install TAUR's hooks.
pub fn ensure_repo(repos: &Path, pkgbase: &str) -> Result<std::path::PathBuf> {
    validate_name(pkgbase)?;
    std::fs::create_dir_all(repos)?;
    let repo = repos.join(format!("{pkgbase}.git"));
    if !repo.exists() {
        let mut c = git();
        c.args(["init", "--quiet", "--bare", "-b", "master"])
            .arg(&repo);
        run(c)?;
        let mut c = git();
        c.arg("--git-dir")
            .arg(&repo)
            .args(["config", "receive.denyNonFastForwards", "true"]);
        run(c)?;
    }
    let hooks = repo.join("hooks");
    std::fs::create_dir_all(&hooks)?;
    for name in ["pre-receive", "post-receive"] {
        let path = hooks.join(name);
        std::fs::write(&path, HOOK.replace("NAME", name))?;
        crate::set_mode(&path, 0o755)?;
    }
    Ok(repo)
}

/// Delete a repository that never received a commit.
///
/// `git receive-pack` quarantines a push and deletes that quarantine when
/// `pre-receive` fails, so the rejected pack is not left behind. Creating the
/// bare repo before the push still leaves an empty directory. A repository
/// that already has a ref is kept, including after a later push is refused.
pub fn remove_if_unborn(repos: &Path, pkgbase: &str) -> Result<bool> {
    validate_name(pkgbase)?;
    let repo = repos.join(format!("{pkgbase}.git"));
    if !repo.is_dir() {
        return Ok(false);
    }
    if repo_has_refs(&repo)? {
        return Ok(false);
    }
    let root = repos.canonicalize().unwrap_or_else(|_| repos.to_path_buf());
    let canon = repo.canonicalize().unwrap_or_else(|_| repo.clone());
    if !canon.starts_with(&root) {
        return Err(Error::Invalid(
            "refusing to remove a repository outside the data directory".into(),
        ));
    }
    std::fs::remove_dir_all(&canon)?;
    Ok(true)
}

fn repo_has_refs(repo: &Path) -> Result<bool> {
    let mut c = git();
    c.arg("--git-dir").arg(repo).arg("show-ref");
    let out = c
        .output()
        .map_err(|e| Error::Git(format!("cannot run git: {e}")))?;
    Ok(out.status.success() && !out.stdout.is_empty())
}

/// True when a push or an existing `git gc` is using this repository.
pub fn repo_busy(repo: &Path) -> bool {
    let objects = repo.join("objects");
    if let Ok(rd) = std::fs::read_dir(&objects) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("incoming") || name == "gc.pid" || name.ends_with(".lock") {
                return true;
            }
        }
    }
    for rel in ["HEAD.lock", "packed-refs.lock", "gc.pid"] {
        if repo.join(rel).exists() {
            return true;
        }
    }
    ref_locks(&repo.join("refs"))
}

fn ref_locks(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if ref_locks(&path) {
                return true;
            }
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".lock"))
        {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaintainReport {
    pub collected: u32,
    pub skipped: u32,
}

/// `git gc --prune` for every package repository.
///
/// `grace` is how long an unreachable object is kept. Objects newer than that
/// stay, which covers a push whose pack has been copied in but whose ref is
/// not updated yet. A repository with a quarantine directory or a lock is
/// skipped entirely.
pub fn maintain_repos(repos: &Path, grace: std::time::Duration) -> Result<MaintainReport> {
    let mut report = MaintainReport {
        collected: 0,
        skipped: 0,
    };
    let Ok(rd) = std::fs::read_dir(repos) else {
        return Ok(report);
    };
    let prune = if grace.is_zero() {
        "now".to_string()
    } else {
        format!("{}.seconds.ago", grace.as_secs().max(1))
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == ".deleted" || !name.ends_with(".git") || !path.is_dir() {
            continue;
        }
        if repo_busy(&path) {
            report.skipped += 1;
            continue;
        }
        let mut c = git();
        c.arg("--git-dir").arg(&path).args([
            "-c",
            "gc.auto=0",
            "gc",
            "--quiet",
            &format!("--prune={prune}"),
        ]);
        run(c)?;
        report.collected += 1;
    }
    Ok(report)
}

/// Blob bytes at `rev:path` inside the package base repo (cgit `plain` URLs).
/// `rev` defaults to `HEAD` when empty. `path` is a file inside the tree, not a URL.
pub fn plain(repos: &Path, pkgbase: &str, rev: &str, path: &str) -> Result<Vec<u8>> {
    validate_name(pkgbase)?;
    let rev = if rev.is_empty() { "HEAD" } else { rev };
    if !rev
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        || rev.contains("..")
    {
        return Err(Error::Invalid("invalid revision".into()));
    }
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == ".." || c.contains('\\'))
    {
        return Err(Error::Invalid("invalid path".into()));
    }
    let repo = repos.join(format!("{pkgbase}.git"));
    if !repo.is_dir() {
        return Err(Error::NotFound(format!("package base {pkgbase}")));
    }
    let spec = format!("{rev}:{path}");
    let mut c = git();
    c.arg("--git-dir")
        .arg(&repo)
        .args(["cat-file", "blob", &spec]);
    run(c).map_err(|_| Error::NotFound(format!("{path} in {pkgbase}")))
}

/// `git archive` of HEAD as `<pkgbase>/...` in a gzipped tarball.
pub fn snapshot(repos: &Path, pkgbase: &str) -> Result<Vec<u8>> {
    validate_name(pkgbase)?;
    let repo = repos.join(format!("{pkgbase}.git"));
    if !repo.is_dir() {
        return Err(Error::NotFound(format!("package base {pkgbase}")));
    }
    let mut c = git();
    c.arg("--git-dir")
        .arg(&repo)
        .args(["archive", "--format=tar.gz"])
        .arg(format!("--prefix={pkgbase}/"))
        .arg("HEAD");
    run(c)
}

/// Resolve a dumb-HTTP request path like `/foo.git/info/refs` to file bytes and a
/// content type. Only `HEAD`, `info/*`, `objects/*`, `refs/*`, `packed-refs` are served.
pub fn git_file(repos: &Path, path: &str) -> Option<(Vec<u8>, &'static str)> {
    let p = path.trim_start_matches('/');
    let (name, rest) = p.split_once(".git/")?;
    validate_name(name).ok()?;
    if rest.is_empty()
        || rest
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == ".." || c.contains('\\'))
    {
        return None;
    }
    let allowed = rest == "HEAD"
        || rest == "packed-refs"
        || rest.starts_with("info/")
        || rest.starts_with("objects/")
        || rest.starts_with("refs/");
    if !allowed {
        return None;
    }
    let file = repos.join(format!("{name}.git")).join(rest);
    if !file.is_file() {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    let ctype = if rest.starts_with("objects/") {
        "application/octet-stream"
    } else {
        "text/plain; charset=utf-8"
    };
    Some((bytes, ctype))
}

impl Store {
    pub fn snapshot(&self, pkgbase: &str) -> Result<Vec<u8>> {
        snapshot(&self.repos_dir(), pkgbase)
    }

    /// File from a package base tree. `h` is the package base (aurweb cgit `?h=`).
    pub fn plain(&self, pkgbase: &str, rev: &str, path: &str) -> Result<Vec<u8>> {
        plain(&self.repos_dir(), pkgbase, rev, path)
    }

    pub fn git_file(&self, path: &str) -> Option<(Vec<u8>, &'static str)> {
        git_file(&self.repos_dir(), path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_file_rejects_traversal() {
        let d = tempfile::tempdir().unwrap();
        assert!(git_file(d.path(), "/x.git/../taur.db").is_none());
        assert!(git_file(d.path(), "/x.git/config").is_none());
        assert!(git_file(d.path(), "/X.git/HEAD").is_none());
        assert!(git_file(d.path(), "/nope.git/HEAD").is_none());
    }
}
