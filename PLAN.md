# TAUR plan

## Owner's plan (verbatim)
- Make an AUR API compatible server so that tools like yay can use it
- Add MCP support
- Do this in rust
- Copy and follow the AGENTS.md rules from ~/Projects/WeirdBrains/muninn
- Add all support at the level that AUR has
- Add a gRPC server
- Add a static site that allows registration, sign in, and management of your packages
- Allow organizations to be created and managed by users
- Add roles for users (Owner, Admin, Contributor)
  - Owners have rights to every action
  - Admins can add users and manage them as well as manage packages
  - Contributors can manage packages
- Allow organizations to own packages
- Add user registration, login, logout, and unregistering
- A CLI that can interact with the server to manage everything

## What "AUR-level support" means
- RPC v5: `info`/`multiinfo`, `search` with every `by=` field (name, name-desc,
  maintainer, depends, makedepends, optdepends, checkdepends, submitter, provides,
  conflicts, replaces, keywords, groups, comaintainers), `msearch`, `suggest`,
  `suggest-pkgbase`; path and legacy `/rpc?v=5&type=` forms; GET and POST.
- Git: clone of `<pkgbase>.git` per package base; snapshot tarballs at
  `/cgit/aur.git/snapshot/<pkgbase>.tar.gz`; ssh push with `.SRCINFO` parsed on push.
- Metadata dumps: `packages.gz`, `pkgbase.gz`, `packages-meta-v1.json.gz`
  (later `packages-meta-ext-v1.json.gz`, `users.gz`).
- Community features: votes, notifications, comments, flag out-of-date, keywords,
  co-maintainers, adopt/disown, requests (orphan, deletion, merge), package pages.

## Roadmap
1. **Phase 1 (v0.1.0, done):** workspace, core library, `.SRCINFO` parser, storage,
   RPC v5, dumb-HTTP git clone, snapshots, dumps, `taur import`, accounts
   (register/login/logout/unregister) JSON API, roles/orgs model.
2. **v0.2.0 (done):** `taurctl` management CLI; org, membership and package ownership
   API (adopt, disown, transfer).
3. **v0.3.0 (done):** pushes over smart HTTP and SSH with aurweb-style validation,
   SSH keys, smart-HTTP clone.
4. **v0.4.0 (done):** email verification, password reset, SMTP/file mailer.
5. **v0.5.0 (done):** `Organization` field in RPC.
6. **v0.6.0 (done):** votes/popularity, flags, comments, co-maintainers, requests,
   site admins. Remaining from phase 2: notification subscriptions, comment pinning,
   auto-accepting orphan requests, `packages-meta-ext-v1.json.gz`.
7. **v0.7.0 (done):** static web UI over the JSON API (bearer tokens).
8. **Phase 2 (rest):** notification subscriptions, comment pinning, orphan auto-accept,
   extended metadata dump.
9. **Phase 5 — MCP:** MCP server exposing search/info and authenticated management.
10. **Phase 6 — gRPC:** tonic service mirroring the JSON API.

## Owner decisions (2026-10-31)
- Pushes via both git smart HTTP (account token) and SSH keys.
- Public domain: taur.weirdbrains.com (hosting and TLS proxy TBD).
- Web UI uses bearer tokens; add email verification and password reset.
- Bundled protoc is fine for gRPC.
- Order: phase 2 AUR features, web UI, (org API done), MCP, gRPC.
- Keep AUR fields as-is and add a TAUR-specific owning-organization field to RPC.

## Decisions
- *Proposal:* axum 0.8 + tokio for HTTP; clap for the CLI.
- *Proposal:* SQLite via `rusqlite` with the bundled feature; one DB at
  `<home>/taur.db`, git repos at `<home>/repos/<pkgbase>.git`.
- *Proposal:* workspace = `taur-core` (library, all logic) + `taur-server` (binary `taur`).
  MCP/gRPC/static site become further thin crates.
- *Proposal:* git repos are managed with the system `git` binary (already installed),
  served via git's dumb HTTP protocol for now; smart HTTP/ssh in phase 2.
- *Proposal:* passwords hashed with Argon2id; sessions are random 256-bit bearer tokens,
  stored as SHA-256 hashes, 30-day expiry.
- *Proposal:* unregistering orphans maintained packages (as AUR does) and is refused while
  the user is the only Owner of an organization.
- *Proposal:* Admins cannot grant or modify the Owner role; every org needs an Owner.
- *Proposal:* gRPC via tonic with `protoc-bin-vendored` so no system protoc is needed.
- *Proposal:* the management CLI is a separate binary `taurctl` (crate `taur-cli`) that
  only talks HTTP; `taur` stays the server/admin binary. Blocking `ureq` client, token in
  a 0600 file in the CLI config dir.
- *Proposal:* SSH is an embedded russh server (`taur serve --ssh-bind`), never system sshd.
- *Proposal:* git smart HTTP runs through the system `git http-backend`; push validation
  runs in repo hooks that call `taur hook …`.
- *Proposal:* mail via lettre SMTP (rustls) configured by `TAUR_SMTP_URL`; file outbox
  when unset. Provider still to be chosen.
- *Proposal:* site admins (`users.is_admin`, granted with `taur admin`) play the AUR
  Package Maintainer role: accept requests and manage any package.
- *Proposal:* the web UI is plain HTML/CSS/JS embedded with `include_str!`, routed
  client-side; bearer token in localStorage; strict CSP.
