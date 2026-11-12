//! Privacy-preserving usage metrics: web UI page views and package downloads.
//!
//! - **Page views** come from the web UI's explicit view report (`POST /api/v1/views`).
//!   A view counts at most once per visitor, page and [`VIEW_REPEAT_SECS`]. A visitor
//!   is a truncated SHA-256 of a random per-UTC-day salt, the client address and the
//!   User-Agent. The address is never stored. The salt and every visitor hash are
//!   deleted when the UTC day ends, so visitors cannot be linked across days.
//! - **Downloads** are successful snapshot/recipe-archive responses, the same event
//!   as `DownloadCount`, also aggregated per UTC day.
//! - **Installs** are not counted. pacman, yay and paru do not report installs, and
//!   a download is not an install.
//!
//! Only daily aggregates are kept, for [`RETENTION_DAYS`].

use crate::{Result, Store, now};
use rand_core::{OsRng, RngCore};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::net::IpAddr;

/// Daily aggregates older than this are deleted.
pub const RETENTION_DAYS: i64 = 400;
/// A repeat view of the same page by the same visitor within this window is ignored.
pub const VIEW_REPEAT_SECS: i64 = 30 * 60;
/// Distinct pages one visitor can add per day; more are ignored.
pub const MAX_PAGES_PER_VISITOR_DAY: i64 = 100;
/// Longest window `metrics_summary` accepts.
pub const MAX_SUMMARY_DAYS: i64 = 365;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS metric_salts (
  day INTEGER PRIMARY KEY,
  salt BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS page_view_seen (
  day INTEGER NOT NULL,
  visitor BLOB NOT NULL,
  page TEXT NOT NULL,
  last_at INTEGER NOT NULL,
  PRIMARY KEY (day, visitor, page));
CREATE TABLE IF NOT EXISTS page_view_days (
  day INTEGER NOT NULL,
  page TEXT NOT NULL,
  views INTEGER NOT NULL DEFAULT 0,
  visitors INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, page));
CREATE TABLE IF NOT EXISTS site_visitor_days (
  day INTEGER PRIMARY KEY,
  visitors INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS download_days (
  day INTEGER NOT NULL,
  pkgbase TEXT NOT NULL,
  downloads INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, pkgbase));
";

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

/// UTC day number (days since 1970-01-01).
pub fn day_of(ts: i64) -> i64 {
    ts.div_euclid(86400)
}

/// Fixed web UI pages that are counted as-is. Other pages collapse to a template.
const FIXED_PAGES: [&str; 14] = [
    "/",
    "/orgs",
    "/docs",
    "/login",
    "/register",
    "/verify",
    "/reset",
    "/account",
    "/upload",
    "/requests",
    "/audit",
    "/admin",
    "/registries",
    "/mirrors",
];

/// Page key for a web UI path, or `None` when it is not counted. Query strings
/// and fragments are dropped (they can carry tokens). Package pages keep the
/// package name only when `package` resolves it; everything else with a name
/// collapses to a template such as `/users/*`, so the set of keys stays small.
pub fn normalize_page(path: &str, package: impl FnOnce(&str) -> Option<String>) -> Option<String> {
    let path = path.split(['?', '#']).next().unwrap_or("");
    if !path.starts_with('/') || path.len() > 512 {
        return None;
    }
    let trimmed = if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    };
    let trimmed = if trimmed.is_empty() { "/" } else { trimmed };
    if trimmed == "/packages" || trimmed == "/stats" {
        return Some(if trimmed == "/stats" { "/stats" } else { "/" }.into());
    }
    if FIXED_PAGES.contains(&trimmed) {
        return Some(trimmed.into());
    }
    let parts: Vec<&str> = trimmed[1..].split('/').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    match parts.as_slice() {
        ["packages", name] => package(name).map(|n| format!("/packages/{n}")),
        ["packages", name, "revisions", _] => {
            package(name).map(|n| format!("/packages/{n}/revisions/*"))
        }
        ["users", _] => Some("/users/*".into()),
        ["orgs", _] => Some("/orgs/*".into()),
        ["docs", _] => Some("/docs/*".into()),
        ["registries", _] => Some("/registries/*".into()),
        ["mirrors", _] => Some("/mirrors/*".into()),
        _ => None,
    }
}

/// One count in a summary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Count {
    pub name: String,
    pub count: i64,
}

