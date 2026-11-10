//! Site moderators and the rejection audit.
//!
//! A moderator may reject a package or an ordinary user. A site admin grants that
//! role and is the only account that can reverse a rejection. Rejected rows stay
//! in the database; the audit stores the decision, not package contents or secrets.

use crate::{Error, Result, Store, now};
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditEvent {
    pub id: i64,
    pub at: i64,
    pub action: String,
    pub target_kind: String,
    pub target: String,
    pub reason: String,
    pub actor: Option<String>,
    pub source: String,
    pub reverses: Option<i64>,
    /// A rejection that no later event has reversed.
    pub open: bool,
}

struct AuditWrite<'a> {
    action: &'a str,
    kind: &'a str,
    target: &'a str,
    reason: &'a str,
    actor_name: Option<&'a str>,
    source: &'a str,
    reverses: Option<i64>,
}

fn clean_reason(reason: &str) -> Result<String> {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(Error::Invalid(
            "a reason of 1 to 500 characters is required".into(),
        ));
    }
    if reason.chars().any(|c| c.is_control()) {
        return Err(Error::Invalid(
            "the reason cannot contain control characters".into(),
        ));
    }
    Ok(reason.to_string())
}

impl Store {
    pub fn is_moderator(&self, user: i64) -> Result<bool> {
        Ok(self
            .lock()
            .query_row(
                "SELECT is_moderator FROM users WHERE id = ?1",
                [user],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0)
            != 0)
    }

    /// Operator command. The HTTP and gRPC paths use [`Store::set_moderator`].
    pub fn grant_moderator(&self, user: i64, on: bool) -> Result<()> {
        let n = self.lock().execute(
            "UPDATE users SET is_moderator = ?1 WHERE id = ?2",
            params![i64::from(on), user],
        )?;
        if n == 0 {
            return Err(Error::NotFound("user".into()));
        }
        Ok(())
    }

