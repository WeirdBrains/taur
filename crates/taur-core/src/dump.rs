//! Metadata dumps like aur.archlinux.org: `packages.gz`, `pkgbase.gz`,
//! `packages-meta-v1.json.gz`. Generated on request.

use crate::{Result, Store, rpc::info_value};
use flate2::{Compression, write::GzEncoder};
use std::io::Write;

pub fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data)?;
    Ok(e.finish()?)
}

fn lines(names: Vec<String>) -> Vec<u8> {
    names
        .into_iter()
        .flat_map(|n| format!("{n}\n").into_bytes())
        .collect()
}

impl Store {
    /// Newline-separated package names, gzipped.
    pub fn dump_packages(&self) -> Result<Vec<u8>> {
        let names = self.all_packages()?.into_iter().map(|p| p.name).collect();
        gzip(&lines(names))
    }

    /// Newline-separated pkgbase names, gzipped.
    pub fn dump_pkgbases(&self) -> Result<Vec<u8>> {
        gzip(&lines(self.all_pkgbase_names()?))
    }

    /// JSON array of info-shaped package objects (with dependencies etc.), gzipped.
    pub fn dump_packages_meta_ext(&self) -> Result<Vec<u8>> {
        let names: Vec<String> = self.all_packages()?.into_iter().map(|p| p.name).collect();
        let all: Vec<_> = names
            .chunks(500)
            .map(|c| self.info(c))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .map(|p| info_value(&p, true))
            .collect();
        gzip(&serde_json::to_vec(&all)?)
    }

    /// JSON array of search-shaped package objects, gzipped.
    pub fn dump_packages_meta(&self) -> Result<Vec<u8>> {
        let all: Vec<_> = self
            .all_packages()?
            .iter()
            .map(|p| info_value(p, false))
            .collect();
        gzip(&serde_json::to_vec(&all)?)
    }
}
