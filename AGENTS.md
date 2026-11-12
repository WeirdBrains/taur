# AGENTS.md — rules for agents working on TAUR

Adapted from `~/Projects/WeirdBrains/muninn/AGENTS.md`.

## Where to work
- Always work directly on the owner's machine (alien-blue), in
  `~/Projects/WeirdBrains/TAUR`. No cloud agents, no clones elsewhere.
- Toolchain: Rust >= 1.99 (edition 2024), cargo on this machine. Prefer bundled/vendored
  crate features over installing system packages (e.g. bundled SQLite; a vendored `protoc`
  crate for gRPC); ask before installing anything.

## Source of truth
- `PLAN.md` holds the owner's plan and the phased roadmap. Items marked *proposal* are
  agent decisions the owner may override; record changes there and in `docs/`.
- `taur-core` holds all business logic. The HTTP server (`taur`), the management CLI
  (`taurctl`, a client of the HTTP API) and future front ends (static site, MCP, gRPC)
  are thin layers over it.

## Safety
- Never touch a real TAUR data dir or the real `~/.config/taurctl`. Tests, examples and manual runs use a temp dir via
  `--home` / `TAUR_HOME` and `--config-dir` / `TAURCTL_CONFIG_DIR`.
- Never touch the owner's real tool configs (e.g. yay); use temp `XDG_CONFIG_HOME` /
  `XDG_CACHE_HOME` for manual checks, and never install packages while testing.
- Never edit files outside this project folder (cargo caches excepted).
- Git: always go in this order: test, commit, tag, push. Commit in logical steps once the
  quality bar passes, tag releases, then push (`git push origin main --follow-tags`).
  Do **not** change remotes, force-push, rewrite published history, or skip hooks.
- Secrets (passwords, session tokens, keys) are never logged, printed, returned after
  issue, or committed. Store only hashes (Argon2id for passwords, SHA-256 for tokens).

## Versioning and releases
- Semantic versioning. Pre-1.0: MINOR bump per new feature, PATCH for fixes/docs-only.
- Each release: bump the workspace version in `Cargo.toml` (+ `Cargo.lock`), regenerate
  man pages, add a
  `CHANGELOG.md` entry, commit as `release: vX.Y.Z`, create an annotated tag `vX.Y.Z`,
  then push main with the tag.
- `python3 scripts/weirdnet_release.py` (the shared WeirdBrains release script, vendored from
  weirdnet-project-helpers; configured by `release.toml`) does all of this. `--dry-run` first.
  Don't edit the vendored copy; change weirdnet-project-helpers and re-vendor it. See
  `docs/releases.md`.
- `CHANGELOG.md` (Keep a Changelog format) stays **summary level**: a few short one-line
  bullets per release naming the feature. No implementation details, file lists or
  sub-bullets. Tag messages are one short line.

## Documentation
- Every feature is documented in `docs/` (one Markdown file per feature, indexed in
  `docs/README.md`): how it works, flags/defaults, examples, failure modes. Update docs in
  the same change as the code.
- Man pages for `taurctl` are generated from the clap definitions (clap_mangen) into
  `man/`. Regenerate whenever the CLI or version changes
  (`TAUR_UPDATE_MAN=1 cargo test -p taur-cli man_pages`); the out-of-date test must pass.

## Quality bar (before every commit)
- `cargo fmt --check`, `cargo build`, `cargo clippy --all-targets` (no warnings),
  `cargo test` all pass.
- New behaviour comes with tests. HTTP behaviour is tested through the router
  (`tower::ServiceExt::oneshot`) against a temp home.

## Communication style
- Keep summaries, changelog entries and commit messages concise.
