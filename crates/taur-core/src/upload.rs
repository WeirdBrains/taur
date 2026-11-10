//! Publish a package from a recipe archive.
//!
//! The archive is a `.tar.gz`, `.tar`, or `.zip` of one package directory:
//! `PKGBUILD` and `.SRCINFO` at the top, or inside a single wrapping folder.
//! The same checks as a git push and `taur import` run here. The PKGBUILD is
//! not executed.

use crate::push::MAX_BLOB_SIZE;
use crate::srcinfo::{self, SrcInfo};
use crate::{Error, Result, Store};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// Largest accepted archive, compressed or not.
pub const MAX_ARCHIVE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FILES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub pkgbase: String,
    /// First `pkgname`, used as the package page.
    pub name: String,
    pub version: String,
    pub created: bool,
}

/// A temporary directory holding the unpacked recipe. Removed on drop.
pub struct RecipeDir {
    path: PathBuf,
}

impl RecipeDir {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RecipeDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Unpack a recipe archive into a flat directory.
pub fn extract_recipe(bytes: &[u8]) -> Result<RecipeDir> {
    if bytes.is_empty() {
        return Err(Error::Invalid("the upload is empty".into()));
    }
    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(Error::Invalid("the archive is larger than 8 MiB".into()));
    }
    let files = match sniff(bytes)? {
        Kind::Zip => read_zip(bytes)?,
        Kind::Gzip => read_tar(flate2::read::GzDecoder::new(std::io::Cursor::new(bytes)))?,
        Kind::Tar => read_tar(std::io::Cursor::new(bytes))?,
    };
    let files = flatten(files)?;
    if !files.iter().any(|(name, _)| name == "PKGBUILD")
        || !files.iter().any(|(name, _)| name == ".SRCINFO")
    {
        return Err(Error::Invalid(
            "the archive must contain PKGBUILD and .SRCINFO".into(),
        ));
    }
    let path = unique_dir()?;
    for (name, data) in &files {
        let dest = path.join(name);
        let mut f = std::fs::File::create(&dest)?;
        f.write_all(data)?;
    }
    Ok(RecipeDir { path })
}

enum Kind {
    Zip,
    Gzip,
    Tar,
}

fn sniff(bytes: &[u8]) -> Result<Kind> {
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        Ok(Kind::Zip)
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        Ok(Kind::Gzip)
    } else if bytes.len() > 262 && &bytes[257..262] == b"ustar" {
        Ok(Kind::Tar)
    } else {
        Err(Error::Invalid(
            "upload a .tar.gz, .tar, or .zip of the package directory".into(),
        ))
    }
}

fn read_tar<R: Read>(reader: R) -> Result<Vec<(String, Vec<u8>)>> {
    let mut archive = tar::Archive::new(std::io::Cursor::new(read_limited(reader)?));
    let mut out = Vec::new();
    for entry in archive.entries().map_err(bad_archive)? {
        let mut entry = entry.map_err(bad_archive)?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(Error::Invalid(
                "the archive may only contain regular files".into(),
            ));
        }
        let rel = entry.path().map_err(bad_archive)?.into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(bad_archive)?;
        out.push((rel.to_string_lossy().replace('\\', "/"), data));
        if out.len() > MAX_FILES {
            return Err(Error::Invalid(format!(
                "a recipe archive may contain at most {MAX_FILES} files"
            )));
        }
    }
    Ok(out)
}

fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(bad_archive)?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(bad_archive)?;
        if file.is_dir() {
            continue;
        }
        if file.is_symlink() {
            return Err(Error::Invalid(
                "the archive may only contain regular files".into(),
            ));
        }
        let name = file.name().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).map_err(bad_archive)?;
        out.push((name.replace('\\', "/"), data));
        if out.len() > MAX_FILES {
            return Err(Error::Invalid(format!(
                "a recipe archive may contain at most {MAX_FILES} files"
            )));
        }
    }
    Ok(out)
}

fn flatten(raw: Vec<(String, Vec<u8>)>) -> Result<Vec<(String, Vec<u8>)>> {
    let mut parsed = Vec::new();
    for (name, data) in raw {
        if data.len() as u64 > MAX_BLOB_SIZE {
            return Err(Error::Invalid(format!(
                "{name} exceeds the {MAX_BLOB_SIZE} byte size limit"
            )));
        }
        let rel = safe_rel(Path::new(&name))?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        parsed.push((rel, data));
    }
    if parsed.is_empty() {
        return Err(Error::Invalid("the archive has no files".into()));
    }
    let strip = parsed
        .iter()
        .all(|(path, _)| path.components().count() >= 2)
        && parsed
            .iter()
            .map(|(path, _)| path.components().next())
            .all(|c| c == parsed[0].0.components().next());
    let mut out = Vec::new();
    for (path, data) in parsed {
        let path = if strip {
            path.components().skip(1).collect::<PathBuf>()
        } else {
            path
        };
        if path.components().count() != 1 {
            return Err(Error::Invalid(
                "subdirectories are not allowed; put PKGBUILD and .SRCINFO at the top, or inside one folder"
                    .into(),
            ));
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Invalid("a file name is not valid UTF-8".into()))?
            .to_string();
        if name.is_empty() || name == "." || name == ".." {
            return Err(Error::Invalid(
                "the archive has an unusable file name".into(),
            ));
        }
        out.push((name, data));
    }
    Ok(out)
}

