//! User accounts: registration, login sessions, logout, unregistering.
//!
//! Passwords are hashed with Argon2id (PHC string format). Session tokens are 32
//! random bytes (hex) handed to the client once; only their SHA-256 is stored.

use crate::{Error, Result, Store, now};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use weirdnet_accounts::names::{NamePolicy, validate_email};
use weirdnet_accounts::password::{check_length, hash_password as wa_hash, verify_password};
use weirdnet_accounts::token::{random_token, token_hash};

/// Session lifetime: 30 days.
pub const SESSION_TTL_SECS: i64 = 30 * 24 * 3600;
pub const MIN_PASSWORD_LEN: usize = weirdnet_accounts::password::MIN_PASSWORD_LEN;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub verified: bool,
    pub is_admin: bool,
    pub is_moderator: bool,
    pub is_owner: bool,
}

/// Email verification links are valid for 24 hours; password reset links for 1 hour.
pub const VERIFY_TTL_SECS: i64 = 24 * 3600;
pub const RESET_TTL_SECS: i64 = 3600;

fn user_row(r: &rusqlite::Row) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        name: r.get(1)?,
        email: r.get(2)?,
        verified: r.get::<_, Option<i64>>(3)?.is_some(),
        is_admin: r.get::<_, i64>(4)? != 0,
        is_moderator: r.get::<_, i64>(5)? != 0,
        is_owner: r.get::<_, i64>(6)? != 0,
    })
}

/// A freshly issued session. Deliberately not `Debug` so the token is not logged by accident.
#[derive(Serialize)]
pub struct Session {
    pub token: String,
    pub expires_at: i64,
}

/// AUR-style user names: 3-16 chars of `[A-Za-z0-9._-]`, starting and ending alphanumeric.
pub fn validate_username(name: &str) -> Result<()> {
    NamePolicy::AUR.validate(name).map_err(map_err)
}

/// Map a `weirdnet-accounts` error onto TAUR\'s.
pub(crate) fn map_err(e: weirdnet_accounts::Error) -> Error {
    match e {
        weirdnet_accounts::Error::Invalid(m) => Error::Invalid(m),
        weirdnet_accounts::Error::Hash => Error::Hash,
        weirdnet_accounts::Error::RateLimited { retry_after } => Error::RateLimited { retry_after },
        weirdnet_accounts::Error::ApiKey(_) => Error::Unauthorized,
    }
}

fn hash_password(password: &str) -> Result<String> {
    wa_hash(password).map_err(map_err)
}

impl Store {
    pub fn register(&self, name: &str, email: &str, password: &str) -> Result<User> {
        validate_username(name)?;
        let email = validate_email(email).map_err(map_err)?;
        check_length(password, MIN_PASSWORD_LEN).map_err(map_err)?;
        self.guard_registration(name, email)?;
        let hash = hash_password(password)?;
        let created = now();
        let mut conn = self.lock();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(Error::from)?;
        let taken: Option<i64> = tx
            .query_row(
                "SELECT id FROM users WHERE name = ?1 OR email = ?2",
                params![name, email],
                |r| r.get(0),
            )
            .optional()?;
        if taken.is_some() {
            return Err(Error::Conflict(
                "user name or email already registered".into(),
            ));
        }
        let privileged: i64 = tx.query_row(
            "SELECT COUNT(*) FROM users WHERE is_admin != 0 OR is_owner != 0",
            [],
            |r| r.get(0),
        )?;
        // One winner: the write lock is held until commit. TAUR_ADMINS,
        // TAUR_OWNERS, `taur admin`, and `taur owner` leave a privileged
        // account or reserve the slot, and then this stays off.
        let bootstrap = privileged == 0 && !self.admin_bootstrap_reserved();
        if bootstrap {
            tx.execute(
                "INSERT INTO users (name, email, password_hash, created_at, verified_at, is_admin, is_owner) VALUES (?1, ?2, ?3, ?4, ?5, 1, 1)",
                params![name, email, hash, created, created],
            )?;
            tx.execute(
                "INSERT INTO audit_events (at, action, target_kind, target, reason, actor_name, source, reverses) VALUES (?1, 'grant', 'user', ?2, ?3, ?2, 'bootstrap', NULL)",
                params![
                    created,
                    name,
                    "first account; no site owner or admin existed and TAUR_ADMINS and TAUR_OWNERS were not set"
                ],
            )?;
        } else {
            tx.execute(
                "INSERT INTO users (name, email, password_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![name, email, hash, created],
            )?;
        }
        let user = User {
            id: tx.last_insert_rowid(),
            name: name.to_string(),
            email: email.to_string(),
            verified: bootstrap,
            is_admin: bootstrap,
            is_moderator: false,
            is_owner: bootstrap,
        };
        tx.commit()?;
        drop(conn);
        self.bind_registered_user(user.id, &user.name, &user.email)?;
        // A delivery failure doesn't undo registration; the user can ask for a resend.
        // The bootstrap admin is already verified, because a new server often has no SMTP.
        if let Err(e) = self.send_verification(&user) {
            eprintln!(
                "taur: could not send verification mail to user {}: {e}",
                user.name
            );
        }
        Ok(user)
    }

