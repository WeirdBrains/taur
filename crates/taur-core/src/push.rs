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
            "PKGBUILD" => has_pkgbuild = true,
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
    Ok(info)
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
        let text = git_out(repo, &["show", "master:.SRCINFO"])?;
        let info = srcinfo::parse(&text)?;
        let new = self.pkgbase_id(pkgbase)?.is_none();
        self.import_srcinfo(&info, if new { Some(pusher) } else { None })?;
        let mut c = git();
        c.arg("--git-dir").arg(repo).arg("update-server-info");
        run(c)?;
        Ok(())
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
