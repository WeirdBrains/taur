# SSH keys

Public keys (OpenSSH format: ed25519, ecdsa, rsa) identify you for git over SSH.
Keys are unique across accounts and stored with their `SHA256:` fingerprint.

| Method | Path | Body | Result |
|---|---|---|---|
| GET | `/api/v1/ssh-keys` | | `[{"id","fingerprint","algorithm","comment","created_at"}]` |
| POST | `/api/v1/ssh-keys` | `{"public_key": "ssh-ed25519 AAAA… comment"}` | 201 key |
| DELETE | `/api/v1/ssh-keys/{id}` | | 204 |

CLI: `taurctl key add ~/.ssh/id_ed25519.pub`, `taurctl key list`, `taurctl key remove ID`.

Errors: 400 not a valid key, 409 key already registered, 404 unknown id.
