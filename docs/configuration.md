# Configuration and data directory

`taur [--home DIR] <command>`. Every deployment setting of `taur serve` can come from an
environment variable. **Precedence: command-line flag > environment variable > default.**
Values are validated at startup; an invalid value stops the server with an error naming
the variable (the SMTP URL is never printed or logged).

| Variable | Flag | Default | Meaning |
|---|---|---|---|
| `TAUR_HOME` | `--home` | `~/.local/share/taur` | Data directory: `taur.db` (SQLite), `repos/<pkgbase>.git`, `outbox/`, `ssh_host_ed25519_key`. Used by every `taur` command. |
| `TAUR_BIND` | `--bind` | `127.0.0.1:8080` | Listen address. Plaintext HTTP when `TAUR_TLS=off`. HTTPS when `TAUR_TLS=acme` (never downgraded). |
| `TAUR_TLS` | `--tls` | `off` | `off` or `acme`. `off` is local HTTP or a TLS reverse proxy. |
| `TAUR_DOMAIN` | `--domain` | unset | DNS names, comma-separated or repeated. Required for `acme`. First name is the default public host. Example: `packages.example.com`. |
| `TAUR_BASE_URL` | `--base-url` | derived | Public URL for mail and clone links. See precedence below. The project site is `https://taur.weirdbrains.com`. |
| `TAUR_ACME_DIRECTORY` | `--acme-directory` | Let's Encrypt production | ACME directory URL. Do not set this and `TAUR_ACME_STAGING`. |
| `TAUR_ACME_STAGING` | `--acme-staging` | `false` | `true` uses Let's Encrypt staging. Those certificates are not publicly trusted. |
| `TAUR_ACME_CONTACT` | `--acme-contact` | required for acme | `mailto:` address. No default. |
| `TAUR_ACME_ACCEPT_TERMS` | `--acme-accept-terms` | unset | Must be the word `yes` after you read the CA agreement. |
| `TAUR_ACME_HTTP_BIND` | `--acme-http-bind` | required for acme | HTTP-01 listen address. The CA connects to TCP port 80. |
| `TAUR_ACME_CACHE` | `--acme-cache` | `<home>/acme` | Account key and certificates. Directory `0700`, keys `0600`. |
| `TAUR_ACME_RENEW_BEFORE` | `--acme-renew-before` | `30` | Days before expiry to renew. |
| `TAUR_SSH_BIND` | `--ssh-bind` | unset = SSH off | Git-over-SSH listen address, e.g. `0.0.0.0:2222`. |
| `TAUR_SSH_PUBLIC_HOST` | `--ssh-public-host` | host of `TAUR_BASE_URL` | Host shown in SSH clone URLs. |
| `TAUR_SSH_PUBLIC_PORT` | `--ssh-public-port` | port of `TAUR_SSH_BIND` | Port shown in SSH clone URLs (port 22 is omitted). |
| `TAUR_GRPC_BIND` | `--grpc-bind` | unset = gRPC off | gRPC listen address. Plaintext HTTP/2 when TLS is off; the HTTPS certificate when `acme` ([grpc.md](grpc.md)). |
| `TAUR_SMTP_URL` | `--smtp-url` | unset = mail to `<home>/outbox` | `smtp://` or `smtps://user:pass@host[:port]`. Secret. |
| `TAUR_MAIL_FROM` | `--mail-from` | `TAUR <noreply@taur.weirdbrains.com>` | Sender address. |
| `TAUR_ADMINS` | `--admins` | unset | Comma-separated user names made site admins at startup. When this or `TAUR_OWNERS` is set, the first registration is not promoted. |
| `TAUR_OWNERS` | `--owners` | unset | Comma-separated user names made site owners at startup. Also grants site admin. |
| `TAUR_RPC_RATE_LIMIT` | `--rpc-rate-limit` | `300` | AUR RPC requests per window per client. `0` disables this class. |
| `TAUR_RPC_RATE_WINDOW` | `--rpc-rate-window` | `60` | Window for `TAUR_RPC_RATE_LIMIT`. |
| `TAUR_AUTH_RATE_LIMIT` | `--auth-rate-limit` | `20` | Login, registration, verification, password reset. `0` disables. |
| `TAUR_AUTH_RATE_WINDOW` | `--auth-rate-window` | `60` | Window for the auth class. |
| `TAUR_GIT_RATE_LIMIT` | `--git-rate-limit` | `60` | Git smart HTTP requests. `0` disables. |
| `TAUR_GIT_RATE_WINDOW` | `--git-rate-window` | `60` | Window for the git class. |
| `TAUR_API_RATE_LIMIT` | `--api-rate-limit` | `120` | JSON API, except auth and `/api/v1/server`. `0` disables. |
| `TAUR_API_RATE_WINDOW` | `--api-rate-window` | `60` | Window for the API class. |
| `TAUR_DUMP_RATE_LIMIT` | `--dump-rate-limit` | `12` | Snapshots and metadata dumps. `0` disables. |
| `TAUR_DUMP_RATE_WINDOW` | `--dump-rate-window` | `60` | Window for the dump class. |
| `TAUR_MCP_RATE_LIMIT` | `--mcp-rate-limit` | `60` | MCP requests. `0` disables. |
| `TAUR_MCP_RATE_WINDOW` | `--mcp-rate-window` | `60` | Window for the MCP class. |
| `TAUR_GRPC_RATE_LIMIT` | `--grpc-rate-limit` | `300` | gRPC, except account auth (that uses the auth class). `0` disables. |
| `TAUR_GRPC_RATE_WINDOW` | `--grpc-rate-window` | `60` | Window for the gRPC class. |
| `TAUR_SSH_RATE_LIMIT` | `--ssh-rate-limit` | `30` | New embedded SSH connections. `0` disables. OpenSSH `git-serve` is not covered. |
| `TAUR_SSH_RATE_WINDOW` | `--ssh-rate-window` | `60` | Window for the SSH class. |
| `TAUR_TRUSTED_PROXIES` | `--trusted-proxies` | empty | CIDRs allowed to send `X-Forwarded-For`. A `/0` prefix is rejected. |
| `TAUR_RATE_MAX_KEYS` | `--rate-max-keys` | `8192` | Client keys remembered per class. |

