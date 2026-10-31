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
        Ok(User {
            id: conn.last_insert_rowid(),
            name: name.to_string(),
            email: email.to_string(),
        })
    }

    fn credentials(&self, name: &str) -> Result<Option<(User, String)>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT id, name, email, password_hash FROM users WHERE name = ?1",
                [name],
                |r| {
                    Ok((
                        User {
                            id: r.get(0)?,
                            name: r.get(1)?,
                            email: r.get(2)?,
                        },
                        r.get(3)?,
                    ))
                },
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
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let token = hex(&bytes);
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
        self.lock()
            .query_row(
                "SELECT u.id, u.name, u.email FROM sessions s JOIN users u ON u.id = s.user_id \
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2",
                params![token_hash(token), now()],
                |r| {
                    Ok(User {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        email: r.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or(Error::Unauthorized)
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

    pub fn user_by_name(&self, name: &str) -> Result<Option<User>> {
        Ok(self.credentials(name)?.map(|(u, _)| u))
    }
}
