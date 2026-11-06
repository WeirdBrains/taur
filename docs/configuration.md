# Configuration and data directory

`taur [--home DIR] <command>`. Every deployment setting of `taur serve` can come from an
environment variable. **Precedence: command-line flag > environment variable > default.**
Values are validated at startup; an invalid value stops the server with an error naming
the variable (the SMTP URL is never printed or logged).

| Variable | Flag | Default | Meaning |
|---|---|---|---|
| `TAUR_HOME` | `--home` | `~/.local/share/taur` | Data directory: `taur.db` (SQLite), `repos/<pkgbase>.git`, `outbox/`, `ssh_host_ed25519_key`. Used by every `taur` command. |
| `TAUR_BIND` | `--bind` | `127.0.0.1:8080` | HTTP listen address (`IP:PORT`). |
| `TAUR_BASE_URL` | `--base-url` | `https://packages.taur.weirdbrains.com` | Public URL of the package service (`http(s)://HOST[:PORT]`, no path) used in email links and clone URLs. The project site is `https://taur.weirdbrains.com`. |
| `TAUR_SSH_BIND` | `--ssh-bind` | unset = SSH off | Git-over-SSH listen address, e.g. `0.0.0.0:2222`. |
| `TAUR_SSH_PUBLIC_HOST` | `--ssh-public-host` | host of `TAUR_BASE_URL` | Host shown in SSH clone URLs. |
| `TAUR_SSH_PUBLIC_PORT` | `--ssh-public-port` | port of `TAUR_SSH_BIND` | Port shown in SSH clone URLs (port 22 is omitted). |
| `TAUR_GRPC_BIND` | `--grpc-bind` | unset = gRPC off | gRPC listen address, plaintext HTTP/2 ([grpc.md](grpc.md)). |
| `TAUR_SMTP_URL` | `--smtp-url` | unset = mail to `<home>/outbox` | `smtp://` or `smtps://user:pass@host[:port]`. Secret. |
| `TAUR_MAIL_FROM` | `--mail-from` | `TAUR <noreply@taur.weirdbrains.com>` | Sender address. |
| `TAUR_ADMINS` | `--admins` | unset | Comma-separated user names made site admins at startup. |
| `TAUR_RPC_RATE_LIMIT` | `--rpc-rate-limit` | `0` (off) | RPC requests allowed per window for each TCP peer. |
| `TAUR_RPC_RATE_WINDOW` | `--rpc-rate-window` | `60` | Rate-limit window in seconds. |

Notes:
- **Site admins:** `TAUR_ADMINS` only grants, idempotently, on every start. Removing a
  name does **not** revoke; use `taur admin NAME --revoke`. Names that are not registered
  yet log a warning and are granted on a later start.
- **SSH clone URLs** are `ssh://taur@HOST[:PORT]/<pkgbase>.git`; `GET /api/v1/server`
  returns `{version, base_url, ssh_clone_base}` and the web UI shows both clone URLs.
- **TLS** is terminated by a reverse proxy (e.g. Caddy or nginx) in front of `TAUR_BIND`
  and `TAUR_GRPC_BIND` (gRPC needs HTTP/2 to the backend, e.g. nginx `grpc_pass`). TAUR
  has no direct-TLS option. Links come from `TAUR_BASE_URL`. The RPC limiter keys on the
  socket peer and ignores `X-Forwarded-For`. Behind a proxy every client shares the
  proxy's address, so put a limit on the proxy as well when the in-process limit is on.
- Git hooks receive the mail settings from the server process, so they need no extra
  configuration.

[`.env.example`](../.env.example) at the repo root has a commented example entry for
every variable (copy to `.env`, which git ignores). A minimal production file for systemd
`EnvironmentFile=` is [`packaging/taur.env.example`](../packaging/taur.env.example).

Service configs for systemd, SysV init, OpenRC, runit, launchd and Windows are described
in [services.md](services.md).

Man pages for `taur` and `taurctl` (with ENVIRONMENT sections) are in `man/`.
`taurctl` reads `TAUR_SERVER` and `TAURCTL_CONFIG_DIR` ([cli.md](cli.md)).

Failure modes: unwritable home → startup error; bind in use → startup error; invalid
setting → startup error naming the variable.