/// One UTC day of a summary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayTotals {
    /// `YYYY-MM-DD` (UTC).
    pub date: String,
    pub views: i64,
    pub visitors: i64,
    pub downloads: i64,
}

/// Aggregate metrics for the last `days` UTC days (today included).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricsSummary {
    pub days: i64,
    pub views: i64,
    /// Sum of daily distinct visitors. The same person on two days counts twice.
    pub visitor_days: i64,
    pub downloads: i64,
    /// All-time snapshot downloads (`DownloadCount` summed), including before daily
    /// aggregates existed.
    pub downloads_all_time: i64,
    pub top_pages: Vec<Count>,
    pub top_downloads: Vec<Count>,
    pub daily: Vec<DayTotals>,
}

/// Page views and downloads for one package.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackageMetrics {
    pub days: i64,
    pub views: i64,
    pub downloads: i64,
}

/// Civil date for a UTC day number (Howard Hinnant's algorithm).
pub fn date_of_day(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn visitor_hash(salt: &[u8], client: Option<IpAddr>, user_agent: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(salt);
    match client {
        Some(ip) => h.update(ip.to_string().as_bytes()),
        None => h.update(b"-"),
    }
    h.update(b"\n");
    h.update(
        user_agent
            .as_bytes()
            .get(..256)
            .unwrap_or(user_agent.as_bytes()),
    );
    h.finalize()[..16].to_vec()
}

/// Today's salt, creating it (and pruning expired data) on the first use of a day.
fn salt_for(conn: &Connection, day: i64) -> Result<Vec<u8>> {
    if let Some(s) = conn
        .query_row("SELECT salt FROM metric_salts WHERE day = ?1", [day], |r| {
            r.get(0)
        })
        .optional()?
    {
        return Ok(s);
    }
    let mut salt = vec![0u8; 32];
    OsRng.fill_bytes(&mut salt);
    conn.execute(
        "INSERT OR IGNORE INTO metric_salts (day, salt) VALUES (?1, ?2)",
        params![day, salt],
    )?;
    prune(conn, day)?;
    Ok(
        conn.query_row("SELECT salt FROM metric_salts WHERE day = ?1", [day], |r| {
            r.get(0)
        })?,
    )
}

/// Drop salts and visitor hashes from earlier days, and aggregates past retention.
fn prune(conn: &Connection, today: i64) -> Result<()> {
    let oldest = today - RETENTION_DAYS + 1;
    conn.execute("DELETE FROM metric_salts WHERE day < ?1", [today])?;
    conn.execute("DELETE FROM page_view_seen WHERE day < ?1", [today])?;
    conn.execute("DELETE FROM page_view_days WHERE day < ?1", [oldest])?;
    conn.execute("DELETE FROM site_visitor_days WHERE day < ?1", [oldest])?;
    conn.execute("DELETE FROM download_days WHERE day < ?1", [oldest])?;
    Ok(())
}

pub(crate) fn record_download_day(conn: &Connection, pkgbase: &str, at: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO download_days (day, pkgbase, downloads)
         SELECT ?1, name, 1 FROM pkgbases WHERE name = ?2
         ON CONFLICT (day, pkgbase) DO UPDATE SET downloads = downloads + 1",
        params![day_of(at), pkgbase],
    )?;
    Ok(())
}

fn canonical_package(conn: &Connection, name: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT p.name FROM packages p JOIN pkgbases b ON b.id = p.pkgbase_id
             WHERE (p.name = ?1 OR b.name = ?1) AND b.suspended_at IS NULL
             ORDER BY CASE WHEN p.name = ?1 THEN 0 ELSE 1 END, p.name LIMIT 1",
            [name],
            |r| r.get(0),
        )
        .optional()?)
}

impl Store {
    /// Record one web UI page view. Returns whether it was counted. Unknown pages,
    /// repeats within [`VIEW_REPEAT_SECS`] and visitors past
    /// [`MAX_PAGES_PER_VISITOR_DAY`] are ignored. `client` is used only to derive
    /// the day's visitor hash; it is not stored.
    pub fn record_page_view(
        &self,
        path: &str,
        client: Option<IpAddr>,
        user_agent: &str,
    ) -> Result<bool> {
        self.record_page_view_at(path, client, user_agent, now())
    }