    /// Site admins grant or remove the moderator role. Moderators cannot.
    pub fn set_moderator(&self, actor: i64, name: &str, on: bool) -> Result<()> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        let user = self
            .user_by_name(name)?
            .ok_or_else(|| Error::NotFound(format!("user {name}")))?;
        if self.user_suspended(user.id)? {
            return Err(Error::Conflict(format!(
                "{name} is suspended; reverse that rejection first"
            )));
        }
        if user.is_moderator == on {
            return Ok(());
        }
        self.lock().execute(
            "UPDATE users SET is_moderator = ?1 WHERE id = ?2",
            params![i64::from(on), user.id],
        )?;
        let actor_name = self.user_by_id(actor)?.name;
        let reason = if on {
            "granted moderator"
        } else {
            "removed moderator"
        };
        self.record(AuditWrite {
            action: if on { "grant" } else { "ungrant" },
            kind: "moderator",
            target: name,
            reason,
            actor_name: Some(&actor_name),
            source: "admin",
            reverses: None,
        })?;
        Ok(())
    }

    /// Grant or remove site admin. Site owners only. An owner stays an admin.
    pub fn set_site_admin(&self, actor: i64, name: &str, on: bool) -> Result<()> {
        if !self.is_owner(actor)? {
            return Err(Error::Forbidden);
        }
        let user = self
            .user_by_name(name)?
            .ok_or_else(|| Error::NotFound(format!("user {name}")))?;
        if user.is_admin == on {
            return Ok(());
        }
        if !on {
            let n: i64 = self.lock().query_row(
                "SELECT COUNT(*) FROM users WHERE is_admin != 0",
                [],
                |r| r.get(0),
            )?;
            if n <= 1 {
                return Err(Error::Conflict(
                    "the last site admin cannot be removed".into(),
                ));
            }
        }
        if self.user_suspended(user.id)? {
            return Err(Error::Conflict(format!(
                "{name} is suspended; reverse that rejection first"
            )));
        }
        self.set_admin(user.id, on)?;
        let actor_name = self.user_by_id(actor)?.name;
        let reason = if on {
            "granted site admin"
        } else {
            "removed site admin"
        };
        self.record(AuditWrite {
            action: if on { "grant" } else { "ungrant" },
            kind: "site-admin",
            target: name,
            reason,
            actor_name: Some(&actor_name),
            source: "owner",
            reverses: None,
        })?;
        Ok(())
    }

    /// On-machine grant. The last site owner cannot be removed. Granting owner
    /// also grants site admin. Revoking owner leaves the admin flag.
    pub fn grant_owner(&self, user: i64, on: bool) -> Result<()> {
        if !on {
            let n: i64 = self.lock().query_row(
                "SELECT COUNT(*) FROM users WHERE is_owner != 0",
                [],
                |r| r.get(0),
            )?;
            if n <= 1 && self.is_owner(user)? {
                return Err(Error::Conflict(
                    "the last site owner cannot be removed".into(),
                ));
            }
        }
        let n = self.lock().execute(
            "UPDATE users SET is_owner = ?1, is_admin = CASE WHEN ?1 = 1 THEN 1 ELSE is_admin END WHERE id = ?2",
            params![i64::from(on), user],
        )?;
        if n == 0 {
            return Err(Error::NotFound("user".into()));
        }
        Ok(())
    }

    /// Grant or remove site owner. Owners only. The last owner stays.
    pub fn set_site_owner(&self, actor: i64, name: &str, on: bool) -> Result<()> {
        if !self.is_owner(actor)? {
            return Err(Error::Forbidden);
        }
        let user = self
            .user_by_name(name)?
            .ok_or_else(|| Error::NotFound(format!("user {name}")))?;
        if user.is_owner == on {
            return Ok(());
        }
        if self.user_suspended(user.id)? {
            return Err(Error::Conflict(format!(
                "{name} is suspended; reverse that rejection first"
            )));
        }
        self.grant_owner(user.id, on)?;
        let actor_name = self.user_by_id(actor)?.name;
        let reason = if on {
            "granted site owner"
        } else {
            "removed site owner"
        };
        self.record(AuditWrite {
            action: if on { "grant" } else { "ungrant" },
            kind: "site-owner",
            target: name,
            reason,
            actor_name: Some(&actor_name),
            source: "owner",
            reverses: None,
        })?;
        Ok(())
    }

    pub fn user_suspended(&self, user: i64) -> Result<bool> {
        Ok(self
            .lock()
            .query_row(
                "SELECT suspended_at FROM users WHERE id = ?1",
                [user],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .is_some())
    }

    pub fn ensure_not_suspended(&self, user: i64) -> Result<()> {
        if self.user_suspended(user)? {
            Err(Error::Suspended("account is suspended".into()))
        } else {
            Ok(())
        }
    }

    pub fn pkgbase_suspended(&self, name: &str) -> Result<bool> {
        Ok(self
            .lock()
            .query_row(
                "SELECT suspended_at FROM pkgbases WHERE name = ?1",
                [name],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .is_some())
    }

    /// Reject a package. The git repository and metadata stay; public reads stop.
    pub fn reject_package(&self, actor: i64, pkgbase: &str, reason: &str) -> Result<i64> {
        let reason = clean_reason(reason)?;
        self.require_moderator(actor)?;
        let id = self
            .pkgbase_id(pkgbase)?
            .ok_or_else(|| Error::NotFound(format!("package base {pkgbase}")))?;
        if self.pkgbase_suspended(pkgbase)? {
            return Err(Error::Conflict(format!("{pkgbase} is already rejected")));
        }
        let source = self.actor_source(actor)?;
        let actor_name = self.user_by_id(actor)?.name;
        self.lock().execute(
            "UPDATE pkgbases SET suspended_at = ?1, suspended_reason = ?2 WHERE id = ?3",
            params![now(), reason, id],
        )?;
        self.record(AuditWrite {
            action: "reject",
            kind: "package",
            target: pkgbase,
            reason: &reason,
            actor_name: Some(&actor_name),
            source,
            reverses: None,
        })
    }

    /// Reject an ordinary account. Sessions end. The account row stays.
    pub fn reject_user(&self, actor: i64, name: &str, reason: &str) -> Result<i64> {
        let reason = clean_reason(reason)?;
        self.require_moderator(actor)?;
        let user = self
            .user_by_name(name)?
            .ok_or_else(|| Error::NotFound(format!("user {name}")))?;
        if user.id == actor {
            return Err(Error::Forbidden);
        }
        if user.is_admin || (user.is_moderator && !self.is_admin(actor)?) {
            return Err(Error::Forbidden);
        }
        if self.user_suspended(user.id)? {
            return Err(Error::Conflict(format!("{name} is already rejected")));
        }
        let source = self.actor_source(actor)?;
        let actor_name = self.user_by_id(actor)?.name;
        {
            let conn = self.lock();
            conn.execute(
                "UPDATE users SET suspended_at = ?1, suspended_reason = ?2 WHERE id = ?3",
                params![now(), reason, user.id],
            )?;
            conn.execute("DELETE FROM sessions WHERE user_id = ?1", [user.id])?;
        }
        self.record(AuditWrite {
            action: "reject",
            kind: "user",
            target: name,
            reason: &reason,
            actor_name: Some(&actor_name),
            source,
            reverses: None,
        })
    }

    /// Record an admin's rejection of a package request. The request row is the
    /// record; this does not copy comments beyond the closure reason.
    pub fn audit_request_rejection(
        &self,
        actor: i64,
        id: i64,
        pkgbase: &str,
        reason: &str,
    ) -> Result<i64> {
        let reason = clean_reason(reason).unwrap_or_else(|_| {
            let trimmed: String = reason
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(500)
                .collect();
            if trimmed.is_empty() {
                "rejected".into()
            } else {
                trimmed
            }
        });
        let actor_name = self.user_by_id(actor).ok().map(|u| u.name);
        self.record(AuditWrite {
            action: "reject",
            kind: "request",
            target: &format!("{id}:{pkgbase}"),
            reason: &reason,
            actor_name: actor_name.as_deref(),
            source: "admin",
            reverses: None,
        })
    }

    pub fn list_audit(&self, actor: i64) -> Result<Vec<AuditEvent>> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT id, at, action, target_kind, target, reason, actor_name, source, reverses FROM audit_events ORDER BY id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(AuditEvent {
                id: r.get(0)?,
                at: r.get(1)?,
                action: r.get(2)?,
                target_kind: r.get(3)?,
                target: r.get(4)?,
                reason: r.get(5)?,
                actor: r.get(6)?,
                source: r.get(7)?,
                reverses: r.get(8)?,
                open: false,
            })
        })?;
        let mut events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let reversed: std::collections::HashSet<i64> =
            events.iter().filter_map(|e| e.reverses).collect();
        for event in &mut events {
            event.open = event.action == "reject" && !reversed.contains(&event.id);
        }
        Ok(events)
    }

    /// Let the same rejected upload or registration be tried again.
    ///
    /// This does not create an account or store a recipe. The next check allows
    /// only the scope recorded on that rejection (registry, repository, package,
    /// version, recipe hashes, and applicant, or the hashed nick and email).
    /// A newer rejection of that scope, or a revocation, wins. Moderators cannot
    /// accept. Suspension still uses [`Store::reverse_audit`].
    pub fn accept_rejection(&self, actor: i64, event_id: i64, reason: &str) -> Result<i64> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        let reason = clean_reason(reason)?;
        let event = self.audit_event(event_id)?;
        if event.action != "reject" {
            return Err(Error::Conflict("only a rejection can be accepted".into()));
        }
        if !matches!(
            event.target_kind.as_str(),
            "registry_upload" | "registration"
        ) {
            return Err(Error::Conflict(
                "accept overrides a rejected upload or registration; reverse a suspension instead"
                    .into(),
            ));
        }
        let actor_name = self.user_by_id(actor)?.name;
        // One connection for the decision and the insert. A second lock here
        // deadlocks, and a gap between the two would let two accepts both insert.
        let conn = self.lock();
        let (scope, applicant): (Option<String>, Option<i64>) = conn.query_row(
            "SELECT scope_hash, applicant_id FROM audit_events WHERE id = ?1",
            [event_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let Some(scope) = scope else {
            return Err(Error::Conflict(
                "this rejection has no scope to accept".into(),
            ));
        };
        let newer: i64 = conn.query_row(
            "SELECT COUNT(*) FROM audit_events WHERE scope_hash = ?1 AND action = 'reject' AND id > ?2",
            params![scope, event_id],
            |r| r.get(0),
        )?;
        if newer > 0 {
            return Err(Error::Conflict(
                "a newer rejection of this attempt supersedes it".into(),
            ));
        }
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM audit_events WHERE action = 'accept' AND reverses = ?1 ORDER BY id DESC LIMIT 1",
                [event_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let revoked = conn
                .query_row(
                    "SELECT 1 FROM audit_events WHERE action = 'revoke' AND reverses = ?1",
                    [existing],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !revoked {
                return Ok(existing);
            }
        }
        conn.execute(
            "INSERT INTO audit_events (at, action, target_kind, target, reason, actor_name, source, reverses, scope_hash, applicant_id) VALUES (?1, 'accept', ?2, ?3, ?4, ?5, 'admin', ?6, ?7, ?8)",
            params![now(), event.target_kind, event.target, reason, actor_name, event_id, scope, applicant],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Withdraw an acceptance. The business rule applies again. The row stays.
    pub fn revoke_acceptance(&self, actor: i64, event_id: i64, reason: &str) -> Result<i64> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        let reason = clean_reason(reason)?;
        let event = self.audit_event(event_id)?;
        if event.action != "accept" {
            return Err(Error::Conflict("only an acceptance can be revoked".into()));
        }
        let actor_name = self.user_by_id(actor)?.name;
        let conn = self.lock();
        let (scope, applicant): (Option<String>, Option<i64>) = conn.query_row(
            "SELECT scope_hash, applicant_id FROM audit_events WHERE id = ?1",
            [event_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if conn
            .query_row(
                "SELECT 1 FROM audit_events WHERE action = 'revoke' AND reverses = ?1",
                [event_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "acceptance {event_id} is already revoked"
            )));
        }
        conn.execute(
            "INSERT INTO audit_events (at, action, target_kind, target, reason, actor_name, source, reverses, scope_hash, applicant_id) VALUES (?1, 'revoke', ?2, ?3, ?4, ?5, 'admin', ?6, ?7, ?8)",
            params![now(), event.target_kind, event.target, reason, actor_name, event_id, scope, applicant],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Undo one open rejection. Does not create an account or a package, and
    /// does not reopen a rejected request. Reversing an upload or registration
    /// rejection does not permit that attempt; [`Store::accept_rejection`] does.
    pub fn reverse_audit(&self, actor: i64, event_id: i64, reason: &str) -> Result<i64> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        let reason = clean_reason(reason)?;
        let event = self.audit_event(event_id)?;
        if event.action != "reject" {
            return Err(Error::Conflict("only a rejection can be reversed".into()));
        }
        if matches!(
            event.target_kind.as_str(),
            "request" | "registration" | "registry_upload"
        ) {
            let actor_name = self.user_by_id(actor)?.name;
            if self
                .lock()
                .query_row(
                    "SELECT 1 FROM audit_events WHERE reverses = ?1",
                    [event_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some()
            {
                return Err(Error::Conflict(format!(
                    "rejection {event_id} is already reversed"
                )));
            }
            return self.record(AuditWrite {
                action: "reverse",
                kind: &event.target_kind,
                target: &event.target,
                reason: &reason,
                actor_name: Some(&actor_name),
                source: "admin",
                reverses: Some(event_id),
            });
        }
        if self
            .lock()
            .query_row(
                "SELECT 1 FROM audit_events WHERE reverses = ?1",
                [event_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "rejection {event_id} is already reversed"
            )));
        }
        let newer_open: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM audit_events r WHERE r.action = 'reject' AND r.target_kind = ?1 AND r.target = ?2 AND r.id != ?3 AND NOT EXISTS (SELECT 1 FROM audit_events x WHERE x.reverses = r.id)",
            params![event.target_kind, event.target, event_id],
            |r| r.get(0),
        )?;
        let actor_name = self.user_by_id(actor)?.name;
        if newer_open == 0 {
            match event.target_kind.as_str() {
                "package" => {
                    self.lock().execute(
                        "UPDATE pkgbases SET suspended_at = NULL, suspended_reason = NULL WHERE name = ?1",
                        [&event.target],
                    )?;
                }
                "user" => {
                    if let Some(user) = self.user_by_name(&event.target)? {
                        self.lock().execute(
                            "UPDATE users SET suspended_at = NULL, suspended_reason = NULL WHERE id = ?1",
                            [user.id],
                        )?;
                    }
                }
                _ => {
                    return Err(Error::Conflict(
                        "this rejection has nothing to restore".into(),
                    ));
                }
            }
        }
        self.record(AuditWrite {
            action: "reverse",
            kind: &event.target_kind,
            target: &event.target,
            reason: &reason,
            actor_name: Some(&actor_name),
            source: "admin",
            reverses: Some(event_id),
        })
    }

    fn require_moderator(&self, actor: i64) -> Result<()> {
        self.ensure_not_suspended(actor)?;
        if self.is_admin(actor)? || self.is_moderator(actor)? {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }

    fn actor_source(&self, actor: i64) -> Result<&'static str> {
        if self.is_admin(actor)? {
            Ok("admin")
        } else {
            Ok("moderator")
        }
    }

    fn record(&self, event: AuditWrite<'_>) -> Result<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO audit_events (at, action, target_kind, target, reason, actor_name, source, reverses) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                now(),
                event.action,
                event.kind,
                event.target,
                event.reason,
                event.actor_name,
                event.source,
                event.reverses
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn audit_event(&self, id: i64) -> Result<AuditEvent> {
        self.lock()
            .query_row(
                "SELECT id, at, action, target_kind, target, reason, actor_name, source, reverses FROM audit_events WHERE id = ?1",
                [id],
                |r| {
                    Ok(AuditEvent {
                        id: r.get(0)?,
                        at: r.get(1)?,
                        action: r.get(2)?,
                        target_kind: r.get(3)?,
                        target: r.get(4)?,
                        reason: r.get(5)?,
                        actor: r.get(6)?,
                        source: r.get(7)?,
                        reverses: r.get(8)?,
                        open: false,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("audit event {id}")))
    }
}
