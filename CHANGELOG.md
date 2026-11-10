# Changelog

All notable changes to this project are documented here. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [Unreleased]
### Added
- Debian, RPM, and Arch packages for x86_64. The packaged binaries need at most glibc 2.31.

## [1.3.1] - 2026-11-09
### Added
- `taur gc` prunes unreachable Git objects older than a grace period and skips a repository that has a push in progress. A refused push does not keep the incoming pack or an empty new repository.
### Fixed
- An uploaded recipe is rejected when an architecture-specific field does not match `arch`, which yay and paru refuse to install.

## [1.3.0] - 2026-11-09
### Added
- Recipe mirrors. An administrator can copy selected packages from an AUR-style Git source. Copies stay read-only, are not operator attestations, and a pacman binary repository is not a source.

## [1.2.0] - 2026-11-07
### Added
- Moderator role that can reject a package or an ordinary account, and a site-admin audit that can reverse those decisions.
- Package registries. The existing `local` registry stays on the previous upload rules. A new registry can require an operator Ed25519 signature over the recipe, and registration can reject a nick that conflicts with an operator identity.
- An administrator can accept one rejected upload or registration so that same attempt can be retried. A newer rejection or a revocation wins.
- `scripts/release.sh` bumps the workspace version, tags `vMAJOR.MINOR.PATCH`, and pushes that commit and tag. It does not upload GitHub release assets.

## [1.1.0] - 2026-11-06
### Added
- `taur serve --tls acme` requests and renews an HTTP-01 certificate (`--domain`, contact, and an explicit terms acceptance). Plain HTTP stays `TAUR_TLS=off`.
- Default per-client limits for RPC, auth, git, the JSON API, dumps, MCP, gRPC, and embedded SSH, with `Retry-After` and an optional trusted-proxy CIDR list.

## [1.0.0] - 2026-11-05
### Added
- RPC rate limit (`TAUR_RPC_RATE_LIMIT`) keyed by the TCP peer, HTTP 429 `Rate limit reached`.
- Adoption requests, aurweb-style disown, and restore of a deleted package base.
### Fixed
- Simple PKGBUILD assignments are checked against `.SRCINFO` without running a shell. `TAUR_PKGBUILD_ISOLATION=bwrap` is opt-in and refuses to run `makepkg` on the host.
- Package search page script parsed again (a missing parenthesis stopped the web UI).

## [0.13.0] - 2026-11-05
### Added
- JSONP callbacks, aurweb `arg`/`arg[]` handling, and JSON bodies on the RPC.
- cgit plain-file URLs and maintainer pages at `/users/<name>`.
- `taur ssh-auth` and `taur git-serve` for OpenSSH, including maintainer SSH commands.

### Fixed
- Smart HTTP clone accepts `/{pkgbase}` as well as `/{pkgbase}.git`.

## [0.12.1] - 2026-10-31
### Fixed
- Tests compile on Windows: Unix file-mode assertions are gated with `cfg(unix)`.

### Changed
- Windows build (`x86_64-pc-windows-gnu`) is now compile-checked (check, clippy, release build); docs/services.md explains how to cross-build.

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