    fn issue_email_token(&self, user: i64, purpose: &str, ttl: i64) -> Result<String> {
        let token = random_token();
        let conn = self.lock();
        conn.execute(
            "DELETE FROM email_tokens WHERE (user_id = ?1 AND purpose = ?2) OR expires_at <= ?3",
            params![user, purpose, now()],
        )?;
        conn.execute(
            "INSERT INTO email_tokens (token_hash, user_id, purpose, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash(&token), user, purpose, now() + ttl],
        )?;
        Ok(token)
    }

    /// Consume a single-use email token; returns its user id.
    fn take_email_token(&self, token: &str, purpose: &str) -> Result<i64> {
        let conn = self.lock();
        let h = token_hash(token);
        let uid: Option<i64> = conn
            .query_row(
                "SELECT user_id FROM email_tokens WHERE token_hash = ?1 AND purpose = ?2 AND expires_at > ?3",
                params![h, purpose, now()],
                |r| r.get(0),
            )
            .optional()?;
        conn.execute("DELETE FROM email_tokens WHERE token_hash = ?1", [h])?;
        uid.ok_or_else(|| Error::Invalid("invalid or expired token".into()))
    }

    fn send_verification(&self, user: &User) -> Result<()> {
        let token = self.issue_email_token(user.id, "verify", VERIFY_TTL_SECS)?;
        let base = self.base_url();
        let body = format!(
            "Hi {},\n\nConfirm your TAUR account email address:\n\n  {base}/verify?token={token}\n\nor run: taurctl verify <token from the link>\n\nThe link expires in 24 hours. If you didn't register, ignore this message.\n",
            user.name
        );
        self.mailer()
            .send(&user.email, "Verify your TAUR account", &body)
    }

    /// Re-send the verification email.
    pub fn resend_verification(&self, user: &User) -> Result<()> {
        if user.verified {
            return Err(Error::Conflict("email address already verified".into()));
        }
        self.send_verification(user)
    }

    /// Confirm an email address with a token from the verification mail.
    pub fn verify_email(&self, token: &str) -> Result<User> {
        let uid = self.take_email_token(token, "verify")?;
        self.mark_verified(uid)?;
        self.user_by_id(uid)
    }

    /// Mark an account verified (admin/maintenance use).
    pub fn mark_verified(&self, user: i64) -> Result<()> {
        self.lock().execute(
            "UPDATE users SET verified_at = COALESCE(verified_at, ?1) WHERE id = ?2",
            params![now(), user],
        )?;
        Ok(())
    }

    /// Start a password reset. Always succeeds so callers can't probe for accounts.
    pub fn request_password_reset(&self, email: &str) -> Result<()> {
        let user = self
            .lock()
            .query_row(
                "SELECT id, name, email, verified_at, is_admin, is_moderator, is_owner FROM users WHERE email = ?1",
                [email.trim()],
                user_row,
            )
            .optional()?;
        if let Some(user) = user {
            let token = self.issue_email_token(user.id, "reset", RESET_TTL_SECS)?;
            let base = self.base_url();
            let body = format!(
                "Hi {},\n\nSomeone asked to reset your TAUR password. To choose a new one, open:\n\n  {base}/reset?token={token}\n\nor run: taurctl reset-password confirm --token <token from the link>\n\nThe link expires in 1 hour and works once. If this wasn't you, ignore this message.\n",
                user.name
            );
            if let Err(e) = self
                .mailer()
                .send(&user.email, "Reset your TAUR password", &body)
            {
                eprintln!("taur: could not send reset mail to user {}: {e}", user.name);
            }
        }
        Ok(())
    }

    /// Finish a password reset: sets the password, ends every session. Resetting via
    /// the emailed link also proves the address, so the account becomes verified.
    pub fn reset_password(&self, token: &str, new_password: &str) -> Result<()> {
        check_length(new_password, MIN_PASSWORD_LEN).map_err(map_err)?;
        let uid = self.take_email_token(token, "reset")?;
        let hash = hash_password(new_password)?;
        let conn = self.lock();
        conn.execute(
            "UPDATE users SET password_hash = ?1, verified_at = COALESCE(verified_at, ?2) WHERE id = ?3",
            params![hash, now(), uid],
        )?;
        conn.execute("DELETE FROM sessions WHERE user_id = ?1", [uid])?;
        conn.execute(
            "DELETE FROM email_tokens WHERE user_id = ?1 AND purpose = 'reset'",
            [uid],
        )?;
        Ok(())
    }

