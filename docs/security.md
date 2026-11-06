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

`TAUR_RPC_RATE_LIMIT` counts RPC requests per TCP connection peer. Over the limit the
status is 429 and the JSON error is `Rate limit reached`. `X-Forwarded-For`,
`X-Real-IP`, and `Forwarded` are not read. Each process keeps its own memory. A reverse
proxy makes every client look like the proxy unless the proxy enforces its own limit.

## PKGBUILD checks

Push validation parses `.SRCINFO` and compares simple `key=value` lines in the PKGBUILD.
It does not run a shell. `TAUR_PKGBUILD_ISOLATION=bwrap` is the only way a push runs
`makepkg --printsrcinfo`, and only inside bubblewrap with `/usr` read-only, a private
network namespace, no home directory, and a 20 second limit. If `bwrap` is not
installed the push is refused. Bubblewrap is not a virtual machine; a bug in `bwrap`
or `makepkg` is still a risk. Leave the variable unset unless you want that check.

## Residual risks

- HTTP and gRPC are plaintext. Put TLS on a proxy. The server does not terminate TLS.
- The web UI keeps the bearer token in `localStorage`. Script injection on the origin
  can use the token. The API does not use a cookie session.
- Rate-limit state is not shared across processes and collapses to the proxy address.
- The literal PKGBUILD check misses assignments that use `$` or backticks, and it does
  not prove `.SRCINFO` was produced by that PKGBUILD.
- Deleted repositories remain on disk under `repos/.deleted` until restored or removed
  by the operator. Filesystem permissions on `TAUR_HOME` are the boundary.
- The embedded SSH server and OpenSSH `command=` integration trust the forced command
  the way aurweb does. Do not expose `taur git-serve` as a normal shell.
- No dependency-advisory scan is part of `cargo test`.
