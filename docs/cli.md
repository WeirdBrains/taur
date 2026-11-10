# taurctl — management CLI

`taurctl` (crate `taur-cli`) is a client for a running TAUR server. Most commands talk
only to its HTTP API ([RPC](rpc.md), [accounts](accounts.md), [management API](management-api.md)).
`taurctl upgrade` is a local operator command: it downloads a published native package
from GitHub, verifies `SHA256SUMS`, and installs it with the system package manager.
The server-side binary stays `taur` (`serve`, `import`); a client machine does not need
the server's SQLite/git data directory.

## Settings
- Server URL: `--server URL`, else `TAUR_SERVER`, else `server` in the config file, else
  `http://127.0.0.1:8080`. Save one with `taurctl config set-server URL`.
- Config dir: `--config-dir DIR`, else `TAURCTL_CONFIG_DIR`, else
  `$XDG_CONFIG_HOME/taurctl` (`~/.config/taurctl`). Holds `config.json` and `token`.
- The session token is written to `token` with mode 0600 (dir 0700) and never printed.
- Global flags: `--json` (raw JSON output), `-y/--yes` (skip confirmations),
  `--password-stdin` (read the password from stdin's first line instead of prompting).

## Commands
```
taurctl config show | config set-server URL
taurctl register NAME --email EMAIL      # prompts for password twice
taurctl login NAME | logout | whoami
taurctl verify TOKEN | resend-verification
taurctl reset-password request EMAIL | reset-password confirm --token TOKEN
taurctl unregister                       # password + confirmation (or -y)
taurctl search TERM [--by FIELD]
taurctl info PKG...
taurctl org list | create ORG | show ORG | delete ORG
taurctl org add-member ORG USER [--role contributor|admin|owner]
taurctl org set-role ORG USER ROLE | remove-member ORG USER
taurctl pkg list | adopt PKGBASE | disown PKGBASE
taurctl pkg transfer PKGBASE (--org ORG | --user USER)
taurctl pkg upload ARCHIVE [--org ORG]
taurctl key list | add FILE|- | remove ID
taurctl pkg vote|unvote|notify|unnotify|flag|unflag|comments|comment|comaintainers …   # see community.md
taurctl comment edit|delete|pin|unpin …
taurctl mcp                                     # MCP over stdio, see mcp.md
taurctl request file|list|show|accept|reject …
taurctl upgrade [--version X] [--restart] [--dry-run]   # local package install, see below
```
`unregister`, `org delete` and `pkg disown` ask for confirmation; without a terminal
they refuse unless `-y` is given. `taurctl upgrade` also asks before installing (and
before a `--restart`) unless `-y` is given.

## Upgrade the server package

`taurctl upgrade` is for the machine that already has TAUR installed from a native
package. It is not an HTTP call to a TAUR server.

```sh
taurctl upgrade
taurctl upgrade --version 1.5.0
taurctl upgrade --version 1.5.0 --restart -y
taurctl upgrade --version 1.5.0 --dry-run
```

- Only Linux x86_64 is supported. The matching published asset is used: a `.deb` with
  `apt-get` (Debian/Ubuntu), an `.rpm` with `dnf` (Fedora/RHEL), or a `.pkg.tar.zst`
  with `pacman` (Arch). Another architecture or an unmatched distro is an error; the
  command will not install a package for the wrong arch.
- The selected file is checked against the release `SHA256SUMS` before anything is
  installed. Root or `sudo` is required to run the package manager.
- `/etc/taur` and `/var/lib/taur` (configuration, the SQLite database, and git
  repositories) are left in place by the package manager. This command does not move
  the database.
- The service is not restarted unless you pass `--restart`. After a restart it checks
  `GET /api/v1/server` (default `http://127.0.0.1:<TAUR_BIND port>`). If that fails and
  the previously installed package was retained, that package is reinstalled and the
  service is started again.
- A package rollback cannot undo a database schema migration. Keep a pre-upgrade
  copy of the database if you may need to go back. `--dry-run` prints the plan
  without downloading or installing.

## Exit status
0 success, 1 error (server refusal, network, input), 2 usage error, 3 not logged in or
invalid credentials/session.

## Man pages
Generated from the clap definitions into `man/` (`taurctl.1`, `taurctl-org-add-member.1`,
…). Regenerate with `TAUR_UPDATE_MAN=1 cargo test -p taur-cli man_pages`; a test fails
when they are out of date.

## Example
```sh
taurctl config set-server https://packages.taur.weirdbrains.com
taurctl register alice --email alice@example.com
taurctl login alice
taurctl org create weirdbrains
taurctl org add-member weirdbrains bob --role admin
taurctl pkg transfer hello-taur --org weirdbrains
taurctl pkg upload ./webup.tar.gz
taurctl pkg upload ./webup.tar.gz --org weirdbrains
taurctl upgrade --version 1.5.0 --dry-run
```

`taurctl pkg upload` takes a `.tar.gz`, `.tar`, or `.zip` with `PKGBUILD` and `.SRCINFO`,
the same archive as [Upload](guides/publish.md) on the site. `--org` is the organization's
exact name (3-16 characters of `A-Z` `a-z` `0-9` `.` `_` `-`, starting and ending
alphanumeric), the same rule as `taurctl org create`. You must be an Owner, Admin, or
Contributor of that organization. Omit `--org` to publish a new package under your
account. An update keeps the current owner; if `--org` is given then, it must already
be that package's organization or the upload is refused and nothing is moved.
