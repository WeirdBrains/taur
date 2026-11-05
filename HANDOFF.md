# TAUR handoff

TAUR is an AUR-compatible package repository. `yay` and `paru` can search, inspect,
and clone from it with `--aururl` and `--aurrpcurl`. This repository
([WeirdBrains/taur](https://github.com/WeirdBrains/taur)) is the primary project:
history, crate layout, binary names (`taur`, `taurctl`), and conventions stay here.
The private Origin project `michael-christenson-ii/TAUR` (called aurd in the port
request) was the feature reference. This checkout could not clone it: the hosted
agent had no `CURSOR_AUTH_TOKEN`, and Origin refuses stored logins on this VM
(`origin auth status` → not authenticated; `https://origin.cursor.com/.../TAUR.git`
returns 401). The gap analysis below compares taur 0.12.1 with aurweb and with the
capability list given for aurd, then records what 0.13.0 added.

## Architecture

Workspace crates (edition 2024, version in the root `Cargo.toml`):

| Crate | Binary | Role |
|---|---|---|
| `taur-core` | — | All business logic: SQLite, git repos, RPC shapes, accounts, orgs, pushes, dumps, git-serve decisions |
| `taur-server` | `taur` | HTTP (axum), embedded SSH (russh), gRPC (tonic), `taur ssh-auth` / `taur git-serve` |
| `taur-cli` | `taurctl` | Management client. Talks only to the HTTP API. Token in a 0600 file. |

Data directory (`--home` / `TAUR_HOME`, default `~/.local/share/taur`):

- `taur.db` — SQLite (bundled `rusqlite`). Passwords are Argon2id; session tokens are stored as SHA-256.
- `repos/<pkgbase>.git` — one bare repo per package base, branch `master`.
- `outbox/` — mail files when SMTP is unset.
- `ssh_host_ed25519_key` — host key for the embedded SSH server (0600).

### Where each surface lives

- **RPC** — `crates/taur-core/src/rpc.rs` (shapes and errors) and routes in `crates/taur-server/src/lib.rs`: legacy `GET`/`POST /rpc`, `/rpc/v5/{type}`, `/rpc/v5/{type}/{arg}`. JSONP via `callback`. Docs: `docs/rpc.md`.
- **Git HTTP** — `crates/taur-server/src/githttp.rs` plus `taur-core` `repo.rs`. Smart HTTP at `/{pkgbase}.git` through `git http-backend`. Push auth is HTTP Basic (account + session token) or a Bearer token. Hooks call `taur hook`.
- **Git SSH** — embedded: `crates/taur-server/src/ssh.rs` (`taur serve --ssh-bind`). OpenSSH: `taur ssh-auth %t %k` prints `authorized_keys`; `taur git-serve --user NAME` reads `SSH_ORIGINAL_COMMAND`. Command behavior: `taur-core/src/gitserve.rs`. Docs: `docs/ssh-keys.md`, `docs/git-and-snapshots.md`.
- **Snapshots and plain files** — `GET /cgit/aur.git/snapshot/<pkgbase>.tar.gz` and `GET /cgit/aur.git/plain/<path>?h=<pkgbase>`.
- **Metadata dumps** — `taur-core/src/dump.rs`: `/packages.gz`, `/pkgbase.gz`, `/packages-meta-v1.json.gz`, `/packages-meta-ext-v1.json.gz`.
- **gRPC** — `crates/taur-server/proto/taur/v1/taur.proto`, `src/grpc.rs`, `taur serve --grpc-bind`. Bearer metadata. Docs: `docs/grpc.md`.
- **MCP** — streamable HTTP `POST /mcp` (`src/mcp.rs`); stdio via `taurctl mcp`. Docs: `docs/mcp.md`.
- **Web UI** — static HTML/JS/CSS in `crates/taur-server/web/`, embedded in the binary, client-routed. Search, package pages, `/users/<name>` maintainer pages, login, registration, account and SSH keys, orgs, requests. Docs: `docs/web-ui.md`.
- **Storage and rules** — `store.rs`, `accounts.rs`, `orgs.rs`, `community.rs`, `roles.rs`, `push.rs`, `srcinfo.rs`.

`taurctl` never opens the database. Front ends stay thin.

## Build, test, and run

```sh
cargo fmt --check
cargo build --release
cargo clippy --all-targets -- -D warnings
cargo test
# man pages, after a CLI or version change:
TAUR_UPDATE_MAN=1 cargo test -p taur-cli man_pages
TAUR_UPDATE_MAN=1 cargo test -p taur-server man_pages
```

Rust ≥ 1.99 (this tree uses edition 2024). Tests and manual runs must use a temp
home (`--home` / `TAUR_HOME`) and a temp `taurctl` config (`--config-dir` /
`TAURCTL_CONFIG_DIR`). Do not point those at a real data directory or at
`~/.config/taurctl`.

Local server and yay (yay's own config stays untouched; pass flags only):

```sh
cargo build --release
./target/release/taur --home /tmp/taur-dev import path/to/pkgdir   # PKGBUILD + .SRCINFO
./target/release/taur --home /tmp/taur-dev serve --bind 127.0.0.1:8080
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Ss hello
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Si hello-taur
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -G hello-taur
```

Quote `--aurrpcurl` so the shell does not eat the `?`. paru uses the same RPC and git URLs.
SSH: `taur serve --ssh-bind 127.0.0.1:2222`, register a key with `taurctl key add`, then
`git clone ssh://taur@127.0.0.1:2222/<pkgbase>.git`. The SSH user name is ignored; the key
is the account.

Headless UI check (needs local `chromium`, ignored by default):

```sh
cargo test -p taur-server -- --ignored
```

## Config and secrets

Copy `.env.example` to a file you source, or use a systemd `EnvironmentFile=`. Flags
override environment variables. Full table: `docs/configuration.md`.

| Variable | Purpose |
|---|---|
| `TAUR_HOME` | Data directory |
| `TAUR_BIND` | HTTP listen address |
| `TAUR_BASE_URL` | Links in mail and the public base URL |
| `TAUR_SSH_BIND`, `TAUR_SSH_PUBLIC_HOST`, `TAUR_SSH_PUBLIC_PORT` | Embedded SSH |
| `TAUR_GRPC_BIND` | gRPC (plaintext HTTP/2; put TLS in front) |
| `TAUR_SMTP_URL` | SMTP URL. Unset → `TAUR_HOME/outbox`. Never log this |
| `TAUR_MAIL_FROM` | From address |
| `TAUR_ADMINS` | Comma-separated users granted site admin at startup (grant only) |
| `TAUR_SERVER`, `TAURCTL_CONFIG_DIR` | `taurctl` |

Secrets: passwords and session tokens are never logged, returned after issue, or
committed. The database stores Argon2id password hashes and SHA-256 token hashes.
OpenSSH must have `TAUR_HOME` in the `AuthorizedKeysCommand` environment; sshd does
not forward the operator's shell environment. Do not expose `taur git-serve` to
untrusted local users — sshd's forced command is the trust boundary (same model as
aurweb).

## Gap analysis

### taur 0.12.1 already had (kept)

- AUR RPC v5: `info`/`multiinfo`, `search` with every `by` field aurweb documents, `msearch`, `suggest`, `suggest-pkgbase`, legacy `/rpc` and `/rpc/v5/{type}/{arg}`, GET and POST form bodies, aurweb error strings for version, type, `by`, short query, and too many results.
- Smart HTTP clone and push at `/{pkgbase}.git`, hook checks modelled on aurweb's git-update (master only, fast-forward, `PKGBUILD` + `.SRCINFO`, no subdirectories, 256 KiB blobs, pkgname uniqueness).
- cgit snapshot tarballs, `packages.gz`, `pkgbase.gz`, `packages-meta-v1.json.gz`, and the extended dump.
- gRPC (tonic, vendored protoc, `proto/taur/v1/taur.proto`) and MCP (HTTP `/mcp` and `taurctl mcp` stdio).
- Embedded SSH with registered keys. Push allowed for maintainer, co-maintainer, org members who can manage the package, and site admins.
- Accounts (register, verify, reset, unregister), orgs and roles, votes and popularity, out-of-date flags, comments, notifications, package requests.
- Client-side web UI for search, package pages, login, SSH keys, orgs, and requests.
- `taurctl` over the JSON API.

Those implementations stayed. The port only filled holes and AUR-compatibility edges.

### What aurd's requested list added, and what 0.13.0 did

| Capability | taur 0.12.1 | 0.13.0 |
|---|---|---|
| JSONP (`callback`) | missing | `/**/name(...)`, aurweb's callback pattern, `Invalid callback name.` |
| aurweb `arg` / `arg[]` last-key rule | collected every arg in order | legacy and typeless v5 routes follow aurweb |
| JSON `POST /rpc/v5/info` and `/search` | form bodies only | array `arg` for info, string `arg`/`by` for search, aurweb 400 strings |
| cgit plain URLs | snapshots only | `/cgit/aur.git/plain/<path>?h=<pkgbase>` |
| OpenSSH `AuthorizedKeysCommand` | embedded russh only | `taur ssh-auth` |
| git-serve commands | upload-pack and receive-pack only | help, list-repos, adopt, disown, flag, unflag, vote, unvote, set-keywords, set-comaintainers |
| Maintainer pages | search `by=maintainer` only | `/users/<name>` in the existing UI |
| Server-rendered HTML | client-rendered SPA | kept the SPA (see skipped) |

### Versus aurweb (still different on purpose)

- One git repository per package base, not one `aur.git` with `GIT_NAMESPACE`. `?h=` on plain URLs is the package base name.
- `Organization` on RPC results is a TAUR field. AUR clients ignore it.
- SSH `adopt` adopts immediately. aurweb files an adoption request for a Package Maintainer.
- SSH `disown` orphans the package. aurweb promotes the first co-maintainer.
- No `restore` of a deleted package base (aurweb can replay a removed repo).
- No IP bans, maintenance mode, or RPC rate limit (aurweb returns `Rate limit reached` with HTTP 429).
- Popularity is recomputed in-process on an hourly timer. It is aurweb-shaped (vote decay) but not a byte-for-byte copy of aurweb's cron.
- Push validation requires `PKGBUILD` and a parseable `.SRCINFO` whose pkgbase matches the repo. It does not parse the `PKGBUILD` or prove the two files agree.
- Web UI is a static client of the JSON API (bearer token in `localStorage`), not aurweb's server-rendered PHP/Python pages.

### What was skipped, and why

- **Line-by-line aurd port.** The Origin repo was not readable from this agent (no injected Origin session). Behavior was taken from aurweb and from the capability list in the port request, and fitted to taur's modules instead of replacing them.
- **Replacing the web UI with server-rendered pages.** Search, package, login, and SSH key management already existed. Only the maintainer page was missing; it was added to the same client.
- **`restore`.** There is no archive of deleted package bases to put back.
- **aurweb's "adopt files a request" SSH path.** `Store::adopt` already takes the orphan; git-serve calls that.
- **Promoting a co-maintainer on disown.** `Store::disown` already orphans; git-serve calls that.
- **Rate limiting.** Called out below; not part of the compatibility fixes yay needs for a private instance.
- **A live `yay`/`paru` binary in CI.** This environment has neither, and the project rules say not to install packages or touch a real yay config. `crates/taur-server/tests/http.rs` issues the RPC queries yay sends (`type=search&by=name-desc`, `type=info&arg[]=`, `type=suggest`) and checks smart HTTP plus plain/snapshot URLs. `tests/push.rs` still pushes with real `git` over HTTP and the embedded SSH server. `tests/gitserve.rs` runs `taur ssh-auth` and `taur git-serve`. gRPC and MCP tests were already in `tests/grpc.rs` and `tests/http.rs`.

## Known issues and TODOs

- **Votes and popularity.** Voting works (HTTP, UI, `taurctl`, git-serve) and popularity is refreshed hourly. Confirm the decay formula against current aurweb if a mirror must match aur.archlinux.org numbers. There is no separate popularity cron unit.
- **Out-of-date flags.** Flag, unflag, maintainer mail, and the RPC `OutOfDate` timestamp work. git-serve requires a comment of at least 3 characters; the HTTP API only requires a non-empty comment. Flagging your own package is allowed.
- **Comments.** Add, edit, delete, and pin work for signed-in users. No comment notifications beyond the existing subscriber mail, and no aurweb-style rendered comment HTML (the UI inserts text nodes).
- **Notifications.** Subscribers are mailed on push and comment when SMTP or the file outbox is configured. There is no notification inbox in the UI, only email.
- **Package requests.** Orphan, deletion, and merge requests exist; site admins accept or reject them; orphan requests auto-accept after the out-of-date window (see `docs/community.md`). SSH `adopt` does not go through that queue.
- **Account registration.** Register, email verification, resend, password reset, and unregister work. Unverified users can sign in, clone, and `list-repos`, and cannot push or run other git-serve commands. There is no captcha or invite gate.
- **Rate limiting.** Not implemented. aurweb limits RPC by IP. A public deployment needs a proxy limit or an in-process limiter that returns `Rate limit reached` with HTTP 429.
- **`.SRCINFO` versus `PKGBUILD`.** The push hook checks that both files exist and that `.SRCINFO` parses. It does not re-parse the `PKGBUILD` (no bash) and does not reject a `PKGBUILD` that disagrees with `.SRCINFO`.
- **Deleted packages.** Deletion removes the repo. There is no `restore`.
- **OpenSSH `PermitUserEnvironment`.** Not required. The account name is inside the forced `command=`, not an `environment=` option.
- **Windows.** File modes and the embedded SSH server are best-effort. `AuthorizedKeysCommand` is an OpenSSH-on-Unix setup. CI historically cross-checks the Windows gnu target; re-run that when touching `std::fs` permissions.

## Suggested next steps

1. Clone `michael-christenson-ii/TAUR` from Origin on a machine with `origin auth login` (or an injected `CURSOR_AUTH_TOKEN`) and diff it against this branch. Port any aurd behavior this gap analysis could not see, still inside `taur-core` rather than as a second server.
2. Add RPC rate limiting compatible with aurweb's `Rate limit reached` / HTTP 429, keyed by client IP, with the limit configurable and disabled in tests.
3. Decide whether SSH `adopt` should file an orphan request (aurweb) or keep the immediate adopt. If you switch, keep one code path in `taur-core` and call it from HTTP, gRPC, and git-serve.
4. Optionally promote the first co-maintainer on disown, behind the same core function the HTTP API uses, so the two interfaces do not diverge.
5. Reject pushes whose `.SRCINFO` does not match the `PKGBUILD` metadata you care about (pkgver, deps). That implies a careful `PKGBUILD` reader, not `bash -c`.
6. Add `restore` only if you start retaining deleted repos (a quarantine directory under `TAUR_HOME`).
7. Put TLS and the OpenSSH `Match User` snippet from `docs/ssh-keys.md` in front of a public `taur.weirdbrains.com`, then run `yay` and `paru` against it from a temp `XDG_CONFIG_HOME`.
8. After merge, tag `v0.13.0` from `main` (`release: v0.13.0` is already described in `CHANGELOG.md`; the annotated tag is the owner's release step).
