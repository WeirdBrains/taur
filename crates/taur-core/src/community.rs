//! AUR community features: votes and popularity, out-of-date flags, comments,
//! co-maintainers, package requests (orphan, deletion, merge) and site admins.

use crate::{Error, Result, Store, now};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

/// aurweb's popularity: each vote is worth 0.98^(days since the vote).
pub fn popularity(vote_times: &[i64], at: i64) -> f64 {
    vote_times
        .iter()
        .map(|t| 0.98f64.powf(((at - t).max(0)) as f64 / 86400.0))
        .sum()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comment {
    pub id: i64,
    pub author: Option<String>,
    pub content: String,
    pub created_at: i64,
    pub edited_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Request {
    pub id: i64,
    pub pkgbase: String,
    pub requester: Option<String>,
    pub kind: String,
    pub merge_into: Option<String>,
    pub comments: String,
    pub status: String,
    pub closure_comment: Option<String>,
    pub created_at: i64,
    pub closed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Flag {
    pub out_of_date: Option<i64>,
    pub flagger: Option<String>,
    pub comment: Option<String>,
}

pub const REQUEST_KINDS: [&str; 3] = ["orphan", "deletion", "merge"];

impl Store {
    fn require_pkgbase(&self, name: &str) -> Result<i64> {
        self.pkgbase_id(name)?
            .ok_or_else(|| Error::NotFound(format!("package base {name}")))
    }

    pub fn is_admin(&self, user: i64) -> Result<bool> {
        Ok(self
            .lock()
            .query_row("SELECT is_admin FROM users WHERE id = ?1", [user], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or(false))
    }

    /// Grant or revoke site-admin rights (accept requests, manage any package).
    pub fn set_admin(&self, user: i64, admin: bool) -> Result<()> {
        self.lock().execute(
            "UPDATE users SET is_admin = ?1 WHERE id = ?2",
            params![admin, user],
        )?;
        Ok(())
    }

    // ---- votes ----

    fn recount(&self, pb: i64) -> Result<()> {
        let conn = self.lock();
        let times: Vec<i64> = {
            let mut st = conn.prepare("SELECT voted_at FROM votes WHERE pkgbase_id = ?1")?;
            st.query_map([pb], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        conn.execute(
            "UPDATE pkgbases SET num_votes = ?1, popularity = ?2 WHERE id = ?3",
            params![times.len() as i64, popularity(&times, now()), pb],
        )?;
        Ok(())
    }

    /// Recompute popularity for every package base (run periodically; decays daily).
    pub fn refresh_popularity(&self) -> Result<()> {
        let ids: Vec<i64> = {
            let conn = self.lock();
            let mut st = conn.prepare("SELECT id FROM pkgbases")?;
            st.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for id in ids {
            self.recount(id)?;
        }
        Ok(())
    }

    pub fn vote(&self, user: i64, pkgbase: &str) -> Result<()> {
        let pb = self.require_pkgbase(pkgbase)?;
        self.lock().execute(
            "INSERT OR IGNORE INTO votes (pkgbase_id, user_id, voted_at) VALUES (?1, ?2, ?3)",
            params![pb, user, now()],
        )?;
        self.recount(pb)
    }

    pub fn unvote(&self, user: i64, pkgbase: &str) -> Result<()> {
        let pb = self.require_pkgbase(pkgbase)?;
        self.lock().execute(
            "DELETE FROM votes WHERE pkgbase_id = ?1 AND user_id = ?2",
            params![pb, user],
        )?;
        self.recount(pb)
    }

    pub fn has_voted(&self, user: i64, pkgbase: &str) -> Result<bool> {
        let pb = self.require_pkgbase(pkgbase)?;
        Ok(self
            .lock()
            .query_row(
                "SELECT 1 FROM votes WHERE pkgbase_id = ?1 AND user_id = ?2",
                params![pb, user],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    // ---- out-of-date ----

    pub fn flag_out_of_date(&self, user: i64, pkgbase: &str, comment: &str) -> Result<()> {
        let pb = self.require_pkgbase(pkgbase)?;
        if comment.trim().is_empty() {
            return Err(Error::Invalid(
                "a comment explaining the flag is required".into(),
            ));
        }
        let n = self.lock().execute(
            "UPDATE pkgbases SET out_of_date = ?1, flagger_id = ?2, flagger_comment = ?3 WHERE id = ?4 AND out_of_date IS NULL",
            params![now(), user, comment.trim(), pb],
        )?;
        if n == 0 {
            return Err(Error::Conflict(format!(
                "{pkgbase} is already flagged out-of-date"
            )));
        }
        self.notify_maintainers(
            pb,
            &format!("{pkgbase} flagged out-of-date"),
            &format!("{pkgbase} was flagged out-of-date:\n\n{}\n", comment.trim()),
        );
        Ok(())
    }

    /// Maintainers (and anyone who can manage the package) or the flagger may unflag.
    pub fn unflag(&self, user: i64, pkgbase: &str) -> Result<()> {
        let pb = self.require_pkgbase(pkgbase)?;
        let flagger: Option<i64> =
            self.lock()
                .query_row("SELECT flagger_id FROM pkgbases WHERE id = ?1", [pb], |r| {
                    r.get(0)
                })?;
        if flagger != Some(user) && !self.can_manage_pkgbase(user, pb)? {
            return Err(Error::Forbidden);
        }
        self.lock().execute(
            "UPDATE pkgbases SET out_of_date = NULL, flagger_id = NULL, flagger_comment = NULL WHERE id = ?1",
            [pb],
        )?;
        Ok(())
    }

    pub fn flag_info(&self, pkgbase: &str) -> Result<Flag> {
        let pb = self.require_pkgbase(pkgbase)?;
        Ok(self.lock().query_row(
            "SELECT b.out_of_date, u.name, b.flagger_comment FROM pkgbases b LEFT JOIN users u ON u.id = b.flagger_id WHERE b.id = ?1",
            [pb],
            |r| {
                Ok(Flag {
                    out_of_date: r.get(0)?,
                    flagger: r.get(1)?,
                    comment: r.get(2)?,
                })
            },
        )?)
    }

    // ---- co-maintainers ----

    /// Replace the co-maintainer list. Allowed for the maintainer, site admins and org
    /// members who may manage members.
    pub fn set_comaintainers(&self, actor: i64, pkgbase: &str, users: &[String]) -> Result<()> {
        let pb = self.require_pkgbase(pkgbase)?;
        let (maintainer, org): (Option<i64>, Option<i64>) = self.lock().query_row(
            "SELECT maintainer_id, org_id FROM pkgbases WHERE id = ?1",
            [pb],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let org_ok = match org {
            Some(o) => self
                .role_in(o, actor)?
                .is_some_and(|r| r.allows(crate::Action::ManageMembers)),
            None => false,
        };
        if maintainer != Some(actor) && !org_ok && !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        let ids = users
            .iter()
            .map(|u| self.user_id(u))
            .collect::<Result<Vec<i64>>>()?;
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM co_maintainers WHERE pkgbase_id = ?1", [pb])?;
        for id in ids {
            if Some(id) != maintainer {
                tx.execute(
                    "INSERT OR IGNORE INTO co_maintainers (pkgbase_id, user_id) VALUES (?1, ?2)",
                    params![pb, id],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // ---- comments ----

    pub fn add_comment(&self, user: i64, pkgbase: &str, content: &str) -> Result<Comment> {
        let pb = self.require_pkgbase(pkgbase)?;
        let content = content.trim();
        if content.is_empty() || content.len() > 10_000 {
            return Err(Error::Invalid("comments must be 1-10000 characters".into()));
        }
        let t = now();
        let id = {
            let conn = self.lock();
            conn.execute(
                "INSERT INTO comments (pkgbase_id, user_id, content, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![pb, user, content, t],
            )?;
            conn.last_insert_rowid()
        };
        self.comment(id)
    }

    fn comment(&self, id: i64) -> Result<Comment> {
        self.lock()
            .query_row(
                "SELECT c.id, u.name, c.content, c.created_at, c.edited_at FROM comments c LEFT JOIN users u ON u.id = c.user_id WHERE c.id = ?1 AND c.deleted_at IS NULL",
                [id],
                |r| {
                    Ok(Comment {
                        id: r.get(0)?,
                        author: r.get(1)?,
                        content: r.get(2)?,
                        created_at: r.get(3)?,
                        edited_at: r.get(4)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("comment {id}")))
    }

    /// Comments on a package base, newest first (deleted ones hidden).
    pub fn comments(&self, pkgbase: &str) -> Result<Vec<Comment>> {
        let pb = self.require_pkgbase(pkgbase)?;
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT c.id, u.name, c.content, c.created_at, c.edited_at FROM comments c LEFT JOIN users u ON u.id = c.user_id \
             WHERE c.pkgbase_id = ?1 AND c.deleted_at IS NULL ORDER BY c.id DESC",
        )?;
        let rows = st.query_map([pb], |r| {
            Ok(Comment {
                id: r.get(0)?,
                author: r.get(1)?,
                content: r.get(2)?,
                created_at: r.get(3)?,
                edited_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn comment_owner(&self, id: i64) -> Result<(Option<i64>, i64)> {
        self.lock()
            .query_row(
                "SELECT user_id, pkgbase_id FROM comments WHERE id = ?1 AND deleted_at IS NULL",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("comment {id}")))
    }

    /// Only the author may edit.
    pub fn edit_comment(&self, user: i64, id: i64, content: &str) -> Result<Comment> {
        let (author, _) = self.comment_owner(id)?;
        if author != Some(user) {
            return Err(Error::Forbidden);
        }
        if content.trim().is_empty() {
            return Err(Error::Invalid("comments must not be empty".into()));
        }
        self.lock().execute(
            "UPDATE comments SET content = ?1, edited_at = ?2 WHERE id = ?3",
            params![content.trim(), now(), id],
        )?;
        self.comment(id)
    }

    /// The author, anyone who can manage the package, or a site admin may delete.
    pub fn delete_comment(&self, user: i64, id: i64) -> Result<()> {
        let (author, pb) = self.comment_owner(id)?;
        if author != Some(user) && !self.can_manage_pkgbase(user, pb)? {
            return Err(Error::Forbidden);
        }
        self.lock().execute(
            "UPDATE comments SET deleted_at = ?1 WHERE id = ?2",
            params![now(), id],
        )?;
        Ok(())
    }

    // ---- requests ----

    pub fn file_request(
        &self,
        user: i64,
        pkgbase: &str,
        kind: &str,
        merge_into: Option<&str>,
        comments: &str,
    ) -> Result<Request> {
        let pb = self.require_pkgbase(pkgbase)?;
        if !REQUEST_KINDS.contains(&kind) {
            return Err(Error::Invalid(
                "kind must be orphan, deletion or merge".into(),
            ));
        }
        if comments.trim().is_empty() {
            return Err(Error::Invalid(
                "a comment explaining the request is required".into(),
            ));
        }
        let merge_into = match (kind, merge_into) {
            ("merge", Some(t)) if t != pkgbase => {
                self.require_pkgbase(t)?;
                Some(t.to_string())
            }
            ("merge", _) => {
                return Err(Error::Invalid(
                    "merge requests need a different target package base".into(),
                ));
            }
            (_, _) => None,
        };
        let id = {
            let conn = self.lock();
            conn.execute(
                "INSERT INTO requests (pkgbase_id, pkgbase_name, user_id, kind, merge_into, comments, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![pb, pkgbase, user, kind, merge_into, comments.trim(), now()],
            )?;
            conn.last_insert_rowid()
        };
        self.notify_maintainers(
            pb,
            &format!("{kind} request for {pkgbase}"),
            &format!(
                "A {kind} request was filed for {pkgbase}:\n\n{}\n",
                comments.trim()
            ),
        );
        self.request(id)
    }

    pub fn request(&self, id: i64) -> Result<Request> {
        self.requests_where("r.id = ?1", id)?
            .pop()
            .ok_or_else(|| Error::NotFound(format!("request {id}")))
    }

    /// All requests, or only pending ones; newest first.
    pub fn list_requests(&self, pending_only: bool) -> Result<Vec<Request>> {
        if pending_only {
            self.requests_where("r.status = 'pending' AND ?1 = ?1", 0)
        } else {
            self.requests_where("?1 = ?1", 0)
        }
    }

    fn requests_where(&self, filter: &str, arg: i64) -> Result<Vec<Request>> {
        let conn = self.lock();
        let mut st = conn.prepare(&format!(
            "SELECT r.id, r.pkgbase_name, u.name, r.kind, r.merge_into, r.comments, r.status, r.closure_comment, r.created_at, r.closed_at \
             FROM requests r LEFT JOIN users u ON u.id = r.user_id WHERE {filter} ORDER BY r.id DESC"
        ))?;
        let rows = st.query_map([arg], |r| {
            Ok(Request {
                id: r.get(0)?,
                pkgbase: r.get(1)?,
                requester: r.get(2)?,
                kind: r.get(3)?,
                merge_into: r.get(4)?,
                comments: r.get(5)?,
                status: r.get(6)?,
                closure_comment: r.get(7)?,
                created_at: r.get(8)?,
                closed_at: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn close_request(&self, actor: i64, id: i64, status: &str, comment: &str) -> Result<()> {
        let n = self.lock().execute(
            "UPDATE requests SET status = ?1, closure_comment = ?2, closed_by = ?3, closed_at = ?4 WHERE id = ?5 AND status = 'pending'",
            params![status, comment, actor, now(), id],
        )?;
        if n == 0 {
            return Err(Error::Conflict(format!("request {id} is not pending")));
        }
        Ok(())
    }

    /// Site admins accept a request and carry it out:
    /// orphan → maintainer cleared; deletion → package base removed (repo moved to
    /// `repos/.deleted/`); merge → votes and comments move to the target, source removed.
    pub fn accept_request(&self, actor: i64, id: i64, comment: &str) -> Result<()> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        let req = self.request(id)?;
        if req.status != "pending" {
            return Err(Error::Conflict(format!("request {id} is not pending")));
        }
        let pb = self.require_pkgbase(&req.pkgbase)?;
        match req.kind.as_str() {
            "orphan" => {
                self.lock().execute(
                    "UPDATE pkgbases SET maintainer_id = NULL WHERE id = ?1",
                    [pb],
                )?;
            }
            "deletion" => self.delete_pkgbase(pb, &req.pkgbase)?,
            "merge" => {
                let target = req.merge_into.clone().unwrap_or_default();
                let tid = self.require_pkgbase(&target)?;
                {
                    let conn = self.lock();
                    conn.execute(
                        "INSERT OR IGNORE INTO votes (pkgbase_id, user_id, voted_at) SELECT ?1, user_id, voted_at FROM votes WHERE pkgbase_id = ?2",
                        params![tid, pb],
                    )?;
                    conn.execute(
                        "UPDATE comments SET pkgbase_id = ?1 WHERE pkgbase_id = ?2",
                        params![tid, pb],
                    )?;
                }
                self.delete_pkgbase(pb, &req.pkgbase)?;
                self.recount(tid)?;
            }
            _ => return Err(Error::Invalid("unknown request kind".into())),
        }
        self.close_request(actor, id, "accepted", comment)
    }

    /// Site admins reject; the requester may close (withdraw) their own request.
    pub fn reject_request(&self, actor: i64, id: i64, comment: &str) -> Result<()> {
        let req = self.request(id)?;
        let own = req
            .requester
            .as_deref()
            .is_some_and(|n| self.user_id(n).ok() == Some(actor));
        if self.is_admin(actor)? {
            self.close_request(actor, id, "rejected", comment)
        } else if own {
            self.close_request(actor, id, "closed", comment)
        } else {
            Err(Error::Forbidden)
        }
    }

    fn delete_pkgbase(&self, pb: i64, name: &str) -> Result<()> {
        self.lock()
            .execute("DELETE FROM pkgbases WHERE id = ?1", [pb])?;
        let repo = self.repos_dir().join(format!("{name}.git"));
        if repo.exists() {
            let trash = self.repos_dir().join(".deleted");
            std::fs::create_dir_all(&trash)?;
            std::fs::rename(&repo, trash.join(format!("{name}-{}.git", now())))?;
        }
        Ok(())
    }

    /// Best-effort email to the maintainer and co-maintainers.
    fn notify_maintainers(&self, pb: i64, subject: &str, body: &str) {
        let emails: Vec<String> = (|| -> Result<Vec<String>> {
            let conn = self.lock();
            let mut st = conn.prepare(
                "SELECT u.email FROM users u WHERE u.id = (SELECT maintainer_id FROM pkgbases WHERE id = ?1) \
                 UNION SELECT u.email FROM co_maintainers c JOIN users u ON u.id = c.user_id WHERE c.pkgbase_id = ?1",
            )?;
            Ok(st.query_map([pb], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
        })()
        .unwrap_or_default();
        let mailer = self.mailer();
        for to in emails {
            let _ = mailer.send(&to, &format!("[TAUR] {subject}"), body);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn popularity_decays() {
        let now = 100 * 86400;
        assert!((super::popularity(&[now], now) - 1.0).abs() < 1e-9);
        let p = super::popularity(&[now - 86400], now);
        assert!((p - 0.98).abs() < 1e-9);
        assert_eq!(super::popularity(&[], now), 0.0);
    }
}
