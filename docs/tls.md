# TLS and automatic certificates

`TAUR_TLS` / `--tls` chooses how `taur serve` speaks to clients.

| Value | Application port (`TAUR_BIND`) | gRPC (`TAUR_GRPC_BIND`) | When to use it |
|---|---|---|---|
| `off` (default) | Plaintext HTTP | Plaintext HTTP/2, not TLS | Local tests, or a reverse proxy you trust terminates TLS |
| `acme` | HTTPS, after a certificate exists | TLS with the same certificate | This process requests and renews a certificate |

`off` does not mean "try ACME and fall back". `acme` does not open the application port as HTTP when a certificate is missing or renewal fails. The previous certificate keeps being served if one was already installed. SSH is not TLS in either mode.

## Domain and the public URL

`--domain` / `TAUR_DOMAIN` is the public DNS name. Repeat the flag or separate names with commas. It is required for `acme`. It is optional for `off`. There is no built-in hostname: `packages.taur.weirdbrains.com` in other docs is an example of a package host, not a default and not a certificate this repository requests.

Precedence:

1. A command-line flag overrides the environment variable.
2. If `TAUR_BASE_URL` is set, that URL is advertised. When any domain is configured, its host must be one of those names. In `acme` mode it must be `https` and its port must be the `TAUR_BIND` port.
3. If it is unset and a domain is set and TLS is `acme`, the URL is `https://<first-domain>` and includes the bind port when that port is not 443.
4. If it is unset and a domain is set and TLS is `off`, the URL is `https://<first-domain>` (port 443). The process still listens with plaintext HTTP; this is the reverse-proxy case, where the proxy's certificate is expected to cover the names you listed. TAUR does not see that certificate.
5. If it is unset and no domain is set, the URL is `http://<bind>`. Unspecified addresses (`0.0.0.0`, `::`) are advertised as loopback.

Every configured domain is a name on the automatic certificate. The advertised host is always one of them when domains are set, so clone links and mail links do not name a host the certificate is not for.

## ACME (HTTP-01)

The client is [instant-acme](https://crates.io/crates/instant-acme). The challenge is HTTP-01 only. TAUR does not call certbot.

```sh
taur serve \
  --tls acme \
  --bind 0.0.0.0:443 \
  --domain pkg.example.com \
  --domain www.pkg.example.com \
  --acme-contact 'mailto:admin@example.com' \
  --acme-accept-terms yes \
  --acme-http-bind 0.0.0.0:80
```

`--acme-accept-terms` must be the word `yes`. Any other value, including leaving it unset, refuses to create an account or request a certificate, and does not contact the directory. Read the agreement the CA publishes. Let's Encrypt keeps its subscriber agreement at <https://letsencrypt.org/repository/>. There is no default contact address.

`--acme-staging` uses `https://acme-staging-v02.api.letsencrypt.org/directory`. Staging certificates are not publicly trusted. Do not set `--acme-staging` and `--acme-directory` together. Unset, the directory is Let's Encrypt production: `https://acme-v02.api.letsencrypt.org/directory`. Another `https://` directory URL is accepted.

What has to be reachable:

- DNS for every domain points at this host.
- TCP port 80 on those names reaches `TAUR_ACME_HTTP_BIND`. The CA fetches `http://<domain>/.well-known/acme-challenge/<token>` there. Other paths on that port redirect to the public HTTPS URL. The application is not served on it.
- TCP on `TAUR_BIND` (443 unless you chose another port) is the HTTPS service. Open it in the firewall when you want clients to connect.
- Outbound HTTPS to the ACME directory.

Nothing in the test suite accepts a CA agreement or requests a publicly trusted certificate. Tests use a local issuer and a self-signed certificate that is not a public CA.

## Keys on disk

`<TAUR_ACME_CACHE>` defaults to `<TAUR_HOME>/acme`, mode `0700`.

- `account.json` is the ACME account private key, mode `0600`. It is not logged.
- `current/cert.pem` is the chain. `current/cert.key` is the certificate private key, mode `0600`.
- A new certificate is written to a staging directory and renamed into `current` only after both files are stored. The replaced directory is kept as `previous`.
- Renewal starts `TAUR_ACME_RENEW_BEFORE` days before `not_after` (default 30, allowed 1–60). A failed renewal logs a short error without key material, keeps serving the current certificate, and retries with backoff (30 seconds, doubling, capped at 6 hours) until success or shutdown.
- The first certificate is retried the same way. Until it exists, the HTTPS port is not opened. Cancelling the process stops the retries.

The rustls config reloads the new chain for later handshakes. Existing connections are not torn down to force the new certificate.

## What stays plaintext

- `TAUR_TLS=off`: HTTP and gRPC are plaintext. Say so in front of a proxy; do not describe that gRPC port as TLS.
- Embedded SSH (`TAUR_SSH_BIND`) is SSH, not the HTTPS certificate.
- OpenSSH `command=` / `taur git-serve` is unchanged.
