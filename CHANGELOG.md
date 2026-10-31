# Changelog

All notable changes to this project are documented here. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [0.2.0] - 2026-10-31
### Added
- `taurctl` management CLI (accounts, search/info, organizations, roles, package ownership).
- Organization, membership and package ownership API (adopt, disown, transfer).
- Man pages for `taurctl`.

## [0.1.0] - 2026-10-31
### Added
- AUR RPC v5 endpoints (info, search with all `by` fields, suggest, suggest-pkgbase, legacy form).
- Git clone and snapshot tarballs per package base.
- Metadata dumps (`packages.gz`, `pkgbase.gz`, `packages-meta-v1.json.gz`).
- `taur import` for package directories with `.SRCINFO`.
- User accounts: register, login, logout, unregister.
- Organization roles model (Owner, Admin, Contributor).
