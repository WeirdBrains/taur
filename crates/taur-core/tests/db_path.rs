//! `TAUR_DB` / `--db` path resolution and `Store::open_with_db`.
//! One test in this binary because it mutates the process environment.

use std::path::PathBuf;
use taur_core::{Store, resolve_db};

fn set(k: &str, v: &str) {
    // SAFETY: this test binary runs a single test that touches the environment.
    unsafe { std::env::set_var(k, v) }
}

fn unset(k: &str) {
    unsafe { std::env::remove_var(k) }
}

fn open_err(home: &std::path::Path, db: &std::path::Path) -> taur_core::Error {
    match Store::open_with_db(home, db) {
        Ok(_) => panic!("expected open_with_db to fail for {}", db.display()),
        Err(e) => e,
    }
}

#[test]
fn resolve_and_open_custom_database_path() {
    unset("TAUR_DB");
    let home = tempfile::tempdir().unwrap();
    let default = home.path().join("taur.db");
    assert_eq!(resolve_db(None, home.path()), default);

    let from_env = home.path().join("from-env.db");
    set("TAUR_DB", from_env.to_str().unwrap());
    assert_eq!(resolve_db(None, home.path()), from_env);

    let from_flag = PathBuf::from("/flag/taur.db");
    assert_eq!(resolve_db(Some(from_flag.clone()), home.path()), from_flag);
    unset("TAUR_DB");

    let store = Store::open(home.path()).unwrap();
    assert_eq!(store.home(), home.path());
    assert_eq!(store.db_path(), default.as_path());
    assert!(default.is_file());
    drop(store);

    let other = tempfile::tempdir().unwrap();
    let db = other.path().join("elsewhere.db");
    let missing_parent = other.path().join("no-such-dir").join("taur.db");
    let err = open_err(home.path(), &missing_parent);
    let msg = err.to_string();
    assert!(
        msg.contains("does not exist") && msg.contains("TAUR_DB"),
        "{msg}"
    );
    assert!(!missing_parent.exists());

    let err = open_err(home.path(), PathBuf::from("").as_path());
    let msg = err.to_string();
    assert!(msg.contains("empty") && msg.contains("TAUR_DB"), "{msg}");

    let err = open_err(home.path(), other.path());
    let msg = err.to_string();
    assert!(
        msg.contains("is a directory") && msg.contains("TAUR_DB"),
        "{msg}"
    );

    let store = Store::open_with_db(home.path(), &db).unwrap();
    assert_eq!(store.db_path(), db.as_path());
    assert!(db.is_file());
    assert!(!other.path().join("taur.db").exists());
    store
        .register("alice", "a@example.com", "correct horse")
        .unwrap();
    drop(store);

    let store = Store::open_with_db(home.path(), &db).unwrap();
    assert!(store.user_by_name("alice").unwrap().is_some());
    // The default file from Store::open was not reused.
    assert_ne!(store.db_path(), default.as_path());
}