    pub fn record_page_view_at(
        &self,
        path: &str,
        client: Option<IpAddr>,
        user_agent: &str,
        at: i64,
    ) -> Result<bool> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut lookup_err = None;
        let page = normalize_page(path, |n| match canonical_package(&tx, n) {
            Ok(v) => v,
            Err(e) => {
                lookup_err = Some(e);
                None
            }
        });
        if let Some(e) = lookup_err {
            return Err(e);
        }
        let Some(page) = page else {
            return Ok(false);
        };
        let day = day_of(at);
        let salt = salt_for(&tx, day)?;
        let visitor = visitor_hash(&salt, client, user_agent);
        let last: Option<i64> = tx
            .query_row(
                "SELECT last_at FROM page_view_seen WHERE day = ?1 AND visitor = ?2 AND page = ?3",
                params![day, visitor, page],
                |r| r.get(0),
            )
            .optional()?;
        match last {
            Some(t) if at - t < VIEW_REPEAT_SECS => return Ok(false),
            Some(_) => {
                tx.execute(
                    "UPDATE page_view_seen SET last_at = ?4 WHERE day = ?1 AND visitor = ?2 AND page = ?3",
                    params![day, visitor, page, at],
                )?;
                tx.execute(
                    "UPDATE page_view_days SET views = views + 1 WHERE day = ?1 AND page = ?2",
                    params![day, page],
                )?;
            }
            None => {
                let pages: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM page_view_seen WHERE day = ?1 AND visitor = ?2",
                    params![day, visitor],
                    |r| r.get(0),
                )?;
                if pages >= MAX_PAGES_PER_VISITOR_DAY {
                    return Ok(false);
                }
                tx.execute(
                    "INSERT INTO page_view_seen (day, visitor, page, last_at) VALUES (?1, ?2, ?3, ?4)",
                    params![day, visitor, page, at],
                )?;
                tx.execute(
                    "INSERT INTO page_view_days (day, page, views, visitors) VALUES (?1, ?2, 1, 1)
                     ON CONFLICT (day, page) DO UPDATE SET views = views + 1, visitors = visitors + 1",
                    params![day, page],
                )?;
                if pages == 0 {
                    tx.execute(
                        "INSERT INTO site_visitor_days (day, visitors) VALUES (?1, 1)
                         ON CONFLICT (day) DO UPDATE SET visitors = visitors + 1",
                        [day],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(true)
    }

    /// Site-wide aggregates for the last `days` UTC days (clamped to 1..=365).
    pub fn metrics_summary(&self, days: i64) -> Result<MetricsSummary> {
        self.metrics_summary_at(days, now())
    }

    pub fn metrics_summary_at(&self, days: i64, at: i64) -> Result<MetricsSummary> {
        let days = days.clamp(1, MAX_SUMMARY_DAYS);
        let today = day_of(at);
        let from = today - days + 1;
        let conn = self.lock();
        let sum = |sql: &str| -> Result<i64> {
            Ok(conn.query_row(sql, params![from, today], |r| r.get(0))?)
        };
        let views =
            sum("SELECT COALESCE(SUM(views), 0) FROM page_view_days WHERE day BETWEEN ?1 AND ?2")?;
        let visitor_days = sum(
            "SELECT COALESCE(SUM(visitors), 0) FROM site_visitor_days WHERE day BETWEEN ?1 AND ?2",
        )?;
        let downloads = sum(
            "SELECT COALESCE(SUM(downloads), 0) FROM download_days WHERE day BETWEEN ?1 AND ?2",
        )?;
        let downloads_all_time: i64 = conn.query_row(
            "SELECT COALESCE(SUM(download_count), 0) FROM pkgbases WHERE suspended_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        let counts = |sql: &str| -> Result<Vec<Count>> {
            let mut st = conn.prepare(sql)?;
            let rows = st.query_map(params![from, today], |r| {
                Ok(Count {
                    name: r.get(0)?,
                    count: r.get(1)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        let top_pages = counts(
            "SELECT page, SUM(views) AS n FROM page_view_days WHERE day BETWEEN ?1 AND ?2
             GROUP BY page ORDER BY n DESC, page ASC LIMIT 10",
        )?;
        let top_downloads = counts(
            "SELECT d.pkgbase, SUM(d.downloads) AS n FROM download_days d
             JOIN pkgbases b ON b.name = d.pkgbase AND b.suspended_at IS NULL
             WHERE d.day BETWEEN ?1 AND ?2
             GROUP BY d.pkgbase ORDER BY n DESC, d.pkgbase ASC LIMIT 10",
        )?;
        let mut daily = Vec::with_capacity(days as usize);
        {
            let mut st = conn.prepare(
                "SELECT
                   (SELECT COALESCE(SUM(views), 0) FROM page_view_days WHERE day = ?1),
                   (SELECT COALESCE(SUM(visitors), 0) FROM site_visitor_days WHERE day = ?1),
                   (SELECT COALESCE(SUM(downloads), 0) FROM download_days WHERE day = ?1)",
            )?;
            for day in from..=today {
                let (v, u, d) = st.query_row([day], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                daily.push(DayTotals {
                    date: date_of_day(day),
                    views: v,
                    visitors: u,
                    downloads: d,
                });
            }
        }
        Ok(MetricsSummary {
            days,
            views,
            visitor_days,
            downloads,
            downloads_all_time,
            top_pages,
            top_downloads,
            daily,
        })
    }

    /// Package page views (package name) and downloads (its package base) for the
    /// last `days` UTC days.
    pub fn package_metrics_at(
        &self,
        name: &str,
        pkgbase: &str,
        days: i64,
        at: i64,
    ) -> Result<PackageMetrics> {
        let days = days.clamp(1, MAX_SUMMARY_DAYS);
        let today = day_of(at);
        let from = today - days + 1;
        let conn = self.lock();
        let views = conn.query_row(
            "SELECT COALESCE(SUM(views), 0) FROM page_view_days WHERE page = ?1 AND day BETWEEN ?2 AND ?3",
            params![format!("/packages/{name}"), from, today],
            |r| r.get(0),
        )?;
        let downloads = conn.query_row(
            "SELECT COALESCE(SUM(downloads), 0) FROM download_days WHERE pkgbase = ?1 AND day BETWEEN ?2 AND ?3",
            params![pkgbase, from, today],
            |r| r.get(0),
        )?;
        Ok(PackageMetrics {
            days,
            views,
            downloads,
        })
    }

    pub fn package_metrics(&self, name: &str, pkgbase: &str, days: i64) -> Result<PackageMetrics> {
        self.package_metrics_at(name, pkgbase, days, now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_utc_civil_dates() {
        assert_eq!(date_of_day(0), "1970-01-01");
        assert_eq!(date_of_day(day_of(1_762_819_200)), "2025-11-11");
        assert_eq!(date_of_day(day_of(951_782_400)), "2000-02-29");
    }

    #[test]
    fn pages_collapse_names_and_drop_queries() {
        let pkg = |n: &str| (n == "foo").then(|| "foo".to_string());
        assert_eq!(normalize_page("/", pkg).as_deref(), Some("/"));
        assert_eq!(normalize_page("/packages/", pkg).as_deref(), Some("/"));
        assert_eq!(normalize_page("/?q=x&by=name", pkg).as_deref(), Some("/"));
        assert_eq!(
            normalize_page("/verify?token=secret", pkg).as_deref(),
            Some("/verify")
        );
        assert_eq!(
            normalize_page("/packages/foo", pkg).as_deref(),
            Some("/packages/foo")
        );
        assert_eq!(normalize_page("/packages/nope", pkg), None);
        assert_eq!(
            normalize_page("/packages/foo/revisions/abc123", pkg).as_deref(),
            Some("/packages/foo/revisions/*")
        );
        assert_eq!(
            normalize_page("/users/alice", pkg).as_deref(),
            Some("/users/*")
        );
        assert_eq!(
            normalize_page("/orgs/acme/", pkg).as_deref(),
            Some("/orgs/*")
        );
        assert_eq!(normalize_page("/stats", pkg).as_deref(), Some("/stats"));
        assert_eq!(normalize_page("/wp-admin.php", pkg), None);
        assert_eq!(normalize_page("https://evil.example/", pkg), None);
        assert_eq!(normalize_page("//x", pkg), None);
    }

    #[test]
    fn visitor_hash_depends_on_salt() {
        let ip = Some("192.0.2.1".parse().unwrap());
        let a = visitor_hash(b"one", ip, "ua");
        assert_eq!(a, visitor_hash(b"one", ip, "ua"));
        assert_ne!(a, visitor_hash(b"two", ip, "ua"));
        assert_ne!(a, visitor_hash(b"one", ip, "other"));
        assert_eq!(a.len(), 16);
    }
}