    pub fn user_by_id(&self, id: i64) -> Result<User> {
        self.lock()
            .query_row(
                "SELECT id, name, email, verified_at, is_admin, is_moderator, is_owner FROM users WHERE id = ?1",
                [id],
                user_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound("user".into()))
    }

    fn credentials(&self, name: &str) -> Result<Option<(User, String)>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT id, name, email, verified_at, is_admin, is_moderator, is_owner, password_hash FROM users WHERE name = ?1",
                [name],
                |r| Ok((user_row(r)?, r.get(7)?)),
            )
            .optional()?)
    }

    /// Verify credentials and issue a new session token.
    pub fn login(&self, name: &str, password: &str) -> Result<Session> {
        let Some((user, hash)) = self.credentials(name)? else {
            return Err(Error::Unauthorized);
        };
        if !verify_password(password, &hash) {
            return Err(Error::Unauthorized);
        }
        self.ensure_not_suspended(user.id)?;
        let token = random_token();
        let now = now();
        let expires_at = now + SESSION_TTL_SECS;
        let conn = self.lock();
        conn.execute("DELETE FROM sessions WHERE expires_at <= ?1", [now])?;
        conn.execute(
            "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash(&token), user.id, now, expires_at],
        )?;
        Ok(Session { token, expires_at })
    }

    /// The user owning a live session token.
    pub fn user_for_token(&self, token: &str) -> Result<User> {
        let user = self
            .lock()
            .query_row(
                "SELECT u.id, u.name, u.email, u.verified_at, u.is_admin, u.is_moderator, u.is_owner FROM sessions s JOIN users u ON u.id = s.user_id \
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2",
                params![token_hash(token), now()],
                user_row,
            )
            .optional()?
            .ok_or(Error::Unauthorized)?;
        self.ensure_not_suspended(user.id)?;
        Ok(user)
    }

    /// End a session. Unknown tokens are rejected so clients notice stale state.
    pub fn logout(&self, token: &str) -> Result<()> {
        let n = self.lock().execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            [token_hash(token)],
        )?;
        if n == 0 {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    /// Delete the account behind `token` after re-checking the password.
    ///
    /// Sessions, org memberships and co-maintainerships go with it; package bases it
    /// maintained become orphans (maintainer cleared), like AUR account deletion.
    /// Refused while the user is the only Owner of an organization.
    pub fn unregister(&self, token: &str, password: &str) -> Result<()> {
        let user = self.user_for_token(token)?;
        let Some((_, hash)) = self.credentials(&user.name)? else {
            return Err(Error::Unauthorized);
        };
        if !verify_password(password, &hash) {
            return Err(Error::Unauthorized);
        }
        let conn = self.lock();
        let sole: Option<String> = conn
            .query_row(
                "SELECT o.name FROM orgs o JOIN org_members m ON m.org_id = o.id \
                 WHERE m.user_id = ?1 AND m.role = 'owner' \
                 AND (SELECT COUNT(*) FROM org_members x WHERE x.org_id = o.id AND x.role = 'owner') = 1 LIMIT 1",
                [user.id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(org) = sole {
            return Err(Error::Conflict(format!(
                "you are the only owner of organization {org}; transfer ownership or delete it first"
            )));
        }
        conn.execute("DELETE FROM users WHERE id = ?1", [user.id])?;
        Ok(())
    }

    /// Unverified accounts can sign in but can't push or manage anything.
    pub fn require_verified(&self, user: &User) -> Result<()> {
        if user.verified {
            Ok(())
        } else {
            Err(Error::Unverified)
        }
    }

    pub fn user_by_name(&self, name: &str) -> Result<Option<User>> {
        Ok(self.credentials(name)?.map(|(u, _)| u))
    }

    fn require_admin(&self, actor: i64) -> Result<()> {
        if self.is_admin(actor)? {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }

    fn require_staff(&self, actor: i64) -> Result<bool> {
        let admin = self.is_admin(actor)?;
        if admin || self.is_moderator(actor)? {
            Ok(admin)
        } else {
            Err(Error::Forbidden)
        }
    }

    /// Accounts a moderator or site admin can see. Email addresses are included
    /// only for site admins.
    pub fn search_users(&self, actor: i64, query: &str) -> Result<Vec<DirectoryUser>> {
        let admin = self.require_staff(actor)?;
        let q = query.trim();
        let conn = self.lock();
        let mut sql = String::from(
            "SELECT id, name, email, verified_at, is_admin, is_moderator, is_owner, suspended_at FROM users",
        );
        let like = if q.is_empty() {
            sql.push_str(" ORDER BY name LIMIT 50");
            None
        } else {
            sql.push_str(" WHERE name LIKE ?1 ESCAPE '\\' ORDER BY name LIMIT 50");
            let escaped = q
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            Some(format!("%{escaped}%"))
        };
        let mut st = conn.prepare(&sql)?;
        let map = |r: &rusqlite::Row| {
            let email: String = r.get(2)?;
            Ok(DirectoryUser {
                name: r.get(1)?,
                email: if admin { Some(email) } else { None },
                verified: r.get::<_, Option<i64>>(3)?.is_some(),
                is_admin: r.get::<_, i64>(4)? != 0,
                is_moderator: r.get::<_, i64>(5)? != 0,
                is_owner: r.get::<_, i64>(6)? != 0,
                suspended: r.get::<_, Option<i64>>(7)?.is_some(),
            })
        };
        let rows = if let Some(like) = like.as_deref() {
            st.query_map([like], map)?
        } else {
            st.query_map([], map)?
        };
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Mark an account verified. Site admin only. Same effect as `taur verify-user`.
    pub fn admin_verify_user(&self, actor: i64, name: &str) -> Result<()> {
        self.require_admin(actor)?;
        let user = self
            .user_by_name(name)?
            .ok_or_else(|| Error::NotFound(format!("user {name}")))?;
        self.mark_verified(user.id)
    }

    /// Send that account's verification mail again. Site admin only.
    pub fn admin_resend_verification(&self, actor: i64, name: &str) -> Result<()> {
        self.require_admin(actor)?;
        let user = self
            .user_by_name(name)?
            .ok_or_else(|| Error::NotFound(format!("user {name}")))?;
        self.resend_verification(&user)
    }

    /// Email a password-reset link. The token is not returned.
    pub fn admin_request_password_reset(&self, actor: i64, name: &str) -> Result<()> {
        self.require_admin(actor)?;
        let user = self
            .user_by_name(name)?
            .ok_or_else(|| Error::NotFound(format!("user {name}")))?;
        let token = self.issue_email_token(user.id, "reset", RESET_TTL_SECS)?;
        let base = self.base_url();
        let body = format!(
            "Hi {},\n\nA site admin asked to reset your TAUR password. To choose a new one, open:\n\n  {base}/reset?token={token}\n\nor run: taurctl reset-password confirm --token <token from the link>\n\nThe link expires in 1 hour and works once. If this wasn't you, ignore this message.\n",
            user.name
        );
        self.mailer()
            .send(&user.email, "Reset your TAUR password", &body)
            .map_err(|e| Error::Mail(format!("password reset: {e}")))
    }

    pub fn list_sessions(&self, user: i64, current_token: &str) -> Result<Vec<SessionInfo>> {
        let current = token_hash(current_token);
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT rowid, created_at, expires_at, token_hash FROM sessions WHERE user_id = ?1 AND expires_at > ?2 ORDER BY created_at",
        )?;
        let rows = st.query_map(params![user, now()], |r| {
            let hash: String = r.get(3)?;
            Ok(SessionInfo {
                id: r.get(0)?,
                created_at: r.get(1)?,
                expires_at: r.get(2)?,
                current: hash == current,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// End one of your own sessions. The token itself is not shown.
    pub fn revoke_session(&self, user: i64, session_id: i64) -> Result<()> {
        let n = self.lock().execute(
            "DELETE FROM sessions WHERE rowid = ?1 AND user_id = ?2",
            params![session_id, user],
        )?;
        if n == 0 {
            return Err(Error::NotFound("session".into()));
        }
        Ok(())
    }

    /// End every session for an account. Site admin only. Moderators cannot.
    pub fn revoke_user_sessions(&self, actor: i64, name: &str) -> Result<()> {
        self.require_admin(actor)?;
        let user = self
            .user_by_name(name)?
            .ok_or_else(|| Error::NotFound(format!("user {name}")))?;
        self.lock()
            .execute("DELETE FROM sessions WHERE user_id = ?1", [user.id])?;
        Ok(())
    }
}

/// A directory row. `email` is omitted unless the viewer is a site admin.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DirectoryUser {
    pub name: String,
    pub email: Option<String>,
    pub verified: bool,
    pub is_admin: bool,
    pub is_moderator: bool,
    pub is_owner: bool,
    pub suspended: bool,
}

/// A session without its token.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionInfo {
    pub id: i64,
    pub created_at: i64,
    pub expires_at: i64,
    pub current: bool,
}
