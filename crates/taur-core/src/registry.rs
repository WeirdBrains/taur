//! Package registries and operator recipe signatures.
//!
//! The built-in `local` registry is exempt: packages already in TAUR, and new
//! packages that do not name another registry, keep the previous upload rules.
//! A registry an administrator creates afterwards is enforcing. Its keys,
//! identities, and expected recipes are operator attestations. They are not an
//! Arch or AUR statement. The public AUR RPC does not provide a maintainer
//! email or a signature over the recipe, and a pacman archive signature does
//! not cover a PKGBUILD.

use crate::{Error, Result, Store, now};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::{OsRng, RngCore};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

/// Registry that holds every package base created before package registries,
/// and every upload that does not name another registry.
pub const LEGACY_REGISTRY: &str = "local";
/// Stored on every key, identity, and expectation this server records.
pub const PROVENANCE_OPERATOR: &str = "operator";

const ENFORCE: &str = "enforce";
const EXEMPT: &str = "exempt";
/// A recipe mirror. Not an operator trust record and not a pacman repository.
pub const MIRROR_ENFORCEMENT: &str = "mirror";

/// Bytes that an Ed25519 signature covers. Registry, repository, package, and
/// version are inside the message, so a signature cannot be moved onto another
/// package, repository, or registry.
pub fn recipe_message(
    registry: &str,
    repository: &str,
    pkgbase: &str,
    version: &str,
    srcinfo: &[u8],
    pkgbuild: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"TAUR-RECIPE-SIG\0v1\0");
    for part in [
        registry.as_bytes(),
        repository.as_bytes(),
        pkgbase.as_bytes(),
        version.as_bytes(),
    ] {
        out.extend_from_slice(part);
        out.push(0);
    }
    out.extend_from_slice(&(srcinfo.len() as u64).to_be_bytes());
    out.extend_from_slice(srcinfo);
    out.extend_from_slice(&(pkgbuild.len() as u64).to_be_bytes());
    out.extend_from_slice(pkgbuild);
    out
}

pub fn fingerprint(public_key: &[u8]) -> String {
    hex_encode(&Sha256::digest(public_key))
}

pub fn generate_key() -> (SigningKey, VerifyingKey) {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let signing = SigningKey::from_bytes(&seed);
    let verifying = signing.verifying_key();
    (signing, verifying)
}

pub fn sign_recipe(
    secret_seed: &[u8; 32],
    registry: &str,
    repository: &str,
    pkgbase: &str,
    version: &str,
    srcinfo: &[u8],
    pkgbuild: &[u8],
) -> [u8; 64] {
    let key = SigningKey::from_bytes(secret_seed);
    let msg = recipe_message(registry, repository, pkgbase, version, srcinfo, pkgbuild);
    key.sign(&msg).to_bytes()
}

#[allow(clippy::too_many_arguments)]
pub fn verify_recipe(
    public_key: &[u8],
    signature: &[u8],
    registry: &str,
    repository: &str,
    pkgbase: &str,
    version: &str,
    srcinfo: &[u8],
    pkgbuild: &[u8],
) -> bool {
    let Ok(key_bytes) = <[u8; 32]>::try_from(public_key) else {
        return false;
    };
    let Ok(sig_bytes) = <[u8; 64]>::try_from(signature) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&key_bytes) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(&sig_bytes) else {
        return false;
    };
    let msg = recipe_message(registry, repository, pkgbase, version, srcinfo, pkgbuild);
    key.verify_strict(&msg, &sig).is_ok()
}

/// `Ordering::Greater` when `left` is a newer `[epoch:]pkgver-pkgrel` than `right`.
pub fn cmp_version(left: &str, right: &str) -> Ordering {
    let (le, lv, lr) = split_version(left);
    let (re, rv, rr) = split_version(right);
    le.cmp(&re)
        .then_with(|| cmp_pkgver(lv, rv))
        .then_with(|| cmp_pkgver(lr, rr))
}

