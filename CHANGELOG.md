# Changelog

All notable changes to this project are documented here. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [0.12.0] - 2026-10-31
### Added
- Service configs for systemd, SysV init, OpenRC, runit, macOS launchd and Windows (WinSW).

### Changed
- Platform-neutral file permission, null device and child environment handling.

## [0.11.1] - 2026-10-31
### Added
- `.env.example` with an example entry for every environment variable.

## [0.11.0] - 2026-10-31
### Added
- Every `taur serve` setting configurable by environment variable (flags override), validated at startup.
- `TAUR_ADMINS` site-admin bootstrap, SSH clone URL settings, and a server info endpoint.
- Example env file and man page ENVIRONMENT sections.

## [0.10.0] - 2026-10-31
### Added
- gRPC API (`taur serve --grpc-bind`) covering packages, accounts, organizations, package management, community features, requests and SSH keys, with bearer-token auth.

## [0.9.0] - 2026-10-31
### Added
- MCP server at `/mcp` (streamable HTTP) with public search/info and token-authenticated management tools.
- `taurctl mcp` stdio bridge.

## [0.8.0] - 2026-10-31
### Added
- Package notification subscriptions (email on pushes and comments), also in the web UI and `taurctl pkg notify|unnotify`.
- Comment pinning (`taurctl comment pin|unpin`, web UI).
- Automatic acceptance of orphan requests for packages flagged out-of-date for 180+ days.
- Extended metadata dump `/packages-meta-ext-v1.json.gz`.

## [0.7.0] - 2026-10-31
### Added
- Static web UI: package search and details, registration, sign-in, verification, password reset, account and SSH keys, organizations and roles, requests.
- Vote status endpoint.

## [0.6.0] - 2026-10-31
### Added
- Votes with aurweb-style popularity.
- Out-of-date flags with maintainer notifications.
- Package comments.
- Co-maintainers.
- Orphan, deletion and merge requests, handled by site admins (`taur admin`).

## [0.5.0] - 2026-10-31
### Added
- `Organization` field in RPC results (TAUR extension).

## [0.4.0] - 2026-10-31
### Added
- Email verification on registration; unverified accounts can't push or manage.
- Password reset by email.
- SMTP or file-outbox mail delivery.
- `taur verify-user` admin command.

## [0.3.0] - 2026-10-31
### Added
- Git pushes over smart HTTP (token auth) and an embedded SSH server, validated like aurweb.
- SSH key management (API and `taurctl key`).
- Smart-HTTP clones.
- Man pages for `taur`.

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
