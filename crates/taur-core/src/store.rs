//! SQLite-backed storage (bundled SQLite via rusqlite).

use crate::srcinfo::{self, SrcInfo};
use crate::{Error, Result, now, repo};
use rusqlite::{Connection, OptionalExtension, Row, ToSql, params};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

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
  created_at INTEGER NOT NULL);
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
CREATE TABLE IF NOT EXISTS co_maintainers (
  pkgbase_id INTEGER NOT NULL REFERENCES pkgbases(id) ON DELETE CASCADE,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY (pkgbase_id, user_id));
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
    pub maintainer: Option<String>,
    pub submitter: Option<String>,
    pub first_submitted: i64,
    pub last_modified: i64,
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
    b.num_votes, b.popularity, b.out_of_date, m.name, s.name, b.first_submitted, b.last_modified \
    FROM packages p JOIN pkgbases b ON b.id = p.pkgbase_id \
    LEFT JOIN users m ON m.id = b.maintainer_id LEFT JOIN users s ON s.id = b.submitter_id";

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
        maintainer: r.get(10)?,
        submitter: r.get(11)?,
        first_submitted: r.get(12)?,
        last_modified: r.get(13)?,
        relations: BTreeMap::new(),
        keywords: Vec::new(),
        comaintainers: Vec::new(),
    })
}

fn query_infos(conn: &Connection, filter: &str, args: &[&dyn ToSql]) -> Result<Vec<PkgInfo>> {
    let sql = format!("{SELECT} WHERE {filter} ORDER BY p.name");
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
}

impl Store {
    /// Open (creating if needed) the store in `home` (`taur.db`, `repos/`).
    pub fn open(home: &Path) -> Result<Store> {
        std::fs::create_dir_all(home.join("repos"))?;
        let conn = Connection::open(home.join("taur.db"))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store {
            conn: Mutex::new(conn),
            home: home.to_path_buf(),
        })
    }

    pub fn home(&self) -> &Path {
        &self.home
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

    /// Insert or update a pkgbase and its packages from parsed `.SRCINFO`.
    pub fn import_srcinfo(&self, info: &SrcInfo, maintainer: Option<&str>) -> Result<i64> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let now = now();
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
                tx.execute(
                    "UPDATE pkgbases SET last_modified = ?1, maintainer_id = COALESCE(?2, maintainer_id) WHERE id = ?3",
                    params![now, mid, id],
                )?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO pkgbases (name, maintainer_id, submitter_id, first_submitted, last_modified) VALUES (?1, ?2, ?2, ?3, ?3)",
                    params![info.pkgbase, mid, now],
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
        let sql = format!(
            "SELECT name FROM {table} WHERE name LIKE ?1 ESCAPE '\\' ORDER BY name LIMIT 20"
        );
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
        let mut st = conn.prepare("SELECT name FROM pkgbases ORDER BY name")?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    }
}
