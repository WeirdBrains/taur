//! Push validation and metadata updates, modelled on aurweb's git-update hook.
//!
//! The server runs git (smart HTTP or SSH) with `TAUR_HOOK_BIN`, `TAUR_HOME` and
//! `TAUR_PUSHER` set; the repo hooks call `taur hook pre-receive|post-receive`, which
//! call into these functions.

use crate::repo::{git, run};
use crate::srcinfo::{self, SrcInfo};
use crate::{Error, Result, Store};
use std::path::Path;

/// Largest allowed blob (aurweb's default `max_blob_size`).
pub const MAX_BLOB_SIZE: u64 = 256 * 1024;
const ZERO: &str = "0000000000000000000000000000000000000000";

#[derive(Debug, Clone, PartialEq)]
pub struct RefUpdate {
    pub old: String,
    pub new: String,
    pub refname: String,
}

/// Parse hook stdin (`<old> <new> <ref>` per line).
pub fn parse_updates(input: &str) -> Vec<RefUpdate> {
    input
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some(RefUpdate {
                old: it.next()?.to_string(),
                new: it.next()?.to_string(),
                refname: it.next()?.to_string(),
            })
        })
        .collect()
}

fn is_zero(id: &str) -> bool {
    id.chars().all(|c| c == '0')
}

fn git_bytes(repo: &Path, commit: &str, path: &str) -> Result<Option<Vec<u8>>> {
    let spec = format!("{commit}:{path}");
    let mut c = git();
    c.arg("--git-dir")
        .arg(repo)
        .args(["cat-file", "blob", &spec]);
    match run(c) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(_) => Ok(None),
    }
}

