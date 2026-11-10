//! SQLite-backed storage (bundled SQLite via rusqlite).

use crate::ratelimit::{LimitClass, Limiter, RatePolicy};
use crate::srcinfo::{self, SrcInfo};
use crate::{Error, Result, now, repo};
use rusqlite::{Connection, OptionalExtension, Row, ToSql, params};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

/// Relation kinds stored per package, in `.SRCINFO` key spelling.
pub const RELATION_KINDS: [&str; 9] = [
    "depends",
    "makedepends",
    "checkdepends",
    "optdepends",
    "conflicts",
    "provides",
    "replaces",
    "groups",
    "license",
];

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE,
  email TEXT NOT NULL UNIQUE COLLATE NOCASE,
  password_hash TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  verified_at INTEGER);
CREATE TABLE IF NOT EXISTS email_tokens (
  token_hash TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  purpose TEXT NOT NULL,
  expires_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
  token_hash TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS orgs (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE,
  created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS org_members (
  org_id INTEGER NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  PRIMARY KEY (org_id, user_id));
CREATE TABLE IF NOT EXISTS pkgbases (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  maintainer_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  submitter_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  org_id INTEGER REFERENCES orgs(id) ON DELETE SET NULL,
  num_votes INTEGER NOT NULL DEFAULT 0,
  popularity REAL NOT NULL DEFAULT 0,
  out_of_date INTEGER,
  archived_at INTEGER,
  first_submitted INTEGER NOT NULL,
  last_modified INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS packages (
  id INTEGER PRIMARY KEY,
  pkgbase_id INTEGER NOT NULL REFERENCES pkgbases(id) ON DELETE CASCADE,
  name TEXT NOT NULL UNIQUE,
  version TEXT NOT NULL,
  description TEXT,
  url TEXT);
CREATE TABLE IF NOT EXISTS relations (
  package_id INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  value TEXT NOT NULL,
  dep_name TEXT NOT NULL,
  position INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS relations_lookup ON relations(kind, dep_name);
CREATE TABLE IF NOT EXISTS keywords (
  pkgbase_id INTEGER NOT NULL REFERENCES pkgbases(id) ON DELETE CASCADE,
  keyword TEXT NOT NULL,
  PRIMARY KEY (pkgbase_id, keyword));
CREATE TABLE IF NOT EXISTS ssh_keys (
  id INTEGER PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  fingerprint TEXT NOT NULL UNIQUE,
  algorithm TEXT NOT NULL,
  public_key TEXT NOT NULL,
  comment TEXT NOT NULL,
  created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS votes (
  pkgbase_id INTEGER NOT NULL REFERENCES pkgbases(id) ON DELETE CASCADE,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  voted_at INTEGER NOT NULL,
  PRIMARY KEY (pkgbase_id, user_id));
CREATE TABLE IF NOT EXISTS comments (
  id INTEGER PRIMARY KEY,
  pkgbase_id INTEGER NOT NULL REFERENCES pkgbases(id) ON DELETE CASCADE,
  user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  content TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  edited_at INTEGER,
  deleted_at INTEGER);
CREATE TABLE IF NOT EXISTS notifications (
  pkgbase_id INTEGER NOT NULL REFERENCES pkgbases(id) ON DELETE CASCADE,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY (pkgbase_id, user_id));
CREATE TABLE IF NOT EXISTS requests (
  id INTEGER PRIMARY KEY,
  pkgbase_id INTEGER REFERENCES pkgbases(id) ON DELETE SET NULL,
  pkgbase_name TEXT NOT NULL,
  user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  kind TEXT NOT NULL,
  merge_into TEXT,
  comments TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  closure_comment TEXT,
  closed_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  closed_at INTEGER);
CREATE TABLE IF NOT EXISTS co_maintainers (
  pkgbase_id INTEGER NOT NULL REFERENCES pkgbases(id) ON DELETE CASCADE,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY (pkgbase_id, user_id));
CREATE TABLE IF NOT EXISTS deleted_pkgbases (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  repo_rel TEXT NOT NULL,
  maintainer_id INTEGER,
  deleted_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS audit_events (
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  action TEXT NOT NULL,
  target_kind TEXT NOT NULL,
  target TEXT NOT NULL,
  reason TEXT NOT NULL,
  actor_name TEXT,
  source TEXT NOT NULL,
  reverses INTEGER REFERENCES audit_events(id));
CREATE TABLE IF NOT EXISTS registries (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  enforcement TEXT NOT NULL,
  created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS registry_keys (
  id INTEGER PRIMARY KEY,
  registry_id INTEGER NOT NULL REFERENCES registries(id),
  public_key BLOB NOT NULL,
  fingerprint TEXT NOT NULL,
  label TEXT NOT NULL,
  UNIQUE (registry_id, fingerprint));
CREATE TABLE IF NOT EXISTS registry_identities (
  id INTEGER PRIMARY KEY,
  registry_id INTEGER NOT NULL REFERENCES registries(id),
  nick TEXT NOT NULL,
  email TEXT NOT NULL,
  user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  provenance TEXT NOT NULL,
  UNIQUE (registry_id, nick));
CREATE TABLE IF NOT EXISTS registry_expectations (
  id INTEGER PRIMARY KEY,
  registry_id INTEGER NOT NULL REFERENCES registries(id),
  pkgbase TEXT NOT NULL,
  repository TEXT NOT NULL,
  version TEXT NOT NULL,
  signature BLOB NOT NULL,
  key_id INTEGER NOT NULL REFERENCES registry_keys(id),
  identity_id INTEGER NOT NULL REFERENCES registry_identities(id),
  provenance TEXT NOT NULL,
  UNIQUE (registry_id, pkgbase));
"#;

/// One package as the RPC sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct PkgInfo {
    pub id: i64,
    pub name: String,
    pub pkgbase_id: i64,
    pub pkgbase: String,
    pub version: String,
    pub description: Option<String>,
    pub url: Option<String>,
    pub num_votes: i64,
    pub popularity: f64,
    pub out_of_date: Option<i64>,
    /// Set when a maintainer archived the package. Hidden from search and dumps.
    pub archived_at: Option<i64>,
    pub maintainer: Option<String>,
    pub submitter: Option<String>,
    pub first_submitted: i64,
    pub last_modified: i64,
    /// TAUR extension: owning organization.
    pub organization: Option<String>,
    /// Filled by `info`, empty for search results.
    pub relations: BTreeMap<String, Vec<String>>,
    pub keywords: Vec<String>,
    pub comaintainers: Vec<String>,
}

/// AUR RPC `by=` search fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchBy {
    Name,
    NameDesc,
    Maintainer,
    Submitter,
    Depends,
    MakeDepends,
    OptDepends,
    CheckDepends,
    Provides,
    Conflicts,
    Replaces,
    Groups,
    Keywords,
    CoMaintainers,
}

impl SearchBy {
    pub fn parse(s: &str) -> Option<SearchBy> {
        use SearchBy::*;
        Some(match s {
            "name" => Name,
            "name-desc" => NameDesc,
            "maintainer" => Maintainer,
            "submitter" => Submitter,
            "depends" => Depends,
            "makedepends" => MakeDepends,
            "optdepends" => OptDepends,
            "checkdepends" => CheckDepends,
            "provides" => Provides,
            "conflicts" => Conflicts,
            "replaces" => Replaces,
            "groups" => Groups,
            "keywords" => Keywords,
            "comaintainers" => CoMaintainers,
            _ => return None,
        })
    }

    fn relation_kind(self) -> Option<&'static str> {
        use SearchBy::*;
        Some(match self {
            Depends => "depends",
            MakeDepends => "makedepends",
            OptDepends => "optdepends",
            CheckDepends => "checkdepends",
            Provides => "provides",
            Conflicts => "conflicts",
            Replaces => "replaces",
            Groups => "groups",
            _ => return None,
        })
    }
}

/// Name part of a dependency: strips `: description` and version constraints.
pub fn dep_name(kind: &str, value: &str) -> String {
    let v = if kind == "optdepends" {
        value.split(':').next().unwrap_or(value)
    } else {
        value
    };
    v.split(['<', '>', '='])
        .next()
        .unwrap_or(v)
        .trim()
        .to_string()
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

const SELECT: &str = "SELECT p.id, p.name, b.id, b.name, p.version, p.description, p.url, \
    b.num_votes, b.popularity, b.out_of_date, m.name, s.name, b.first_submitted, b.last_modified, o.name, b.archived_at \
    FROM packages p JOIN pkgbases b ON b.id = p.pkgbase_id \
    LEFT JOIN users m ON m.id = b.maintainer_id LEFT JOIN users s ON s.id = b.submitter_id \
    LEFT JOIN orgs o ON o.id = b.org_id";

fn row_to_info(r: &Row) -> rusqlite::Result<PkgInfo> {
    Ok(PkgInfo {
        id: r.get(0)?,
        name: r.get(1)?,
        pkgbase_id: r.get(2)?,
        pkgbase: r.get(3)?,
        version: r.get(4)?,
        description: r.get(5)?,
        url: r.get(6)?,
        num_votes: r.get(7)?,
        popularity: r.get(8)?,
        out_of_date: r.get(9)?,
        archived_at: r.get(15)?,
        maintainer: r.get(10)?,
        submitter: r.get(11)?,
        first_submitted: r.get(12)?,
        last_modified: r.get(13)?,
        organization: r.get(14)?,
        relations: BTreeMap::new(),
        keywords: Vec::new(),
        comaintainers: Vec::new(),
    })
}

fn query_infos(conn: &Connection, filter: &str, args: &[&dyn ToSql]) -> Result<Vec<PkgInfo>> {
    let sql = format!(
        "{SELECT} WHERE ({filter}) AND b.suspended_at IS NULL AND b.archived_at IS NULL ORDER BY p.name"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(args, row_to_info)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn strings(conn: &Connection, sql: &str, arg: i64) -> Result<Vec<String>> {
    let mut st = conn.prepare(sql)?;
    let rows = st.query_map([arg], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
}

pub struct Store {
    conn: Mutex<Connection>,
    home: PathBuf,
    db_path: PathBuf,
    mailer: std::sync::RwLock<std::sync::Arc<dyn crate::mail::Mailer>>,
    base_url: std::sync::RwLock<String>,
    hook_env: std::sync::RwLock<Vec<(String, String)>>,
    ssh_clone_base: std::sync::RwLock<Option<String>>,
    rate: Mutex<Limiter>,
    /// `TAUR_ADMINS` named someone. Registration must not invent another admin.
    reserve_admins: AtomicBool,
}

fn read_line(dir: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

impl Store {
    /// Open (creating if needed) the store in `home` (`<home>/taur.db`, `repos/`).
    pub fn open(home: &Path) -> Result<Store> {
        Self::open_with_db(home, &home.join("taur.db"))
    }

    /// Open the store in `home` using an explicit SQLite path (for `TAUR_DB` / `--db`).
    ///
    /// `home` still holds git repositories, the mail outbox, and the SSH host key.
    /// The database parent directory must already exist; it is not created. The
    /// path is stored absolute so git hooks can open the same file.
    pub fn open_with_db(home: &Path, db: &Path) -> Result<Store> {
        std::fs::create_dir_all(home.join("repos"))?;
        let db_path = absolutize_db_path(db)?;
        let conn = Connection::open(&db_path).map_err(|e| {
            Error::Invalid(format!("cannot open database {}: {e}", db_path.display()))
        })?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        // v0.4.0: users.verified_at
        let has_verified: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('users') WHERE name = 'verified_at'")?
            .exists([])?;
        if !has_verified {
            conn.execute_batch("ALTER TABLE users ADD COLUMN verified_at INTEGER")?;
        }
        // v0.6.0: site admins, out-of-date flagger
        for (table, col, ty) in [
            ("users", "is_admin", "INTEGER NOT NULL DEFAULT 0"),
            ("users", "is_owner", "INTEGER NOT NULL DEFAULT 0"),
            (
                "pkgbases",
                "flagger_id",
                "INTEGER REFERENCES users(id) ON DELETE SET NULL",
            ),
            ("pkgbases", "flagger_comment", "TEXT"),
            ("comments", "pinned_at", "INTEGER"),
            ("users", "is_moderator", "INTEGER NOT NULL DEFAULT 0"),
            ("users", "suspended_at", "INTEGER"),
            ("users", "suspended_reason", "TEXT"),
            ("pkgbases", "suspended_at", "INTEGER"),
            ("pkgbases", "suspended_reason", "TEXT"),
            ("pkgbases", "archived_at", "INTEGER"),
        ] {
            let exists = conn
                .prepare(&format!(
                    "SELECT 1 FROM pragma_table_info('{table}') WHERE name = '{col}'"
                ))?
                .exists([])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {ty}"))?;
            }
        }
        crate::registry::migrate(&conn)?;
        crate::mirror::migrate(&conn)?;
        let mailer = crate::mail::FileMailer::new(home.join("outbox"), crate::DEFAULT_MAIL_FROM);
        Ok(Store {
            conn: Mutex::new(conn),
            home: home.to_path_buf(),
            db_path,
            mailer: std::sync::RwLock::new(std::sync::Arc::new(mailer)),
            base_url: std::sync::RwLock::new(crate::DEFAULT_BASE_URL.to_string()),
            hook_env: std::sync::RwLock::new(Vec::new()),
            ssh_clone_base: std::sync::RwLock::new(None),
            rate: Mutex::new(Limiter::default()),
            reserve_admins: AtomicBool::new(false),
        })
    }

    /// When set, registration does not grant site admin to the first account.
    /// `taur serve` sets this if `TAUR_ADMINS` names anyone, including a user
    /// who has not registered yet.
    pub fn reserve_admin_bootstrap(&self, reserved: bool) {
        self.reserve_admins.store(reserved, Ordering::SeqCst);
    }

    pub fn admin_bootstrap_reserved(&self) -> bool {
        self.reserve_admins.load(Ordering::SeqCst)
    }

    /// Replace the in-memory limits. `Store::open` starts with every class disabled
    /// so library callers and tests are not limited until a server installs a policy.
    pub fn configure_rate_limits(&self, policy: &RatePolicy) {
        *self.rate.lock().unwrap_or_else(|e| e.into_inner()) = Limiter::from_policy(policy);
    }

    /// Count one request. `peer == None` (no TCP socket) shares a single bucket for
    /// that class. `forwarded` is an `X-Forwarded-For` value and is used only when
    /// `peer` is in the policy's trusted proxy list.
    pub fn enforce_limit(
        &self,
        class: LimitClass,
        peer: Option<IpAddr>,
        forwarded: Option<&str>,
    ) -> Result<()> {
        self.enforce_limit_at(class, peer, forwarded, Instant::now())
    }

    pub fn enforce_limit_at(
        &self,
        class: LimitClass,
        peer: Option<IpAddr>,
        forwarded: Option<&str>,
        now: Instant,
    ) -> Result<()> {
        self.rate
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .enforce(class, peer, forwarded, now)
    }

    /// Replace the mailer (default: file outbox at `<home>/outbox`).
    pub fn set_mailer(&self, mailer: std::sync::Arc<dyn crate::mail::Mailer>) {
        *self.mailer.write().unwrap_or_else(|e| e.into_inner()) = mailer;
    }

    pub fn mailer(&self) -> std::sync::Arc<dyn crate::mail::Mailer> {
        self.mailer
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Public base URL used for links in emails.
    pub fn set_base_url(&self, url: &str) {
        *self.base_url.write().unwrap_or_else(|e| e.into_inner()) =
            url.trim_end_matches('/').to_string();
    }

    /// Configure mail delivery and the public base URL (used by `taur serve` and by git
    /// hooks). Without an SMTP URL, mail goes to `<home>/outbox`. The settings are also
    /// exported to git hook processes via `hook_env` (never logged).
    pub fn configure_mail(&self, smtp_url: Option<&str>, from: &str, base_url: &str) -> Result<()> {
        let mailer: std::sync::Arc<dyn crate::mail::Mailer> = match smtp_url {
            Some(url) => std::sync::Arc::new(crate::mail::SmtpMailer::new(url, from)?),
            None => {
                std::sync::Arc::new(crate::mail::FileMailer::new(self.home.join("outbox"), from))
            }
        };
        self.set_mailer(mailer);
        self.set_base_url(base_url);
        let mut env = vec![
            ("TAUR_MAIL_FROM".to_string(), from.to_string()),
            ("TAUR_BASE_URL".to_string(), base_url.to_string()),
        ];
        if let Some(url) = smtp_url {
            env.push(("TAUR_SMTP_URL".to_string(), url.to_string()));
        }
        *self.hook_env.write().unwrap_or_else(|e| e.into_inner()) = env;
        Ok(())
    }

    /// Public SSH clone prefix (e.g. `ssh://taur@host:2222`), if SSH is enabled.
    pub fn set_ssh_clone_base(&self, base: Option<String>) {
        *self
            .ssh_clone_base
            .write()
            .unwrap_or_else(|e| e.into_inner()) = base;
    }

    pub fn ssh_clone_base(&self) -> Option<String> {
        self.ssh_clone_base
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Environment passed to git hook processes.
    pub fn hook_env(&self) -> Vec<(String, String)> {
        self.hook_env
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn base_url(&self) -> String {
        self.base_url
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Absolute path of the open SQLite database.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn repos_dir(&self) -> PathBuf {
        self.home.join("repos")
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Import a package directory containing `.SRCINFO` (and PKGBUILD etc.):
    /// updates metadata and commits the directory into the pkgbase git repo.
    pub fn import_dir(&self, dir: &Path, maintainer: Option<&str>) -> Result<SrcInfo> {
        let path = dir.join(".SRCINFO");
        let text = std::fs::read_to_string(&path).map_err(|_| {
            Error::NotFound(format!(
                "{} (generate it with `makepkg --printsrcinfo > .SRCINFO`)",
                path.display()
            ))
        })?;
        let info = srcinfo::parse(&text)?;
        if let Some(m) = maintainer
            && self.user_by_name(m)?.is_none()
        {
            return Err(Error::NotFound(format!("user {m}")));
        }
        repo::commit_dir(
            &self.repos_dir(),
            &info.pkgbase,
            dir,
            &format!("Update to {}", info.version()),
        )?;
        self.import_srcinfo(&info, maintainer)?;
        Ok(info)
    }

    /// Import into a named package registry. `None` uses the exempt `local` registry.
    /// A known package in an enforcing registry is checked before any commit.
    pub fn import_into(
        &self,
        dir: &Path,
        maintainer: Option<&str>,
        registry: Option<&str>,
    ) -> Result<SrcInfo> {
        let srcinfo = std::fs::read(dir.join(".SRCINFO")).map_err(|_| {
            Error::NotFound(format!(
                "{} (generate it with `makepkg --printsrcinfo > .SRCINFO`)",
                dir.join(".SRCINFO").display()
            ))
        })?;
        let pkgbuild = std::fs::read(dir.join("PKGBUILD"))
            .map_err(|_| Error::NotFound(format!("missing {}", dir.join("PKGBUILD").display())))?;
        let info = srcinfo::parse(&String::from_utf8_lossy(&srcinfo))?;
        let version = info.version();
        let registry =
            self.resolve_registry(&info.pkgbase, registry, &read_line(dir, "TAUR-REGISTRY"))?;
        let actor = match maintainer {
            Some(name) => Some(
                self.user_by_name(name)?
                    .ok_or_else(|| Error::NotFound(format!("user {name}")))?
                    .id,
            ),
            None => None,
        };
        let signature = read_line(dir, "TAUR-RECIPE.sig")
            .and_then(|text| crate::registry::parse_hex_key(&text, 64).ok());
        let repository = read_line(dir, "TAUR-REPOSITORY");
        let upload = crate::registry::RecipeUpload {
            registry: &registry,
            repository: repository.as_deref(),
            pkgbase: &info.pkgbase,
            version: &version,
            srcinfo: &srcinfo,
            pkgbuild: &pkgbuild,
            signature: signature.as_deref(),
            actor,
        };
        let decision = self.check_upload(&upload)?;
        repo::commit_dir(
            &self.repos_dir(),
            &info.pkgbase,
            dir,
            &format!("Update to {}", info.version()),
        )?;
        self.import_srcinfo_in(&info, maintainer, &registry, None)?;
        if decision == crate::registry::UploadDecision::NewVersion {
            self.record_new_version(
                &registry,
                &info.pkgbase,
                &version,
                repository.as_deref().unwrap_or(""),
                signature.as_deref().unwrap_or(&[]),
            )?;
        }
        Ok(info)
    }

    pub(crate) fn resolve_registry(
        &self,
        pkgbase: &str,
        explicit: Option<&str>,
        file_name: &Option<String>,
    ) -> Result<String> {
        let asked = explicit
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or(file_name.as_deref())
            .unwrap_or(crate::registry::LEGACY_REGISTRY)
            .to_string();
        if let Some(current) = self.pkgbase_registry(pkgbase)? {
            if current != asked {
                return Err(Error::Conflict(format!(
                    "{pkgbase} is in registry {current} and cannot be moved to {asked}"
                )));
            }
            return Ok(current);
        }
        Ok(asked)
    }

    pub fn pkgbase_registry(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT r.name FROM pkgbases b JOIN registries r ON r.id = b.registry_id WHERE b.name = ?1",
                [name],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Insert or update a pkgbase and its packages from parsed `.SRCINFO`.
    /// The package stays on the legacy `local` registry.
    pub fn import_srcinfo(&self, info: &SrcInfo, maintainer: Option<&str>) -> Result<i64> {
        self.import_srcinfo_in(info, maintainer, crate::registry::LEGACY_REGISTRY, None)
    }

    /// Like [`Store::import_srcinfo`], placing a new package base in `registry`.
    /// An existing base cannot move to a different registry.
    pub fn import_srcinfo_in(
        &self,
        info: &SrcInfo,
        maintainer: Option<&str>,
        registry: &str,
        org: Option<i64>,
    ) -> Result<i64> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let now = now();
        let registry_id: i64 = tx
            .query_row(
                "SELECT id FROM registries WHERE name = ?1",
                [registry],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("package registry {registry}")))?;
        let mid: Option<i64> = match maintainer {
            Some(n) => Some(
                tx.query_row("SELECT id FROM users WHERE name = ?1", [n], |r| r.get(0))
                    .optional()?
                    .ok_or_else(|| Error::NotFound(format!("user {n}")))?,
            ),
            None => None,
        };
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM pkgbases WHERE name = ?1",
                [&info.pkgbase],
                |r| r.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => {
                let current: Option<i64> = tx
                    .query_row(
                        "SELECT registry_id FROM pkgbases WHERE id = ?1",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if current.is_some_and(|c| c != registry_id) {
                    return Err(Error::Conflict(format!(
                        "{} is already in another package registry",
                        info.pkgbase
                    )));
                }
                tx.execute(
                    "UPDATE pkgbases SET last_modified = ?1, maintainer_id = COALESCE(?2, maintainer_id), registry_id = COALESCE(registry_id, ?3) WHERE id = ?4",
                    params![now, mid, registry_id, id],
                )?;
                // A new version clears the out-of-date flag.
                let old: Option<String> = tx
                    .query_row(
                        "SELECT version FROM packages WHERE pkgbase_id = ?1 LIMIT 1",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if old.as_deref() != Some(info.version().as_str()) {
                    tx.execute(
                        "UPDATE pkgbases SET out_of_date = NULL, flagger_id = NULL, flagger_comment = NULL WHERE id = ?1",
                        [id],
                    )?;
                }
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO pkgbases (name, maintainer_id, submitter_id, first_submitted, last_modified, registry_id, org_id) VALUES (?1, ?2, ?2, ?3, ?3, ?4, ?5)",
                    params![info.pkgbase, mid, now, registry_id, org],
                )?;
                tx.last_insert_rowid()
            }
        };
        for (name, _) in &info.packages {
            let owner: Option<i64> = tx
                .query_row(
                    "SELECT pkgbase_id FROM packages WHERE name = ?1",
                    [name],
                    |r| r.get(0),
                )
                .optional()?;
            if owner.is_some_and(|o| o != id) {
                return Err(Error::Conflict(format!(
                    "package {name} belongs to another package base"
                )));
            }
        }
        tx.execute("DELETE FROM packages WHERE pkgbase_id = ?1", [id])?;
        let version = info.version();
        for (i, (name, _)) in info.packages.iter().enumerate() {
            tx.execute(
                "INSERT INTO packages (pkgbase_id, name, version, description, url) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, name, version, info.first(i, "pkgdesc"), info.first(i, "url")],
            )?;
            let pid = tx.last_insert_rowid();
            for kind in RELATION_KINDS {
                for (pos, v) in info.values(i, kind).iter().enumerate() {
                    tx.execute(
                        "INSERT INTO relations (package_id, kind, value, dep_name, position) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![pid, kind, v, dep_name(kind, v), pos as i64],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(id)
    }

    pub fn pkgbase_id(&self, name: &str) -> Result<Option<i64>> {
        Ok(self
            .lock()
            .query_row("SELECT id FROM pkgbases WHERE name = ?1", [name], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_keywords(&self, pkgbase: &str, keywords: &[String]) -> Result<()> {
        let id = self
            .pkgbase_id(pkgbase)?
            .ok_or_else(|| Error::NotFound(format!("package base {pkgbase}")))?;
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM keywords WHERE pkgbase_id = ?1", [id])?;
        for k in keywords {
            tx.execute(
                "INSERT OR IGNORE INTO keywords (pkgbase_id, keyword) VALUES (?1, ?2)",
                params![id, k],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Full package details for the given package names (RPC `info`).
    pub fn info(&self, names: &[String]) -> Result<Vec<PkgInfo>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let placeholders = vec!["?"; names.len()].join(",");
        let args: Vec<&dyn ToSql> = names.iter().map(|n| n as &dyn ToSql).collect();
        let mut infos = query_infos(&conn, &format!("p.name IN ({placeholders})"), &args)?;
        for p in &mut infos {
            let mut st = conn.prepare(
                "SELECT kind, value FROM relations WHERE package_id = ?1 ORDER BY kind, position",
            )?;
            let rows = st.query_map([p.id], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?;
            for row in rows {
                let (k, v) = row?;
                p.relations.entry(k).or_default().push(v);
            }
            p.keywords = strings(
                &conn,
                "SELECT keyword FROM keywords WHERE pkgbase_id = ?1 ORDER BY keyword",
                p.pkgbase_id,
            )?;
            p.comaintainers = strings(
                &conn,
                "SELECT u.name FROM co_maintainers c JOIN users u ON u.id = c.user_id WHERE c.pkgbase_id = ?1 ORDER BY u.name",
                p.pkgbase_id,
            )?;
        }
        Ok(infos)
    }

    /// Package page lookup. Includes archived packages and hides rejected ones.
    pub fn package_page(&self, name: &str) -> Result<Option<PkgInfo>> {
        let conn = self.lock();
        let sql = format!(
            "{SELECT} WHERE (p.name = ?1 OR b.name = ?1) AND b.suspended_at IS NULL ORDER BY CASE WHEN p.name = ?1 THEN 0 ELSE 1 END, p.name"
        );
        let mut st = conn.prepare(&sql)?;
        let mut infos = st.query_map([name], row_to_info)?;
        let Some(mut info) = infos.next().transpose()? else {
            return Ok(None);
        };
        drop(infos);
        drop(st);
        let mut rel = conn.prepare(
            "SELECT kind, value FROM relations WHERE package_id = ?1 ORDER BY kind, position",
        )?;
        for row in rel.query_map([info.id], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))? {
            let (k, v) = row?;
            info.relations.entry(k).or_default().push(v);
        }
        info.keywords = strings(
            &conn,
            "SELECT keyword FROM keywords WHERE pkgbase_id = ?1 ORDER BY keyword",
            info.pkgbase_id,
        )?;
        info.comaintainers = strings(
            &conn,
            "SELECT u.name FROM co_maintainers c JOIN users u ON u.id = c.user_id WHERE c.pkgbase_id = ?1 ORDER BY u.name",
            info.pkgbase_id,
        )?;
        Ok(Some(info))
    }

    /// RPC `search`. An empty `arg` with `by=maintainer` lists orphans.
    pub fn search(&self, by: SearchBy, arg: &str) -> Result<Vec<PkgInfo>> {
        let conn = self.lock();
        let like = format!("%{}%", escape_like(arg));
        if let Some(kind) = by.relation_kind() {
            return query_infos(
                &conn,
                "EXISTS (SELECT 1 FROM relations r WHERE r.package_id = p.id AND r.kind = ?1 AND r.dep_name = ?2)",
                &[&kind, &arg],
            );
        }
        match by {
            SearchBy::Name => query_infos(&conn, "p.name LIKE ?1 ESCAPE '\\'", &[&like]),
            SearchBy::NameDesc => query_infos(
                &conn,
                "(p.name LIKE ?1 ESCAPE '\\' OR p.description LIKE ?1 ESCAPE '\\')",
                &[&like],
            ),
            SearchBy::Maintainer if arg.is_empty() => {
                query_infos(&conn, "b.maintainer_id IS NULL", &[])
            }
            SearchBy::Maintainer => query_infos(&conn, "m.name = ?1", &[&arg]),
            SearchBy::Submitter => query_infos(&conn, "s.name = ?1", &[&arg]),
            SearchBy::Keywords => query_infos(
                &conn,
                "EXISTS (SELECT 1 FROM keywords k WHERE k.pkgbase_id = b.id AND k.keyword = ?1)",
                &[&arg],
            ),
            _ => query_infos(
                &conn,
                "EXISTS (SELECT 1 FROM co_maintainers c JOIN users u ON u.id = c.user_id WHERE c.pkgbase_id = b.id AND u.name = ?1)",
                &[&arg],
            ),
        }
    }

    fn prefix_names(&self, table: &str, prefix: &str) -> Result<Vec<String>> {
        let conn = self.lock();
        let sql = if table == "packages" {
            "SELECT p.name FROM packages p JOIN pkgbases b ON b.id = p.pkgbase_id \
             WHERE b.suspended_at IS NULL AND b.archived_at IS NULL AND p.name LIKE ?1 ESCAPE '\\' ORDER BY p.name LIMIT 20"
                .to_string()
        } else {
            format!(
                "SELECT name FROM {table} WHERE suspended_at IS NULL AND archived_at IS NULL AND name LIKE ?1 ESCAPE '\\' ORDER BY name LIMIT 20"
            )
        };
        let mut st = conn.prepare(&sql)?;
        let rows = st.query_map([format!("{}%", escape_like(prefix))], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    }

    /// RPC `suggest`: up to 20 package names starting with `prefix`.
    pub fn suggest(&self, prefix: &str) -> Result<Vec<String>> {
        self.prefix_names("packages", prefix)
    }

    /// RPC `suggest-pkgbase`: up to 20 pkgbase names starting with `prefix`.
    pub fn suggest_pkgbase(&self, prefix: &str) -> Result<Vec<String>> {
        self.prefix_names("pkgbases", prefix)
    }

    pub fn all_packages(&self) -> Result<Vec<PkgInfo>> {
        query_infos(&self.lock(), "1 = 1", &[])
    }

    pub fn all_pkgbase_names(&self) -> Result<Vec<String>> {
        let conn = self.lock();
        let mut st =
            conn.prepare(
                "SELECT name FROM pkgbases WHERE suspended_at IS NULL AND archived_at IS NULL ORDER BY name",
            )?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    }
}

fn absolutize_db_path(db: &Path) -> Result<PathBuf> {
    if db.as_os_str().is_empty() {
        return Err(Error::Invalid(
            "database path is empty (TAUR_DB / --db)".into(),
        ));
    }
    let db = if db.is_absolute() {
        db.to_path_buf()
    } else {
        std::env::current_dir()?.join(db)
    };
    if db.is_dir() {
        return Err(Error::Invalid(format!(
            "database path {} is a directory (TAUR_DB / --db)",
            db.display()
        )));
    }
    match db.parent() {
        None => {
            return Err(Error::Invalid(format!(
                "database path {} has no parent directory (TAUR_DB / --db)",
                db.display()
            )));
        }
        Some(parent) if !parent.exists() => {
            return Err(Error::Invalid(format!(
                "database directory {} does not exist (TAUR_DB / --db)",
                parent.display()
            )));
        }
        Some(parent) if !parent.is_dir() => {
            return Err(Error::Invalid(format!(
                "database directory {} is not a directory (TAUR_DB / --db)",
                parent.display()
            )));
        }
        Some(_) => {}
    }
    Ok(db)
}