Notes:
- **Site admins:** `TAUR_ADMINS` only grants, idempotently, on every start. Removing a
  name does **not** revoke; use `taur admin NAME --revoke`. Names that are not registered
  yet log a warning and are granted on a later start.
- **SSH clone URLs** are `ssh://taur@HOST[:PORT]/<pkgbase>.git`; `GET /api/v1/server`
  returns `{version, base_url, ssh_clone_base}` and the web UI shows both clone URLs.
- **Public URL.** With no domain and `TAUR_TLS=off`, the base URL is `http://<bind>`
  (`0.0.0.0` and `::` are advertised as loopback). With a domain and TLS off, it is
  `https://<first-domain>` (port 443): the listener stays plaintext and a proxy is assumed
  to terminate TLS. With `TAUR_TLS=acme` and no base URL, it is `https://<first-domain>`
  plus the bind port when that port is not 443. An explicit base URL wins, but its host
  must be one of `TAUR_DOMAIN` when any domain is set. In `acme` mode the URL must be
  `https` and its port must be the bind port. `packages.taur.weirdbrains.com` is an
  example name, not a default.
- **TLS.** `TAUR_TLS=off` is plaintext, for local use or a reverse proxy you trust.
  `TAUR_TLS=acme` is HTTP-01 inside the process. Details, firewall rules, and what is
  not encrypted: [tls.md](tls.md).
- **Rate limits** are on by default for `taur serve`. Counters are per process. Without
  `TAUR_TRUSTED_PROXIES`, `X-Forwarded-For` is ignored and every client behind one proxy
  shares that proxy's address. List only the proxy CIDR, and configure the proxy to append
  the client. HTML pages and `/static` are not limited. See [rpc.md](rpc.md) and
  [security.md](security.md).
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
