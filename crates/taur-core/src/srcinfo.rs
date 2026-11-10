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
    // Architectures named by `arch` in the pkgbase section, in file order.
    // yay and paru (go-srcinfo) check an architecture-specific field against
    // this list as it stands when the field is read. A later `arch` line does
    // not apply backwards, and a pkgname section's own `arch` does not apply.
    let mut base_arches: Vec<String> = Vec::new();
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
                if current.is_none() && k == "arch" && !v.is_empty() {
                    base_arches.push(v.to_string());
                }
                if let Some(msg) = arch_field_error(k, &base_arches) {
                    return Err(Error::Parse(format!("line {}: {msg}", n + 1)));
                }
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

/// Simple `key=value` lines in a PKGBUILD that disagree with `info`.
///
/// A line is simple when it has no `$`, backtick, or command substitution.
/// `pkgname=(a b)` is read as a list. Anything else is left unchecked: this
/// function does not run the PKGBUILD and cannot prove it matches `.SRCINFO`.
pub fn literal_mismatches(info: &SrcInfo, pkgbuild: &str) -> Vec<String> {
    let simple = simple_assignments(pkgbuild);
    let mut out = Vec::new();
    if let Some(values) = simple.get("pkgbase")
        && values.iter().any(|v| v != &info.pkgbase)
    {
        out.push(format!(
            "PKGBUILD pkgbase ({}) does not match .SRCINFO pkgbase {}",
            values.join(" "),
            info.pkgbase
        ));
    }
    for key in ["pkgver", "pkgrel", "epoch"] {
        let Some(want) = info.base.get(key).and_then(|v| v.first()) else {
            continue;
        };
        if let Some(values) = simple.get(key)
            && values.iter().any(|v| v != want)
        {
            out.push(format!(
                "PKGBUILD {key} ({}) does not match .SRCINFO {key} {want}",
                values.join(" ")
            ));
        }
    }
    if let Some(names) = simple.get("pkgname") {
        let expected: Vec<&str> = info.packages.iter().map(|(n, _)| n.as_str()).collect();
        if names.iter().map(String::as_str).collect::<Vec<_>>() != expected {
            out.push(format!(
                "PKGBUILD pkgname ({}) does not match .SRCINFO pkgnames ({})",
                names.join(" "),
                expected.join(" ")
            ));
        }
    }
    out
}

/// yay and paru refuse a field such as `depends_x86_64` unless that architecture
/// was already named by a pkgbase `arch` line. `depends_any` is never a real
/// architecture. The suffix is the text after the first `_`.
fn arch_field_error(key: &str, base_arches: &[String]) -> Option<String> {
    let (_, arch) = key.split_once('_')?;
    if arch.is_empty() {
        return None;
    }
    if arch == "any" {
        return Some(format!("invalid key \"{key}\" arch \"any\" is not allowed"));
    }
    if !base_arches.iter().any(|a| a == arch) {
        return Some(format!("invalid key \"{key}\" unsupported arch \"{arch}\""));
    }
    None
}

fn simple_assignments(pkgbuild: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut out = std::collections::BTreeMap::<String, Vec<String>>::new();
    for raw in pkgbuild.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        if value.contains('$') || value.contains('`') {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'');
        let values = if let Some(inner) = value.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
        {
            inner
                .split_whitespace()
                .map(|s| s.trim_matches('"').trim_matches('\'').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![value.to_string()]
        };
        out.entry(key.to_string()).or_default().extend(values);
    }
    out
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

    fn recipe(body: &str) -> String {
        format!("pkgbase = demo\n\tpkgver = 1\n\tpkgrel = 1\n{body}\npkgname = demo\n")
    }

    #[test]
    fn rejects_arch_fields_helpers_cannot_parse() {
        let any = parse(&recipe("\tarch = any\n\tdepends_x86_64 = lib32-glibc\n")).unwrap_err();
        assert!(
            any.to_string().contains("depends_x86_64")
                && any.to_string().contains("unsupported arch \"x86_64\""),
            "{any}"
        );

        let denied = parse(&recipe("\tarch = x86_64\n\tdepends_any = glibc\n")).unwrap_err();
        assert!(
            denied.to_string().contains("depends_any")
                && denied.to_string().contains("not allowed"),
            "{denied}"
        );

        let early = parse(&recipe("\tdepends_x86_64 = glibc\n\tarch = x86_64\n")).unwrap_err();
        assert!(early.to_string().contains("unsupported arch"), "{early}");

        let other = parse(&recipe("\tarch = x86_64\n\tdepends_aarch64 = glibc\n")).unwrap_err();
        assert!(other.to_string().contains("aarch64"), "{other}");

        // A pkgname `arch` does not make an architecture-specific field legal.
        let split = "pkgbase = demo\n\tpkgver = 1\n\tpkgrel = 1\n\tarch = any\n\npkgname = demo\n\tarch = x86_64\n\tdepends_x86_64 = glibc\n";
        assert!(parse(split).is_err());

        let ok = parse(&recipe(
            "\tarch = x86_64\n\tarch = aarch64\n\tdepends_x86_64 = glibc\n\tdepends_aarch64 = glibc\n",
        ))
        .unwrap();
        assert_eq!(ok.values(0, "depends"), vec!["glibc"]);

        let inherited = "pkgbase = demo\n\tpkgver = 1\n\tpkgrel = 1\n\tarch = x86_64\n\npkgname = demo\n\tdepends_x86_64 = glibc\n";
        assert!(parse(inherited).is_ok());
    }

    #[test]
    fn literal_pkgbuild_lines() {
        let s = parse(FIXTURE).unwrap();
        let pkg = include_str!("../../../tests/fixtures/hello-taur/PKGBUILD");
        assert!(
            literal_mismatches(&s, pkg).is_empty(),
            "{:?}",
            literal_mismatches(&s, pkg)
        );
        let bad = "pkgbase=other\npkgname=other\npkgver=9\npkgrel=1\n";
        let m = literal_mismatches(&s, bad);
        assert!(m.iter().any(|e| e.contains("pkgbase")), "{m:?}");
        let dynamic = "pkgver=$pkgver\npkgname=(hello-taur hello-taur-docs)\n";
        let m = literal_mismatches(&s, dynamic);
        assert!(m.iter().all(|e| !e.contains("pkgver")), "{m:?}");
    }
}
