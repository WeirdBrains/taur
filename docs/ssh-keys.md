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

## OpenSSH (`AuthorizedKeysCommand`)

The embedded server (`taur serve --ssh-bind`) is enough for git over SSH. To use the
system `sshd` instead, point `AuthorizedKeysCommand` at `taur ssh-auth`. It prints one
`authorized_keys` line when `%t` `%k` is a registered key, and prints nothing (exit 0)
otherwise. The forced command is `taur git-serve --user NAME`, which reads
`SSH_ORIGINAL_COMMAND`. Run it as a user that can read the TAUR data directory
(`TAUR_HOME` or `--home`).

```
Match User aur
    AuthorizedKeysCommand /usr/local/bin/taur ssh-auth %t %k
    AuthorizedKeysCommandUser taur
```

`TAUR_HOME` must be set for that command (sshd does not pass your shell environment).
The login name (`aur`) is ignored; the key selects the account.

## git-serve commands

`ssh aur@host <command>` (embedded or OpenSSH) accepts:

| Command | Who | Effect |
|---|---|---|
| `git-upload-pack`, `git-receive-pack` | clone: anyone with a key; push: verified maintainer, co-maintainer, org manager, or site admin | git transfer. A new name creates the package base. |
| `list-repos` | any account | package bases you maintain, one per line |
| `help` | any account | command list on stderr |
| `vote` / `unvote <pkgbase>` | verified | one vote per account |
| `flag <pkgbase> <comment>` | verified | comment is one argument (quote it) and at least 3 characters |
| `unflag <pkgbase>` | flagger, maintainer, co-maintainer, or site admin | clears the flag |
| `adopt <pkgbase>` | verified | adopts an orphan immediately (TAUR does not file an adoption request) |
| `disown <pkgbase>` | someone who can manage it | orphans the package base (co-maintainers are not promoted) |
| `set-keywords <pkgbase> [keyword…]` | someone who can manage it | replaces keywords |
| `set-comaintainers <pkgbase> [user…]` | maintainer, org manager, or site admin | replaces co-maintainers |

An interactive shell is rejected. `restore` (aurweb's deleted-package recovery) is not
implemented. Unverified accounts can clone and list repos; other commands ask them to
verify their email.
