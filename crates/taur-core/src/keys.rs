//! SSH public keys for git pushes over SSH (identified by SHA-256 fingerprint).

use crate::{Error, Result, Store, accounts::User, now};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SshKey {
    pub id: i64,
    pub fingerprint: String,
    pub algorithm: String,
    pub comment: String,
    pub created_at: i64,
}

/// Parse an OpenSSH public key line; returns `(fingerprint, algorithm, canonical line, comment)`.
pub fn parse_public_key(line: &str) -> Result<(String, String, String, String)> {
    let key = ssh_key::PublicKey::from_openssh(line.trim())
        .map_err(|_| Error::Invalid("not a valid OpenSSH public key".into()))?;
    let fp = key.fingerprint(ssh_key::HashAlg::Sha256).to_string();
    let alg = key.algorithm().to_string();
    let comment = key.comment().to_string();
    let canonical = key
        .to_openssh()
        .map_err(|_| Error::Invalid("cannot encode key".into()))?;
    Ok((fp, alg, canonical, comment))
}

impl Store {
    pub fn add_ssh_key(&self, user: i64, line: &str) -> Result<SshKey> {
        let (fp, alg, canonical, comment) = parse_public_key(line)?;
        let conn = self.lock();
        let taken: Option<i64> = conn
            .query_row(
                "SELECT id FROM ssh_keys WHERE fingerprint = ?1",
                [&fp],
                |r| r.get(0),
            )
            .optional()?;
        if taken.is_some() {
            return Err(Error::Conflict("this key is already registered".into()));
        }
        let created_at = now();
        conn.execute(
            "INSERT INTO ssh_keys (user_id, fingerprint, algorithm, public_key, comment, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![user, fp, alg, canonical, comment, created_at],
        )?;
        Ok(SshKey {
            id: conn.last_insert_rowid(),
            fingerprint: fp,
            algorithm: alg,
            comment,
            created_at,
        })
    }

    pub fn list_ssh_keys(&self, user: i64) -> Result<Vec<SshKey>> {
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT id, fingerprint, algorithm, comment, created_at FROM ssh_keys WHERE user_id = ?1 ORDER BY id",
        )?;
        let rows = st.query_map([user], |r| {
            Ok(SshKey {
                id: r.get(0)?,
                fingerprint: r.get(1)?,
                algorithm: r.get(2)?,
                comment: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn remove_ssh_key(&self, user: i64, id: i64) -> Result<()> {
        let n = self.lock().execute(
            "DELETE FROM ssh_keys WHERE id = ?1 AND user_id = ?2",
            params![id, user],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("ssh key {id}")));
        }
        Ok(())
    }

    /// Account and canonical OpenSSH line for an offered `ssh-ed25519 AAAA…` key.
    /// An unparseable line is "no such key", not an error.
    pub fn user_for_openssh(&self, line: &str) -> Result<Option<(User, String)>> {
        let (fp, _, canonical, _) = match parse_public_key(line) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        Ok(self.user_for_fingerprint(&fp)?.map(|u| (u, canonical)))
    }

    /// The account owning a key fingerprint (`SHA256:...`).
    pub fn user_for_fingerprint(&self, fp: &str) -> Result<Option<User>> {
        let name: Option<String> = self
            .lock()
            .query_row(
                "SELECT u.name FROM ssh_keys k JOIN users u ON u.id = k.user_id WHERE k.fingerprint = ?1",
                [fp],
                |r| r.get(0),
            )
            .optional()?;
        match name {
            Some(n) => self.user_by_name(&n),
            None => Ok(None),
        }
    }
}
