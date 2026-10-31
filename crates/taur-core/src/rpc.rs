//! AUR RPC v5 request handling and response shapes (aurweb-compatible).

use crate::{Error, PkgInfo, Result, SearchBy, Store};
use serde_json::{Map, Value, json};

pub const VERSION: u8 = 5;
/// aurweb's limit; larger result sets return an error.
pub const MAX_RESULTS: usize = 5000;

const LIST_KEYS: [(&str, &str); 8] = [
    ("depends", "Depends"),
    ("makedepends", "MakeDepends"),
    ("optdepends", "OptDepends"),
    ("checkdepends", "CheckDepends"),
    ("conflicts", "Conflicts"),
    ("provides", "Provides"),
    ("replaces", "Replaces"),
    ("groups", "Groups"),
];

pub fn url_path(pkgbase: &str) -> String {
    format!("/cgit/aur.git/snapshot/{pkgbase}.tar.gz")
}

/// One package object. `full` adds dependency lists (omitted when empty), License,
/// Keywords and CoMaintainers, as in `type=info`.
pub fn info_value(p: &PkgInfo, full: bool) -> Value {
    let mut m = Map::new();
    m.insert("ID".into(), json!(p.id));
    m.insert("Name".into(), json!(p.name));
    m.insert("PackageBaseID".into(), json!(p.pkgbase_id));
    m.insert("PackageBase".into(), json!(p.pkgbase));
    m.insert("Version".into(), json!(p.version));
    m.insert("Description".into(), json!(p.description));
    m.insert("URL".into(), json!(p.url));
    m.insert("NumVotes".into(), json!(p.num_votes));
    m.insert("Popularity".into(), json!(p.popularity));
    m.insert("OutOfDate".into(), json!(p.out_of_date));
    m.insert("Maintainer".into(), json!(p.maintainer));
    m.insert("Submitter".into(), json!(p.submitter));
    m.insert("FirstSubmitted".into(), json!(p.first_submitted));
    m.insert("LastModified".into(), json!(p.last_modified));
    m.insert("URLPath".into(), json!(url_path(&p.pkgbase)));
    // TAUR extension (not in aurweb); clients ignore unknown fields.
    m.insert("Organization".into(), json!(p.organization));
    if full {
        for (kind, key) in LIST_KEYS {
            if let Some(v) = p.relations.get(kind).filter(|v| !v.is_empty()) {
                m.insert(key.into(), json!(v));
            }
        }
        let license = p.relations.get("license").cloned().unwrap_or_default();
        m.insert("License".into(), json!(license));
        m.insert("Keywords".into(), json!(p.keywords));
        m.insert("CoMaintainers".into(), json!(p.comaintainers));
    }
    Value::Object(m)
}

pub fn response(kind: &str, results: Vec<Value>) -> Value {
    json!({"resultcount": results.len(), "results": results, "type": kind, "version": VERSION})
}

pub fn error(msg: &str) -> Value {
    json!({"error": msg, "resultcount": 0, "results": [], "type": "error", "version": VERSION})
}

/// Dispatch an RPC request. `kind` is `type` (info, multiinfo, search, msearch,
/// suggest, suggest-pkgbase); `args` are the `arg`/`arg[]` values in order.
pub fn handle(store: &Store, kind: Option<&str>, args: &[String], by: Option<&str>) -> Value {
    match handle_inner(store, kind, args, by) {
        Ok(v) => v,
        Err(Error::Db(_)) | Err(Error::Io(_)) => error("Internal server error."),
        Err(e) => error(&e.to_string()),
    }
}

fn handle_inner(
    store: &Store,
    kind: Option<&str>,
    args: &[String],
    by: Option<&str>,
) -> Result<Value> {
    let Some(kind) = kind.filter(|k| !k.is_empty()) else {
        return Ok(error("No request type/data specified."));
    };
    let arg = args.first().map(String::as_str).unwrap_or("");
    Ok(match kind {
        "info" | "multiinfo" => {
            let r = store.info(args)?;
            response("multiinfo", r.iter().map(|p| info_value(p, true)).collect())
        }
        "search" | "msearch" => {
            let by = if kind == "msearch" {
                Some("maintainer")
            } else {
                by
            };
            let by = match by.filter(|b| !b.is_empty()) {
                None => SearchBy::NameDesc,
                Some(b) => match SearchBy::parse(b) {
                    Some(x) => x,
                    None => return Ok(error("Incorrect by field specified.")),
                },
            };
            if matches!(by, SearchBy::Name | SearchBy::NameDesc) && arg.chars().count() < 2 {
                return Ok(error("Query arg too small."));
            }
            if arg.is_empty() && by != SearchBy::Maintainer {
                return Ok(error("No request type/data specified."));
            }
            let r = store.search(by, arg)?;
            if r.len() > MAX_RESULTS {
                return Ok(error("Too many package results."));
            }
            response("search", r.iter().map(|p| info_value(p, false)).collect())
        }
        "suggest" => json!(store.suggest(arg)?),
        "suggest-pkgbase" => json!(store.suggest_pkgbase(arg)?),
        _ => error("Incorrect request type specified."),
    })
}
