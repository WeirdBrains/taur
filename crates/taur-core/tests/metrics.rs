//! Page views and daily download aggregates.

use std::fs;
use std::net::IpAddr;
use std::path::Path;
use taur_core::Store;
use taur_core::metrics::{MAX_PAGES_PER_VISITOR_DAY, RETENTION_DAYS, VIEW_REPEAT_SECS, day_of};

const T0: i64 = 1_762_819_200 + 3600; // 2025-11-11 01:00 UTC

fn store() -> (tempfile::TempDir, Store) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    (d, s)
}

fn import_pkg(s: &Store, root: &Path, name: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("PKGBUILD"),
        format!("pkgbase={name}\npkgname={name}\npkgver=1\npkgrel=1\narch=(any)\n"),
    )
    .unwrap();
    fs::write(
        dir.join(".SRCINFO"),
        format!(
            "pkgbase = {name}\n\tpkgdesc = x\n\tpkgver = 1\n\tpkgrel = 1\n\tarch = any\n\npkgname = {name}\n"
        ),
    )
    .unwrap();
    s.import_dir(&dir, None).unwrap();
}

fn ip(s: &str) -> Option<IpAddr> {
    Some(s.parse().unwrap())
}

fn raw_tables_contain(dir: &Path, needle: &str) -> bool {
    let db = fs::read(dir.join("taur.db")).unwrap();
    let wal = fs::read(dir.join("taur.db-wal")).unwrap_or_default();
    let n = needle.as_bytes();
    db.windows(n.len()).any(|w| w == n) || wal.windows(n.len()).any(|w| w == n)
}

#[test]
fn views_dedupe_per_visitor_and_window() {
    let (d, s) = store();
    let pk = tempfile::tempdir().unwrap();
    import_pkg(&s, pk.path(), "foo");
    let a = ip("198.51.100.7");
    assert!(s.record_page_view_at("/packages/foo", a, "ua", T0).unwrap());
    assert!(
        !s.record_page_view_at("/packages/foo/", a, "ua", T0 + 60)
            .unwrap()
    );
    assert!(
        s.record_page_view_at("/packages/foo", a, "ua", T0 + VIEW_REPEAT_SECS)
            .unwrap()
    );
    assert!(
        s.record_page_view_at("/packages/foo", ip("198.51.100.8"), "ua", T0)
            .unwrap()
    );
    assert!(s.record_page_view_at("/", a, "ua", T0).unwrap());
    // Unknown packages and unknown routes are not counted.
    assert!(
        !s.record_page_view_at("/packages/nope", a, "ua", T0)
            .unwrap()
    );
    assert!(!s.record_page_view_at("/.env", a, "ua", T0).unwrap());

    let m = s.package_metrics_at("foo", "foo", 30, T0).unwrap();
    assert_eq!(m.views, 3);
    let sum = s.metrics_summary_at(30, T0).unwrap();
    assert_eq!(sum.views, 4);
    assert_eq!(sum.visitor_days, 2, "two distinct visitors today");
    assert_eq!(sum.top_pages[0].name, "/packages/foo");
    assert_eq!(sum.daily.len(), 30);
    assert_eq!(sum.daily.last().unwrap().date, "2025-11-11");
    assert_eq!(sum.daily.last().unwrap().views, 4);
    // The address is never written to the database.
    drop(s);
    assert!(!raw_tables_contain(d.path(), "198.51.100.7"));
}

#[test]
fn visitor_salt_rotates_daily_and_old_hashes_are_deleted() {
    let (_d, s) = store();
    let a = ip("203.0.113.5");
    assert!(s.record_page_view_at("/", a, "ua", T0).unwrap());
    let next = T0 + 86400;
    assert!(s.record_page_view_at("/", a, "ua", next).unwrap());
    let sum = s.metrics_summary_at(2, next).unwrap();
    assert_eq!(sum.visitor_days, 2, "same person on two days counts twice");
    assert_eq!(sum.views, 2);
}

#[test]
fn one_visitor_cannot_add_unbounded_pages() {
    let (_d, s) = store();
    let pk = tempfile::tempdir().unwrap();
    let n = MAX_PAGES_PER_VISITOR_DAY as usize + 5;
    for i in 0..n {
        import_pkg(&s, pk.path(), &format!("p{i}"));
    }
    let a = ip("192.0.2.9");
    let counted = (0..n)
        .filter(|i| {
            s.record_page_view_at(&format!("/packages/p{i}"), a, "ua", T0)
                .unwrap()
        })
        .count();
    assert_eq!(counted as i64, MAX_PAGES_PER_VISITOR_DAY);
}

#[test]
fn downloads_are_aggregated_per_day_and_pruned_after_retention() {
    let (_d, s) = store();
    let pk = tempfile::tempdir().unwrap();
    import_pkg(&s, pk.path(), "foo");
    s.record_snapshot_download_at("foo", T0).unwrap();
    s.record_snapshot_download_at("foo", T0 + 10).unwrap();
    s.record_snapshot_download_at("missing", T0).unwrap();
    let sum = s.metrics_summary_at(30, T0).unwrap();
    assert_eq!(sum.downloads, 2);
    assert_eq!(sum.downloads_all_time, 2);
    assert_eq!(sum.top_downloads[0].name, "foo");
    assert_eq!(
        s.package_metrics_at("foo", "foo", 30, T0)
            .unwrap()
            .downloads,
        2
    );

    // A view on a later day prunes aggregates past retention.
    let later = T0 + RETENTION_DAYS * 86400;
    s.record_page_view_at("/", ip("192.0.2.1"), "ua", later)
        .unwrap();
    let sum = s.metrics_summary_at(365, later).unwrap();
    assert_eq!(sum.downloads, 0);
    assert_eq!(
        sum.downloads_all_time, 2,
        "the all-time counter is not pruned"
    );
    assert_eq!(day_of(later) - day_of(T0), RETENTION_DAYS);
}

#[test]
fn summary_days_are_clamped() {
    let (_d, s) = store();
    assert_eq!(s.metrics_summary_at(0, T0).unwrap().days, 1);
    assert_eq!(s.metrics_summary_at(10_000, T0).unwrap().days, 365);
}
