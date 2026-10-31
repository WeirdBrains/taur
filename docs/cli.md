# taurctl — management CLI

`taurctl` (crate `taur-cli`) is a client for a running TAUR server, talking only to its
HTTP API ([RPC](rpc.md), [accounts](accounts.md), [management API](management-api.md)).
The server-side binary stays `taur` (`serve`, `import`); keeping them separate means a
client machine never needs the server's SQLite/git dependencies or data dir.

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
taurctl key list | add FILE|- | remove ID
taurctl pkg vote|unvote|notify|unnotify|flag|unflag|comments|comment|comaintainers …   # see community.md
taurctl comment edit|delete|pin|unpin …
taurctl mcp                                     # MCP over stdio, see mcp.md
taurctl request file|list|show|accept|reject …
```
`unregister`, `org delete` and `pkg disown` ask for confirmation; without a terminal
they refuse unless `-y` is given.

## Exit status
0 success, 1 error (server refusal, network, input), 2 usage error, 3 not logged in or
invalid credentials/session.

## Man pages
Generated from the clap definitions into `man/` (`taurctl.1`, `taurctl-org-add-member.1`,
…). Regenerate with `TAUR_UPDATE_MAN=1 cargo test -p taur-cli man_pages`; a test fails
when they are out of date.

## Example
```sh
taurctl config set-server https://taur.weirdbrains.com
taurctl register alice --email alice@example.com
taurctl login alice
taurctl org create weirdbrains
taurctl org add-member weirdbrains bob --role admin
taurctl pkg transfer hello-taur --org weirdbrains
```
