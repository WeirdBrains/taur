# User accounts

JSON API (logic in `taur_core::accounts`):

| Method | Path | Body | Result |
|---|---|---|---|
| POST | `/api/v1/register` | `{"name","email","password"}` | 201 `{"id","name","email"}` |
| POST | `/api/v1/login` | `{"name","password"}` | 200 `{"token","expires_at"}` |
| POST | `/api/v1/logout` | — (Bearer token) | 204 |
| GET | `/api/v1/account` | — (Bearer token) | 200 `{"id","name","email","verified"}` |
| DELETE | `/api/v1/account` | `{"password"}` (Bearer token) | 204 |

Authenticate with `Authorization: Bearer <token>`.

Rules: names 3–16 chars of `A-Za-z0-9._-`, start/end alphanumeric, unique
case-insensitively (as are emails); passwords ≥ 8 characters.

Security: passwords hashed with Argon2id (salted PHC string). Tokens are 256-bit random,
shown once at login, stored only as SHA-256; sessions expire after 30 days. Neither is
ever logged or returned again.

Unregistering requires the current password. It deletes the account, its sessions, org
memberships and co-maintainerships. Package bases it maintained become **orphans**
(maintainer cleared; submitter cleared), matching AUR, so others can adopt them. It is
refused (409) while the user is the only Owner of an organization.

Errors: 400 invalid input, 401 bad credentials/session, 409 taken name/email or sole owner.

```sh
curl -X POST localhost:8080/api/v1/login -H 'content-type: application/json' \
  -d '{"name":"alice","password":"..."}'
```

New accounts must verify their email address before pushing or managing anything; see
[email](email.md) for verification and password reset.