fn split_version(v: &str) -> (u64, &str, &str) {
    let (epoch, rest) = match v.split_once(':') {
        Some((e, r)) => (e.parse::<u64>().unwrap_or(0), r),
        None => (0, v),
    };
    let (pkgver, pkgrel) = rest.rsplit_once('-').unwrap_or((rest, "0"));
    (epoch, pkgver, pkgrel)
}

fn cmp_pkgver(left: &str, right: &str) -> Ordering {
    let ls: Vec<&str> = left.split(['.', '_']).collect();
    let rs: Vec<&str> = right.split(['.', '_']).collect();
    for (l, r) in ls.iter().zip(rs.iter()) {
        let ord = match (l.parse::<u64>(), r.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            _ => l.cmp(r),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    ls.len().cmp(&rs.len())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

pub fn parse_hex_key(text: &str, len: usize) -> Result<Vec<u8>> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if text.len() != len * 2 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Invalid(format!(
            "expected {len} bytes of hex ({len}×2 characters)"
        )));
    }
    let mut out = Vec::with_capacity(len);
    let bytes = text.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let h = |c: u8| match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 0,
        };
        out.push((h(bytes[i]) << 4) | h(bytes[i + 1]));
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RegistryInfo {
    pub name: String,
    pub enforcement: String,
    /// `operator` for local and enforcing registries. `mirror` for a recipe mirror.
    pub provenance: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadDecision {
    /// Legacy registry, or a name the enforcing registry does not list.
    ExistingRules,
    /// Signature, version, and repository match the operator expectation.
    Exact,
    /// Newer version, trusted signature, and the authenticated account is the bound identity.
    NewVersion,
    /// An admin accepted this exact rejected attempt. The signature or identity rule is not applied again.
    Overridden,
}

pub struct ReviewedPush {
    pub decision: UploadDecision,
    pub registry: String,
    pub repository: Option<String>,
    pub signature: Option<Vec<u8>>,
}

pub struct RecipeUpload<'a> {
    pub registry: &'a str,
    pub repository: Option<&'a str>,
    pub pkgbase: &'a str,
    pub version: &'a str,
    pub srcinfo: &'a [u8],
    pub pkgbuild: &'a [u8],
    pub signature: Option<&'a [u8]>,
    pub actor: Option<i64>,
}

struct Expectation {
    repository: String,
    version: String,
    signature: Vec<u8>,
    nick: String,
    email: String,
    user_id: Option<i64>,
}

