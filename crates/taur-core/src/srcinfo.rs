//! `.SRCINFO` parser (the format `makepkg --printsrcinfo` emits).

use crate::{Error, Result};
use std::collections::BTreeMap;

pub type Fields = BTreeMap<String, Vec<String>>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SrcInfo {
    pub pkgbase: String,
    pub base: Fields,
    /// `(pkgname, overrides)` in file order.
    pub packages: Vec<(String, Fields)>,
}

/// AUR package name rule: `[a-z0-9@._+-]+`, not starting with `-` or `.`.
pub fn validate_name(s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.len() <= 255
        && !s.starts_with('-')
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "@._+-".contains(c));
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(format!("invalid package name {s:?}")))
    }
}

pub fn parse(text: &str) -> Result<SrcInfo> {
    let mut info = SrcInfo::default();
    let mut seen_base = false;
    let mut current: Option<usize> = None;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| Error::Parse(format!("line {}: expected `key = value`", n + 1)))?;
        let (k, v) = (k.trim(), v.trim());
        match k {
            "pkgbase" => {
                if seen_base {
                    return Err(Error::Parse(format!("line {}: duplicate pkgbase", n + 1)));
                }
                seen_base = true;
                info.pkgbase = v.to_string();
            }
            _ if !seen_base => {
                return Err(Error::Parse(format!("line {}: {k} before pkgbase", n + 1)));
            }
            "pkgname" => {
                info.packages.push((v.to_string(), Fields::new()));
                current = Some(info.packages.len() - 1);
            }
            _ => {
                let target = match current {
                    Some(i) => &mut info.packages[i].1,
                    None => &mut info.base,
                };
                target.entry(k.to_string()).or_default().push(v.to_string());
            }
        }
    }
    if !seen_base {
        return Err(Error::Parse("missing pkgbase".into()));
    }
    if info.packages.is_empty() {
        return Err(Error::Parse("no pkgname sections".into()));
    }
    validate_name(&info.pkgbase)?;
    for (name, _) in &info.packages {
        validate_name(name)?;
    }
    for key in ["pkgver", "pkgrel"] {
        if info.base.get(key).and_then(|v| v.first()).is_none() {
            return Err(Error::Parse(format!("missing {key}")));
        }
    }
    Ok(info)
}

impl SrcInfo {
    /// Full version string: `[epoch:]pkgver-pkgrel`.
    pub fn version(&self) -> String {
        let get = |k: &str| {
            self.base
                .get(k)
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or_default()
        };
        let epoch = get("epoch");
        let core = format!("{}-{}", get("pkgver"), get("pkgrel"));
        if epoch.is_empty() || epoch == "0" {
            core
        } else {
            format!("{epoch}:{core}")
        }
    }

    /// Values of `key` for package `idx`, merging architecture-specific variants
    /// (`key_<arch>`). A package section overrides the pkgbase section per key;
    /// an empty override (`depends =`) clears it.
    pub fn values(&self, idx: usize, key: &str) -> Vec<String> {
        let pkg = &self.packages[idx].1;
        let prefix = format!("{key}_");
        let mut keys: Vec<&String> = self
            .base
            .keys()
            .chain(pkg.keys())
            .filter(|k| k.as_str() == key || k.starts_with(&prefix))
            .collect();
        keys.sort();
        keys.dedup();
        // plain key first, then arch variants
        keys.sort_by_key(|k| k.as_str() != key);
        let mut out: Vec<String> = Vec::new();
        for k in keys {
            let vals = pkg.get(k).or_else(|| self.base.get(k));
            for v in vals.into_iter().flatten() {
                if !v.is_empty() && !out.contains(v) {
                    out.push(v.clone());
                }
            }
        }
        out
    }

    pub fn first(&self, idx: usize, key: &str) -> Option<String> {
        self.values(idx, key).into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/hello-taur/.SRCINFO");

    #[test]
    fn parses_fixture() {
        let s = parse(FIXTURE).unwrap();
        assert_eq!(s.pkgbase, "hello-taur");
        assert_eq!(s.version(), "1:1.2.3-1");
        assert_eq!(s.packages.len(), 2);
        assert_eq!(s.values(0, "depends"), vec!["glibc", "lib32-glibc"]);
        assert_eq!(s.values(1, "depends"), vec!["lib32-glibc"]);
        assert_eq!(s.first(1, "pkgdesc").unwrap(), "Docs for hello-taur");
        assert_eq!(
            s.first(0, "pkgdesc").unwrap(),
            "A tiny test package for TAUR"
        );
        assert_eq!(s.values(0, "makedepends"), vec!["rust>=1.80"]);
        assert!(s.values(0, "conflicts").is_empty());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse("pkgname = x\n").is_err());
        assert!(parse("pkgbase = x\npkgver = 1\npkgrel = 1\n").is_err());
        assert!(parse("pkgbase = x\npkgname = x\n").is_err());
        assert!(parse("pkgbase = Bad\npkgver=1\npkgrel=1\npkgname = Bad\n").is_err());
        assert!(parse("pkgbase = x\nnonsense\n").is_err());
        assert!(validate_name("-x").is_err());
        assert!(validate_name("a@b.c_d+e-f").is_ok());
    }
}