fn safe_rel(path: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => {
                return Err(Error::Invalid(
                    "the archive contains a path that escapes the package directory".into(),
                ));
            }
        }
    }
    Ok(out)
}

fn read_limited<R: Read>(mut reader: R) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if out.len() as u64 + n as u64 > MAX_ARCHIVE_BYTES {
            return Err(Error::Invalid(
                "the uncompressed archive is larger than 8 MiB".into(),
            ));
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

fn unique_dir() -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "taur-upload-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir(&path)?;
    Ok(path)
}

fn bad_archive(err: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("could not read the archive: {err}"))
}

fn read_line_file(dir: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

impl Store {
    /// Create or update a package base from an unpacked recipe directory.
    /// A new name is adopted by `user`. An existing base must be one they can manage.
    pub fn publish_recipe(&self, user: &crate::accounts::User, dir: &Path) -> Result<Published> {
        self.require_verified(user)?;
        self.ensure_not_suspended(user.id)?;
        let srcinfo_bytes = std::fs::read(dir.join(".SRCINFO")).map_err(|_| {
            Error::Invalid(
                "missing .SRCINFO (generate it with `makepkg --printsrcinfo > .SRCINFO`)".into(),
            )
        })?;
        let pkgbuild_bytes = std::fs::read(dir.join("PKGBUILD"))
            .map_err(|_| Error::Invalid("missing PKGBUILD".into()))?;
        for ent in std::fs::read_dir(dir)? {
            let ent = ent?;
            let meta = std::fs::symlink_metadata(ent.path())?;
            if meta.file_type().is_symlink() {
                return Err(Error::Invalid("symlinks are not allowed".into()));
            }
            if meta.is_dir() {
                return Err(Error::Invalid(
                    "subdirectories are not allowed in the package directory".into(),
                ));
            }
            if meta.len() > MAX_BLOB_SIZE {
                return Err(Error::Invalid(format!(
                    "{} exceeds the {MAX_BLOB_SIZE} byte size limit",
                    ent.file_name().to_string_lossy()
                )));
            }
        }
        let text = String::from_utf8(srcinfo_bytes.clone())
            .map_err(|_| Error::Invalid(".SRCINFO is not UTF-8".into()))?;
        let info = srcinfo::parse(&text)?;
        let pkgbuild = String::from_utf8(pkgbuild_bytes.clone())
            .map_err(|_| Error::Invalid("PKGBUILD is not UTF-8".into()))?;
        let mismatches = srcinfo::literal_mismatches(&info, &pkgbuild);
        if !mismatches.is_empty() {
            return Err(Error::Invalid(format!(
                "{} (simple PKGBUILD assignments only; the PKGBUILD was not executed)",
                mismatches.join("; ")
            )));
        }
        if self.pkgbase_suspended(&info.pkgbase)? {
            return Err(Error::Conflict(format!(
                "package {} is rejected",
                info.pkgbase
            )));
        }
        for (name, _) in &info.packages {
            if let Some(owner) = self.package_owner(name)?
                && owner != info.pkgbase
            {
                return Err(Error::Conflict(format!(
                    "package {name} already exists in {owner}"
                )));
            }
        }
        let created = self.pkgbase_id(&info.pkgbase)?.is_none();
        if let Some(id) = self.pkgbase_id(&info.pkgbase)?
            && !self.can_manage_pkgbase(user.id, id)?
        {
            return Err(Error::Forbidden);
        }
        let version = info.version();
        let registry =
            self.resolve_registry(&info.pkgbase, None, &read_line_file(dir, "TAUR-REGISTRY"))?;
        let repository = read_line_file(dir, "TAUR-REPOSITORY");
        let signature = read_line_file(dir, "TAUR-RECIPE.sig")
            .and_then(|text| crate::registry::parse_hex_key(&text, 64).ok());
        let decision = self.check_upload(&crate::registry::RecipeUpload {
            registry: &registry,
            repository: repository.as_deref(),
            pkgbase: &info.pkgbase,
            version: &version,
            srcinfo: &srcinfo_bytes,
            pkgbuild: &pkgbuild_bytes,
            signature: signature.as_deref(),
            actor: Some(user.id),
        })?;
        crate::repo::commit_dir(
            &self.repos_dir(),
            &info.pkgbase,
            dir,
            &format!("Update to {version}"),
        )?;
        let pb = self.import_srcinfo_in(
            &info,
            if created {
                Some(user.name.as_str())
            } else {
                None
            },
            &registry,
        )?;
        if decision == crate::registry::UploadDecision::NewVersion
            && let (Some(repository), Some(signature)) =
                (repository.as_deref(), signature.as_deref())
        {
            self.record_new_version(&registry, &info.pkgbase, &version, repository, signature)?;
        }
        if !created {
            self.notify_subscribers(
                pb,
                Some(user.id),
                &format!("{} updated to {version}", info.pkgbase),
                &format!(
                    "{} published {} {version}.\n\n{}/packages/{}\n",
                    user.name,
                    info.pkgbase,
                    self.base_url(),
                    info.packages
                        .first()
                        .map(|(n, _)| n.as_str())
                        .unwrap_or(&info.pkgbase)
                ),
            );
        }
        Ok(published(&info, created))
    }
}

fn published(info: &SrcInfo, created: bool) -> Published {
    Published {
        name: info
            .packages
            .first()
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| info.pkgbase.clone()),
        pkgbase: info.pkgbase.clone(),
        version: info.version(),
        created,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn recipe(rel: &str) -> (String, String) {
        (
            format!("pkgbase=webup\npkgname=(webup)\npkgver=1.0.0\npkgrel={rel}\narch=(any)\n"),
            format!(
                "pkgbase = webup\n\tpkgdesc = Uploaded\n\tpkgver = 1.0.0\n\tpkgrel = {rel}\n\tarch = any\n\npkgname = webup\n"
            ),
        )
    }

    fn gzip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut raw = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut raw);
            for (name, body) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, *name, body.as_bytes())
                    .unwrap();
            }
            builder.finish().unwrap();
        }
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&raw).unwrap();
        enc.finish().unwrap()
    }

    fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, body) in files {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn archives_round_trip_including_a_wrapping_folder() {
        let (pkg, src) = recipe("1");
        let flat = extract_recipe(&gzip(&[("PKGBUILD", &pkg), (".SRCINFO", &src)])).unwrap();
        assert!(flat.path().join("PKGBUILD").is_file());
        let wrapped =
            extract_recipe(&gzip(&[("webup/PKGBUILD", &pkg), ("webup/.SRCINFO", &src)])).unwrap();
        assert!(wrapped.path().join(".SRCINFO").is_file());
        let zipped = extract_recipe(&zip_bytes(&[("PKGBUILD", &pkg), (".SRCINFO", &src)])).unwrap();
        assert!(zipped.path().join("PKGBUILD").is_file());
        let mut evil = vec![0u8; 512];
        evil[..5].copy_from_slice(b"../x\0");
        evil.extend(vec![0u8; 1024]);
        assert!(
            extract_recipe(&{
                let mut enc =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                enc.write_all(&evil).unwrap();
                enc.finish().unwrap()
            })
            .is_err()
        );
    }

    #[test]
    fn publish_creates_then_updates_and_refuses_another_user() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store.reserve_admin_bootstrap(true);
        store.register("ada", "ada@x", "longenough").unwrap();
        let ada = store.user_by_name("ada").unwrap().unwrap();
        store.mark_verified(ada.id).unwrap();
        let ada = store.user_by_name("ada").unwrap().unwrap();
        let (pkg, src) = recipe("1");
        let unpacked = extract_recipe(&gzip(&[("PKGBUILD", &pkg), (".SRCINFO", &src)])).unwrap();
        let created = store.publish_recipe(&ada, unpacked.path()).unwrap();
        assert!(created.created);
        assert_eq!(created.pkgbase, "webup");
        assert_eq!(
            store.info(&["webup".into()]).unwrap()[0]
                .maintainer
                .as_deref(),
            Some("ada")
        );
        let (pkg, src) = recipe("2");
        let next = extract_recipe(&gzip(&[("PKGBUILD", &pkg), (".SRCINFO", &src)])).unwrap();
        let updated = store.publish_recipe(&ada, next.path()).unwrap();
        assert!(!updated.created);
        assert!(updated.version.ends_with("-2"));
        store.register("bob", "bob@x", "longenough").unwrap();
        store
            .mark_verified(store.user_by_name("bob").unwrap().unwrap().id)
            .unwrap();
        let bob = store.user_by_name("bob").unwrap().unwrap();
        assert!(matches!(
            store.publish_recipe(&bob, next.path()),
            Err(Error::Forbidden)
        ));
        let carol = store.register("carol", "carol@x", "longenough").unwrap();
        assert!(matches!(
            store.publish_recipe(&carol, next.path()),
            Err(Error::Unverified)
        ));
        let bad = extract_recipe(&gzip(&[
            (
                "PKGBUILD",
                "pkgbase=webup\npkgname=(webup)\npkgver=9\npkgrel=1\n",
            ),
            (".SRCINFO", &src),
        ]))
        .unwrap();
        assert!(store.publish_recipe(&ada, bad.path()).is_err());
    }
}
