# Security notes

This is a review of the code in this repository. It is not a
certification, a penetration test, or a claim that every bug has been found.

## What was reviewed

- HTTP RPC (GET, form POST, JSON POST, JSONP callback names) and the per-peer rate limit.
- Git smart HTTP (`git http-backend`), including `/{pkgbase}` and `/{pkgbase}.git`, and
  cgit plain-file paths (`..` is rejected).
- SSH command parsing and `git-serve` authorization (verified email, maintainer,
  co-maintainer, org member, site admin).
- gRPC bearer metadata and MCP bearer authentication.
- Account passwords (Argon2id) and session tokens (SHA-256 at rest; the raw token is
  returned once).
- Subprocess calls: fixed `git` and `bwrap` argument lists, `GIT_CONFIG_GLOBAL` pointed
  at the null device, hook environment limited to what the server sets.
- Restore moves a repository only when the stored path stays inside `repos/.deleted`.

## Rate limit

`taur serve` limits RPC, auth, git HTTP, the JSON API, dumps, MCP, gRPC, and new
embedded SSH connections. Defaults are in [configuration](configuration.md). `0` disables
one class. Over the limit: HTTP 429 with `Retry-After`, or gRPC `RESOURCE_EXHAUSTED`.
The RPC body stays `Rate limit reached`. Web pages, `/static`, and `/api/v1/server` are
not limited. `X-Forwarded-For` is ignored unless the TCP peer is in
`TAUR_TRUSTED_PROXIES` (a `/0` prefix is rejected). The trusted proxy must append the
client; otherwise every client behind it shares one bucket. State is per process and
capped by `TAUR_RATE_MAX_KEYS`. OpenSSH `taur git-serve` is a separate process and is
not limited here.

## PKGBUILD checks

Push validation parses `.SRCINFO` and compares simple `key=value` lines in the PKGBUILD.
It does not run a shell. `TAUR_PKGBUILD_ISOLATION=bwrap` is the only way a push runs
`makepkg --printsrcinfo`, and only inside bubblewrap with `/usr` read-only, a private
network namespace, no home directory, and a 20 second limit. If `bwrap` is not
installed the push is refused. Bubblewrap is not a virtual machine; a bug in `bwrap`
or `makepkg` is still a risk. Leave the variable unset unless you want that check.

`cargo test -p taur-server --test fixture_isolation` builds and installs
`tests/fixtures/hello-taur` inside that sandbox (`makepkg --nodeps`, then `pacman -dd`
into a temp root as uid 0 in a user namespace). It skips when `bwrap` or `makepkg` is
missing unless `TAUR_REQUIRE_ISOLATION=1`. The sandbox still reads the host `/usr` and
`/etc/makepkg.conf`. It does not install dependencies and does not prove a hostile
PKGBUILD cannot escape bubblewrap.

## Residual risks

- `TAUR_TLS=off` leaves HTTP and gRPC plaintext. `TAUR_TLS=acme` encrypts those two
  listeners only after a certificate exists; SSH is a separate protocol. A proxy in
  front of plaintext mode is still a valid deployment. See [tls.md](tls.md).
- The web UI keeps the bearer token in `localStorage`. Script injection on the origin
  can use the token. The API does not use a cookie session.
- Rate-limit state is not shared across processes. Without a trusted-proxy CIDR it
  collapses to the proxy address.
- The literal PKGBUILD check misses assignments that use `$` or backticks, and it does
  not prove `.SRCINFO` was produced by that PKGBUILD.
- Deleted repositories remain on disk under `repos/.deleted` until restored or removed
  by the operator. Filesystem permissions on `TAUR_HOME` are the boundary.
- The embedded SSH server and OpenSSH `command=` integration trust the forced command
  the way aurweb does. Do not expose `taur git-serve` as a normal shell.
- No dependency-advisory scan is part of `cargo test`.
- Official AUR and Arch identity checks are not implemented. Public AUR package info
  has a maintainer nickname and no email or signature. See [upload policy](upload-policy.md).
- Moderators can suspend a package or an ordinary account. Site admins reverse that
  from the audit. See [moderation](moderation.md).