fn git_text(repo: &Path, commit: &str, path: &str) -> Option<String> {
    let bytes = git_bytes(repo, commit, path).ok()??;
    let text = String::from_utf8(bytes).ok()?;
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

fn git_out(repo: &Path, args: &[&str]) -> Result<String> {
    let mut c = git();
    c.arg("--git-dir").arg(repo).args(args);
    Ok(String::from_utf8_lossy(&run(c)?).into_owned())
}

fn deny(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

/// Validate one commit's tree; returns its parsed `.SRCINFO`.
fn check_commit(repo: &Path, pkgbase: &str, commit: &str) -> Result<SrcInfo> {
    let short = &commit[..commit.len().min(8)];
    let tree = git_out(repo, &["ls-tree", "-l", commit])?;
    let mut srcinfo_blob = None;
    let mut pkgbuild_blob = None;
    let mut has_pkgbuild = false;
    for line in tree.lines() {
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let f: Vec<&str> = meta.split_whitespace().collect();
        let (mode, kind, id, size) = (f[0], f[1], f[2], f.get(3).copied().unwrap_or("-"));
        if kind != "blob" || mode == "160000" {
            return Err(deny(format!(
                "{short}: subdirectories and submodules are not allowed ({path})"
            )));
        }
        if size.parse::<u64>().unwrap_or(0) > MAX_BLOB_SIZE {
            return Err(deny(format!(
                "{short}: {path} exceeds the {MAX_BLOB_SIZE} byte size limit"
            )));
        }
        match path {
            ".SRCINFO" => srcinfo_blob = Some(id.to_string()),
            "PKGBUILD" => {
                has_pkgbuild = true;
                pkgbuild_blob = Some(id.to_string());
            }
            _ => {}
        }
    }
    if !has_pkgbuild {
        return Err(deny(format!("{short}: missing PKGBUILD")));
    }
    let blob = srcinfo_blob.ok_or_else(|| deny(format!("{short}: missing .SRCINFO")))?;
    let text = git_out(repo, &["cat-file", "blob", &blob])?;
    let info = srcinfo::parse(&text).map_err(|e| deny(format!("{short}: .SRCINFO: {e}")))?;
    if info.pkgbase != pkgbase {
        return Err(deny(format!(
            "{short}: .SRCINFO pkgbase {} does not match repository {pkgbase}",
            info.pkgbase
        )));
    }
    if let Some(id) = pkgbuild_blob {
        let pkgbuild = git_out(repo, &["cat-file", "blob", &id])?;
        let mismatches = srcinfo::literal_mismatches(&info, &pkgbuild);
        if !mismatches.is_empty() {
            return Err(deny(format!(
                "{short}: {} (simple PKGBUILD assignments only; shell was not executed)",
                mismatches.join("; ")
            )));
        }
        isolated_pkgbuild(repo, &pkgbuild, &info)?;
    }
    Ok(info)
}

/// Opt-in `TAUR_PKGBUILD_ISOLATION=bwrap` runs `makepkg --printsrcinfo` in a
/// bubblewrap sandbox (no home, no network, read-only package files). Any other
/// value is refused. Unset skips the check. This executes the PKGBUILD.
fn isolated_pkgbuild(repo: &Path, pkgbuild: &str, info: &SrcInfo) -> Result<()> {
    let mode = match std::env::var("TAUR_PKGBUILD_ISOLATION") {
        Err(_) => return Ok(()),
        Ok(v) if v == "bwrap" => v,
        Ok(v) => {
            return Err(deny(format!(
                "TAUR_PKGBUILD_ISOLATION={v} is not supported (only bwrap runs PKGBUILDs, and only inside a sandbox)"
            )));
        }
    };
    let _ = mode;
    if std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_err()
    {
        return Err(deny(
            "TAUR_PKGBUILD_ISOLATION=bwrap but bwrap is not installed; refusing to run the PKGBUILD on the host",
        ));
    }
    let dir = tempfile_dir()?;
    std::fs::write(dir.join("PKGBUILD"), pkgbuild)?;
    let mut cmd = std::process::Command::new("bwrap");
    cmd.args([
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
        "--ro-bind",
    ])
    .arg(&dir)
    .args(["/work", "--chdir", "/work", "--clearenv"])
    .args(["--setenv", "PATH", "/usr/bin:/bin"])
    .args(["--setenv", "HOME", "/tmp"])
    .args(["makepkg", "--printsrcinfo"]);
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| deny(format!("cannot start bwrap: {e}")))?;
    let finished = wait_timeout(&mut child, std::time::Duration::from_secs(20))?;
    if !finished.status.success() {
        return Err(deny(format!(
            "isolated makepkg --printsrcinfo failed: {}",
            String::from_utf8_lossy(&finished.stderr).trim()
        )));
    }
    let printed = srcinfo::parse(&String::from_utf8_lossy(&finished.stdout))
        .map_err(|e| deny(format!("isolated .SRCINFO: {e}")))?;
    if printed.pkgbase != info.pkgbase || printed.version() != info.version() {
        return Err(deny(format!(
            "isolated makepkg .SRCINFO ({} {}) does not match the committed .SRCINFO ({} {})",
            printed.pkgbase,
            printed.version(),
            info.pkgbase,
            info.version()
        )));
    }
    let _ = repo;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn tempfile_dir() -> Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "taur-pkgbuild-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

struct Done {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn wait_timeout(child: &mut std::process::Child, limit: std::time::Duration) -> Result<Done> {
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(Error::Io)? {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(mut o) = child.stdout.take() {
                std::io::Read::read_to_end(&mut o, &mut stdout)?;
            }
            if let Some(mut e) = child.stderr.take() {
                std::io::Read::read_to_end(&mut e, &mut stderr)?;
            }
            return Ok(Done {
                status,
                stdout,
                stderr,
            });
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(deny("isolated makepkg --printsrcinfo timed out"));
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

impl Store {
    /// Check a push before git accepts it. Errors are user-facing messages.
    pub fn pre_receive(
        &self,
        repo: &Path,
        pkgbase: &str,
        pusher: &str,
        updates: &[RefUpdate],
    ) -> Result<()> {
        let user = self
            .user_by_name(pusher)?
            .ok_or_else(|| deny(format!("unknown user {pusher}")))?;
        self.require_verified(&user)?;
        self.ensure_not_suspended(user.id)?;
        if self.pkgbase_suspended(pkgbase)? {
            return Err(deny(format!("package {pkgbase} is rejected")));
        }
        let existing = self.pkgbase_id(pkgbase)?;
        if let Some(id) = existing
            && !self.can_manage_pkgbase(user.id, id)?
        {
            return Err(deny(format!(
                "permission denied: {pusher} cannot push to {pkgbase}"
            )));
        }
        for u in updates {
            if u.refname != "refs/heads/master" {
                return Err(deny("pushing to a branch other than master is restricted"));
            }
            if is_zero(&u.new) {
                return Err(deny("deleting the master branch is not allowed"));
            }
            let range = if is_zero(&u.old) {
                u.new.clone()
            } else {
                let mut c = git();
                c.arg("--git-dir")
                    .arg(repo)
                    .args(["merge-base", "--is-ancestor", &u.old, &u.new]);
                if run(c).is_err() {
                    return Err(deny("denying non-fast-forward (you should pull first)"));
                }
                format!("{}..{}", u.old, u.new)
            };
            let commits = git_out(repo, &["rev-list", &range])?;
            let mut tip = None;
            for c in commits.lines() {
                let info = check_commit(repo, pkgbase, c)?;
                if tip.is_none() {
                    tip = Some(info);
                }
            }
            self.review_pushed_recipe(repo, pkgbase, pusher, &u.new)?;
            if let Some(info) = tip {
                for (name, _) in &info.packages {
                    if let Some(owner) = self.package_owner(name)?
                        && owner != pkgbase
                    {
                        return Err(deny(format!("package {name} already exists in {owner}")));
                    }
                }
            }
        }
        let _ = ZERO;
        Ok(())
    }

    /// After a successful push: refresh metadata from the new master `.SRCINFO`.
    /// A new package base gets the pusher as maintainer and submitter.
    pub fn post_receive(&self, repo: &Path, pkgbase: &str, pusher: &str) -> Result<()> {
        let reviewed = self.review_pushed_recipe(repo, pkgbase, pusher, "master")?;
        let text = git_out(repo, &["show", "master:.SRCINFO"])?;
        let info = srcinfo::parse(&text)?;
        let new = self.pkgbase_id(pkgbase)?.is_none();
        let pb = self.import_srcinfo_in(
            &info,
            if new { Some(pusher) } else { None },
            &reviewed.registry,
        )?;
        if reviewed.decision == crate::registry::UploadDecision::NewVersion
            && let (Some(repository), Some(signature)) = (reviewed.repository, reviewed.signature)
        {
            self.record_new_version(
                &reviewed.registry,
                pkgbase,
                &info.version(),
                &repository,
                &signature,
            )?;
        }
        if !new {
            let actor = self.user_by_name(pusher)?.map(|u| u.id);
            self.notify_subscribers(
                pb,
                actor,
                &format!("{pkgbase} updated to {}", info.version()),
                &format!(
                    "{pusher} pushed {pkgbase} {}.\n\n{}/packages/{pkgbase}\n",
                    info.version(),
                    self.base_url()
                ),
            );
        }
        let mut c = git();
        c.arg("--git-dir").arg(repo).arg("update-server-info");
        run(c)?;
        Ok(())
    }

    /// Apply the package-registry policy to the tree `commit` before it is accepted.
    pub fn review_pushed_recipe(
        &self,
        repo: &Path,
        pkgbase: &str,
        pusher: &str,
        commit: &str,
    ) -> Result<crate::registry::ReviewedPush> {
        let srcinfo = git_bytes(repo, commit, ".SRCINFO")?
            .ok_or_else(|| deny(format!("{pkgbase}: missing .SRCINFO")))?;
        let pkgbuild = git_bytes(repo, commit, "PKGBUILD")?
            .ok_or_else(|| deny(format!("{pkgbase}: missing PKGBUILD")))?;
        let info = srcinfo::parse(&String::from_utf8_lossy(&srcinfo))
            .map_err(|e| deny(format!("{pkgbase}: .SRCINFO: {e}")))?;
        let version = info.version();
        let registry_file = git_text(repo, commit, "TAUR-REGISTRY");
        let registry = self
            .resolve_registry(pkgbase, None, &registry_file)
            .map_err(|e| deny(e.to_string()))?;
        let repository = git_text(repo, commit, "TAUR-REPOSITORY");
        let signature = git_text(repo, commit, "TAUR-RECIPE.sig")
            .and_then(|text| crate::registry::parse_hex_key(&text, 64).ok());
        let actor = self.user_by_name(pusher)?.map(|u| u.id);
        let decision = self
            .check_upload(&crate::registry::RecipeUpload {
                registry: &registry,
                repository: repository.as_deref(),
                pkgbase,
                version: &version,
                srcinfo: &srcinfo,
                pkgbuild: &pkgbuild,
                signature: signature.as_deref(),
                actor,
            })
            .map_err(|e| deny(e.to_string()))?;
        Ok(crate::registry::ReviewedPush {
            decision,
            registry,
            repository,
            signature,
        })
    }

    /// Name of the pkgbase that owns package `name`, if any.
    pub fn package_owner(&self, name: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .lock()
            .query_row(
                "SELECT b.name FROM packages p JOIN pkgbases b ON b.id = p.pkgbase_id WHERE p.name = ?1",
                [name],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hook_input() {
        let u = parse_updates(&format!("{ZERO} abc refs/heads/master\n"));
        assert_eq!(u[0].refname, "refs/heads/master");
        assert!(is_zero(&u[0].old));
    }
}
