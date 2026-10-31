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
2. **Phase 2 — pushing & community:** smart-HTTP and ssh push with `.SRCINFO` validation,
   votes, comments, flag out-of-date, co-maintainers, adopt/disown, requests, keywords API.
3. **Phase 3 — web UI:** static site for registration, sign-in, package management, over
   the JSON API.
4. **Phase 4 — organizations:** org CRUD and membership API/UI, org-owned packages.
5. **Phase 5 — MCP:** MCP server exposing search/info and authenticated management.
6. **Phase 6 — gRPC:** tonic service mirroring the JSON API.

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
