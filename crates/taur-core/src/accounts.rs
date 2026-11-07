//! User accounts: registration, login sessions, logout, unregistering.
//!
//! Passwords are hashed with Argon2id (PHC string format). Session tokens are 32
//! random bytes (hex) handed to the client once; only their SHA-256 is stored.

use crate::{Error, Result, Store, hex, now};
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rand_core::{OsRng, RngCore};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Session lifetime: 30 days.
pub const SESSION_TTL_SECS: i64 = 30 * 24 * 3600;
pub const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub verified: bool,
    pub is_admin: bool,
    pub is_moderator: bool,
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
    })
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex(&bytes)
}

/// A freshly issued session. Deliberately not `Debug` so the token is not logged by accident.
#[derive(Serialize)]
pub struct Session {
    pub token: String,
    pub expires_at: i64,
}

/// AUR-style user names: 3-16 chars of `[A-Za-z0-9._-]`, starting and ending alphanumeric.
pub fn validate_username(name: &str) -> Result<()> {
    let ok = (3..=16).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name.ends_with(|c: char| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(
            "user name must be 3-16 characters of A-Z a-z 0-9 . _ - and start/end alphanumeric"
                .into(),
        ))
    }
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| Error::Hash)
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

impl Store {
    pub fn register(&self, name: &str, email: &str, password: &str) -> Result<User> {
        validate_username(name)?;
        let email = email.trim();
        if !email.contains('@') || email.len() > 254 {
            return Err(Error::Invalid("invalid email address".into()));
        }
        if password.chars().count() < MIN_PASSWORD_LEN {
            return Err(Error::Invalid(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }
        self.guard_registration(name, email)?;
        let hash = hash_password(password)?;
        let conn = self.lock();
        let taken: Option<i64> = conn
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
        conn.execute(
            "INSERT INTO users (name, email, password_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![name, email, hash, now()],
        )?;
        let user = User {
            id: conn.last_insert_rowid(),
            name: name.to_string(),
            email: email.to_string(),
            verified: false,
            is_admin: false,
            is_moderator: false,
        };
        drop(conn);
        self.bind_registered_user(user.id, &user.name, &user.email)?;
        // A delivery failure doesn't undo registration; the user can ask for a resend.
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
                "SELECT id, name, email, verified_at, is_admin, is_moderator FROM users WHERE email = ?1",
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
        if new_password.chars().count() < MIN_PASSWORD_LEN {
            return Err(Error::Invalid(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }
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
                "SELECT id, name, email, verified_at, is_admin, is_moderator FROM users WHERE id = ?1",
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
                "SELECT id, name, email, verified_at, is_admin, is_moderator, password_hash FROM users WHERE name = ?1",
                [name],
                |r| Ok((user_row(r)?, r.get(6)?)),
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
                "SELECT u.id, u.name, u.email, u.verified_at, u.is_admin, u.is_moderator FROM sessions s JOIN users u ON u.id = s.user_id \
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
}
