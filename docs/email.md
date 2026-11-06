# Email, verification and password reset

## Mail delivery
`taur serve` picks a mailer at startup:
- **SMTP** when `--smtp-url` / `TAUR_SMTP_URL` is set, e.g.
  `smtps://user:password@smtp.example.com` (implicit TLS, port 465) or
  `smtp://user:password@smtp.example.com:587?tls=required` (STARTTLS). Uses lettre with
  rustls. The URL is never logged; prefer the env var so it stays out of `ps` output.
- **File outbox** otherwise: each message is written to `<home>/outbox/*.eml` (0600).
  Meant for development and tests.

`--mail-from` / `TAUR_MAIL_FROM` (default `TAUR <noreply@taur.weirdbrains.com>`) sets
the sender; `--base-url` / `TAUR_BASE_URL` (derived from `--domain` or the listen address; see [configuration](configuration.md)) is
used for links.

## Verification
Registering sends a link `<base>/verify?token=…` (valid 24 h, single use). Until
verified, an account can sign in, view and resend, but can't push, add SSH keys, or
manage organizations or packages (403 `email address not verified`).

| Method | Path | Body | Result |
|---|---|---|---|
| POST | `/api/v1/verify` | `{"token"}` | 200 account |
| POST | `/api/v1/verify/resend` | — (Bearer) | 202 |

Admins can run `taur verify-user NAME` on the server.

## Password reset
| Method | Path | Body | Result |
|---|---|---|---|
| POST | `/api/v1/password-reset` | `{"email"}` | 202 (always, so accounts can't be probed) |
| POST | `/api/v1/password-reset/confirm` | `{"token","password"}` | 204 |

Reset links are valid 1 hour and single use. Confirming sets the new password, signs out
every session and marks the address verified.

## Security
Tokens are 256-bit random values; only SHA-256 hashes are stored. Tokens and passwords
are never logged. Errors: 400 invalid/expired token or short password, 409 already
verified.

CLI: `taurctl verify TOKEN`, `taurctl resend-verification`,
`taurctl reset-password request EMAIL`, `taurctl reset-password confirm --token TOKEN`.