pub fn migrate(conn: &rusqlite::Connection) -> Result<()> {
    let has: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('pkgbases') WHERE name = 'registry_id'")?
        .exists([])?;
    if !has {
        conn.execute_batch(
            "ALTER TABLE pkgbases ADD COLUMN registry_id INTEGER REFERENCES registries(id)",
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO registries (name, enforcement, created_at) VALUES (?1, ?2, ?3)",
        params![LEGACY_REGISTRY, EXEMPT, now()],
    )?;
    conn.execute(
        "UPDATE pkgbases SET registry_id = (SELECT id FROM registries WHERE name = ?1) WHERE registry_id IS NULL",
        [LEGACY_REGISTRY],
    )?;
    for (col, ty) in [("scope_hash", "TEXT"), ("applicant_id", "INTEGER")] {
        let exists = conn
            .prepare(&format!(
                "SELECT 1 FROM pragma_table_info('audit_events') WHERE name = '{col}'"
            ))?
            .exists([])?;
        if !exists {
            conn.execute_batch(&format!("ALTER TABLE audit_events ADD COLUMN {col} {ty}"))?;
        }
    }
    Ok(())
}

/// Identity of one rejected upload. A later admin acceptance matches only this.
pub fn upload_scope(upload: &RecipeUpload<'_>) -> String {
    let src = fingerprint(upload.srcinfo);
    let pkg = fingerprint(upload.pkgbuild);
    let applicant = upload
        .actor
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".into());
    let repository = upload.repository.unwrap_or("");
    fingerprint(
        format!(
            "upload\0{}\0{repository}\0{}\0{}\0{src}\0{pkg}\0{applicant}",
            upload.registry, upload.pkgbase, upload.version
        )
        .as_bytes(),
    )
}

/// Identity of one rejected registration claim. The email is hashed, not stored.
/// The registry is the one whose operator identity conflicted; another registry's
/// identity is a different claim.
pub fn registration_scope(registry: &str, nick: &str, email: &str) -> String {
    let email_hash = fingerprint(email.as_bytes());
    fingerprint(format!("register\0{registry}\0{nick}\0{email_hash}").as_bytes())
}

impl Store {
    pub fn create_registry(&self, actor: i64, name: &str) -> Result<()> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        self.insert_registry(name, ENFORCE)
    }

    pub fn insert_registry(&self, name: &str, enforcement: &str) -> Result<()> {
        let name = clean_registry_name(name)?;
        if name == LEGACY_REGISTRY {
            return Err(Error::Conflict(format!(
                "{LEGACY_REGISTRY} is the existing package registry and cannot be recreated"
            )));
        }
        if enforcement != ENFORCE {
            return Err(Error::Invalid(
                "a new package registry is enforcing; the existing local registry stays exempt"
                    .into(),
            ));
        }
        self.lock().execute(
            "INSERT INTO registries (name, enforcement, created_at) VALUES (?1, ?2, ?3)",
            params![name, ENFORCE, now()],
        )?;
        Ok(())
    }

    pub fn list_registries(&self) -> Result<Vec<RegistryInfo>> {
        let conn = self.lock();
        let mut st = conn.prepare("SELECT name, enforcement FROM registries ORDER BY id")?;
        let rows = st.query_map([], |r| {
            let enforcement: String = r.get(1)?;
            let provenance = if enforcement == MIRROR_ENFORCEMENT {
                crate::mirror::PROVENANCE_MIRROR
            } else {
                PROVENANCE_OPERATOR
            };
            Ok(RegistryInfo {
                name: r.get(0)?,
                enforcement,
                provenance: provenance.to_string(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn add_registry_key(
        &self,
        actor: i64,
        registry: &str,
        label: &str,
        public_key: &[u8],
    ) -> Result<String> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        self.insert_registry_key(registry, label, public_key)
    }

    pub fn insert_registry_key(
        &self,
        registry: &str,
        label: &str,
        public_key: &[u8],
    ) -> Result<String> {
        self.require_enforcing(registry)?;
        if public_key.len() != 32
            || VerifyingKey::from_bytes(public_key.try_into().unwrap()).is_err()
        {
            return Err(Error::Invalid(
                "registry key must be a 32-byte Ed25519 public key".into(),
            ));
        }
        let label = label.trim();
        if label.is_empty() || label.len() > 80 {
            return Err(Error::Invalid(
                "key label must be 1 to 80 characters".into(),
            ));
        }
        let id = self.registry_row(registry)?.0;
        let fp = fingerprint(public_key);
        self.lock().execute(
            "INSERT INTO registry_keys (registry_id, public_key, fingerprint, label) VALUES (?1, ?2, ?3, ?4)",
            params![id, public_key, fp, label],
        )?;
        Ok(fp)
    }

    /// Operator attestation that `nick` + `email` belong together in this registry.
    /// It does not claim Arch or the AUR checked them. `user_id` is set only when
    /// that person registers with both values, or is already that account.
    pub fn bind_identity(&self, actor: i64, registry: &str, nick: &str, email: &str) -> Result<()> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        self.insert_identity(registry, nick, email)
    }

    pub fn insert_identity(&self, registry: &str, nick: &str, email: &str) -> Result<()> {
        self.require_enforcing(registry)?;
        crate::accounts::validate_username(nick)?;
        let email = normalize_email(email)?;
        let id = self.registry_row(registry)?.0;
        let user_id: Option<i64> = self
            .lock()
            .query_row(
                "SELECT id FROM users WHERE name = ?1 AND lower(email) = ?2",
                params![nick, email],
                |r| r.get(0),
            )
            .optional()?;
        self.lock().execute(
            "INSERT INTO registry_identities (registry_id, nick, email, user_id, provenance) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, nick, email, user_id, PROVENANCE_OPERATOR],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn expect_package(
        &self,
        actor: i64,
        registry: &str,
        pkgbase: &str,
        repository: &str,
        version: &str,
        signature: &[u8],
        key_fingerprint: &str,
        nick: &str,
    ) -> Result<()> {
        if !self.is_admin(actor)? {
            return Err(Error::Forbidden);
        }
        self.insert_expectation(
            registry,
            pkgbase,
            repository,
            version,
            signature,
            key_fingerprint,
            nick,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_expectation(
        &self,
        registry: &str,
        pkgbase: &str,
        repository: &str,
        version: &str,
        signature: &[u8],
        key_fingerprint: &str,
        nick: &str,
    ) -> Result<()> {
        self.require_enforcing(registry)?;
        crate::srcinfo::validate_name(pkgbase)?;
        let repository = clean_token(repository, "repository")?;
        if version.is_empty() || version.len() > 64 {
            return Err(Error::Invalid("version is missing".into()));
        }
        if signature.len() != 64 {
            return Err(Error::Invalid("recipe signature must be 64 bytes".into()));
        }
        let (registry_id, _) = self.registry_row(registry)?;
        let key_id: i64 = self
            .lock()
            .query_row(
                "SELECT id FROM registry_keys WHERE registry_id = ?1 AND fingerprint = ?2",
                params![registry_id, key_fingerprint],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound("registry key".into()))?;
        let identity_id: i64 = self
            .lock()
            .query_row(
                "SELECT id FROM registry_identities WHERE registry_id = ?1 AND nick = ?2",
                params![registry_id, nick],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("identity {nick} in registry {registry}")))?;
        self.lock().execute(
            "INSERT INTO registry_expectations (registry_id, pkgbase, repository, version, signature, key_id, identity_id, provenance) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![registry_id, pkgbase, repository, version, signature, key_id, identity_id, PROVENANCE_OPERATOR],
        )?;
        Ok(())
    }

    /// Reject a registration when a registry identity has this nick and a different email.
    /// No identity for the nick is not a conflict. Does not create an account.
    pub fn guard_registration(&self, name: &str, email: &str) -> Result<()> {
        let email = match normalize_email(email) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT i.email, r.name FROM registry_identities i JOIN registries r ON r.id = i.registry_id WHERE i.nick = ?1 AND r.enforcement = ?2",
        )?;
        let rows = st
            .query_map(params![name, ENFORCE], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(st);
        drop(conn);
        for (bound, registry) in rows {
            if bound != email {
                let scope = registration_scope(&registry, name, &email);
                if self.acceptance_permits(&scope)? {
                    continue;
                }
                self.audit_scoped(
                    "registration",
                    name,
                    &format!(
                        "nick {name} is bound to a different email by an operator identity in registry {registry}"
                    ),
                    "registry",
                    Some(&scope),
                    None,
                )?;
                return Err(Error::Conflict(format!(
                    "registration rejected: {name} does not match the operator identity in registry {registry}"
                )));
            }
        }
        Ok(())
    }

    pub fn bind_registered_user(&self, user_id: i64, name: &str, email: &str) -> Result<()> {
        let Ok(email) = normalize_email(email) else {
            return Ok(());
        };
        self.lock().execute(
            "UPDATE registry_identities SET user_id = ?1 WHERE nick = ?2 AND email = ?3 AND user_id IS NULL",
            params![user_id, name, email],
        )?;
        Ok(())
    }

    /// Known-package policy for an enforcing registry. Exempt registries and
    /// unknown names return [`UploadDecision::ExistingRules`] without writing
    /// a package. A rejection is audited and stores no recipe bytes.
    pub fn check_upload(&self, upload: &RecipeUpload<'_>) -> Result<UploadDecision> {
        let (registry_id, enforcement) = self.registry_row(upload.registry)?;
        if enforcement == EXEMPT {
            return Ok(UploadDecision::ExistingRules);
        }
        if enforcement == MIRROR_ENFORCEMENT {
            return Err(Error::Invalid(format!(
                "registry {} is a recipe mirror and is read-only; mirroring is not an operator signature or an acceptance",
                upload.registry
            )));
        }
        let Some(expected) = self.expectation(registry_id, upload.pkgbase)? else {
            return Ok(UploadDecision::ExistingRules);
        };
        if let Some(actor) = upload.actor {
            self.ensure_not_suspended(actor)?;
        }
        if self.acceptance_permits(&upload_scope(upload))? {
            return Ok(UploadDecision::Overridden);
        }
        let repository = upload.repository.unwrap_or("");
        let Some(signature) = upload.signature else {
            return self.reject_upload(upload, "missing recipe signature");
        };
        if !self.signature_is_trusted(
            registry_id,
            signature,
            upload.registry,
            repository,
            upload.pkgbase,
            upload.version,
            upload.srcinfo,
            upload.pkgbuild,
        )? {
            return self.reject_upload(
                upload,
                "recipe signature is not valid for a trusted key of this registry",
            );
        }
        if repository != expected.repository {
            return self
                .reject_upload(upload, "repository does not match the operator expectation");
        }
        if signature == expected.signature.as_slice() && upload.version == expected.version {
            return Ok(UploadDecision::Exact);
        }
        if cmp_version(upload.version, &expected.version) != Ordering::Greater {
            return self.reject_upload(
                upload,
                "version is not newer than the recorded recipe, and the signature does not match it",
            );
        }
        if !self.uploader_is_bound(upload.actor, &expected)? {
            return self.reject_upload(
                upload,
                "signature differs from the recorded recipe; a new version requires the authenticated account bound to that nick and email",
            );
        }
        Ok(UploadDecision::NewVersion)
    }

    /// The latest admin decision for this exact scope. A newer rejection or a
    /// revocation wins over an older acceptance.
    pub fn acceptance_permits(&self, scope_hash: &str) -> Result<bool> {
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT action FROM audit_events WHERE scope_hash = ?1 AND action IN ('reject', 'accept', 'revoke') ORDER BY id",
        )?;
        let actions = st
            .query_map([scope_hash], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(matches!(actions.last().map(String::as_str), Some("accept")))
    }

    pub fn record_new_version(
        &self,
        registry: &str,
        pkgbase: &str,
        version: &str,
        repository: &str,
        signature: &[u8],
    ) -> Result<()> {
        let (registry_id, _) = self.registry_row(registry)?;
        self.lock().execute(
            "UPDATE registry_expectations SET version = ?1, signature = ?2, repository = ?3 WHERE registry_id = ?4 AND pkgbase = ?5",
            params![version, signature, repository, registry_id, pkgbase],
        )?;
        Ok(())
    }

    fn reject_upload(&self, upload: &RecipeUpload<'_>, detail: &str) -> Result<UploadDecision> {
        let scope = upload_scope(upload);
        self.audit_scoped(
            "registry_upload",
            &format!("{}/{}", upload.registry, upload.pkgbase),
            &format!(
                "known package {} in registry {} rejected: {detail}",
                upload.pkgbase, upload.registry
            ),
            "registry",
            Some(&scope),
            upload.actor,
        )?;
        Err(Error::Invalid(format!(
            "registry {}: known package {} rejected: {detail}",
            upload.registry, upload.pkgbase
        )))
    }

    #[allow(clippy::too_many_arguments)]
    fn signature_is_trusted(
        &self,
        registry_id: i64,
        signature: &[u8],
        registry: &str,
        repository: &str,
        pkgbase: &str,
        version: &str,
        srcinfo: &[u8],
        pkgbuild: &[u8],
    ) -> Result<bool> {
        let conn = self.lock();
        let mut st = conn.prepare("SELECT public_key FROM registry_keys WHERE registry_id = ?1")?;
        let rows = st.query_map([registry_id], |r| r.get::<_, Vec<u8>>(0))?;
        for key in rows {
            if verify_recipe(
                &key?, signature, registry, repository, pkgbase, version, srcinfo, pkgbuild,
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn uploader_is_bound(&self, actor: Option<i64>, expected: &Expectation) -> Result<bool> {
        let Some(actor) = actor else {
            return Ok(false);
        };
        let Some(bound) = expected.user_id else {
            return Ok(false);
        };
        if actor != bound {
            return Ok(false);
        }
        let user = self.user_by_id(actor)?;
        Ok(user.name == expected.nick
            && normalize_email(&user.email).ok().as_deref() == Some(expected.email.as_str()))
    }

    fn expectation(&self, registry_id: i64, pkgbase: &str) -> Result<Option<Expectation>> {
        Ok(self.lock().query_row(
            "SELECT e.repository, e.version, e.signature, i.nick, i.email, i.user_id FROM registry_expectations e JOIN registry_identities i ON i.id = e.identity_id WHERE e.registry_id = ?1 AND e.pkgbase = ?2",
            params![registry_id, pkgbase],
            |r| Ok(Expectation {
                repository: r.get(0)?,
                version: r.get(1)?,
                signature: r.get(2)?,
                nick: r.get(3)?,
                email: r.get(4)?,
                user_id: r.get(5)?,
            }),
        ).optional()?)
    }

    fn require_enforcing(&self, registry: &str) -> Result<()> {
        let (_, enforcement) = self.registry_row(registry)?;
        if enforcement == EXEMPT {
            return Err(Error::Conflict(format!(
                "registry {registry} is an existing package registry and does not take new trust records"
            )));
        }
        if enforcement != ENFORCE {
            return Err(Error::Conflict(format!(
                "registry {registry} is a recipe mirror and does not take operator trust records"
            )));
        }
        Ok(())
    }

    fn registry_row(&self, name: &str) -> Result<(i64, String)> {
        self.lock()
            .query_row(
                "SELECT id, enforcement FROM registries WHERE name = ?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("package registry {name}")))
    }

    fn audit_scoped(
        &self,
        kind: &str,
        target: &str,
        reason: &str,
        source: &str,
        scope_hash: Option<&str>,
        applicant_id: Option<i64>,
    ) -> Result<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO audit_events (at, action, target_kind, target, reason, actor_name, source, reverses, scope_hash, applicant_id) VALUES (?1, 'reject', ?2, ?3, ?4, NULL, ?5, NULL, ?6, ?7)",
            params![now(), kind, target, reason, source, scope_hash, applicant_id],
        )?;
        Ok(conn.last_insert_rowid())
    }
}

fn clean_registry_name(name: &str) -> Result<String> {
    let name = name.trim().to_ascii_lowercase();
    let ok = (1..=32).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && name.starts_with(|c: char| c.is_ascii_lowercase());
    if ok {
        Ok(name)
    } else {
        Err(Error::Invalid(
            "registry name must be 1-32 characters, start with a letter, and use a-z 0-9 _ -"
                .into(),
        ))
    }
}

fn clean_token(value: &str, what: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        return Err(Error::Invalid(format!("invalid {what}")));
    }
    Ok(value.to_string())
}

fn normalize_email(email: &str) -> Result<String> {
    let email = email.trim().to_ascii_lowercase();
    if !email.contains('@') || email.len() > 254 || email.contains(char::is_whitespace) {
        return Err(Error::Invalid("invalid email address".into()));
    }
    Ok(email)
}
