//! Per-pkgbase bare git repositories (`<home>/repos/<pkgbase>.git`), served over git's
//! "dumb" HTTP protocol, plus snapshot tarballs. Uses the system `git` binary with
//! system/global config disabled so the user's settings never leak in.

use crate::srcinfo::validate_name;
use crate::{Error, Result, Store};
use std::path::Path;
use std::process::Command;

fn git() -> Command {
    let mut c = Command::new("git");
    c.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0");
    c
}

fn run(mut c: Command) -> Result<Vec<u8>> {
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
    let repo = repos.join(format!("{pkgbase}.git"));
    if !repo.exists() {
        let mut c = git();
        c.args(["init", "--quiet", "--bare", "-b", "master"])
            .arg(&repo);
        run(c)?;
    }
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
