# Configuration and data directory

`taur [--home DIR] <command>`

Data directory resolution: `--home`, then `TAUR_HOME`, then `~/.local/share/taur`.
Contents: `taur.db` (SQLite, bundled) and `repos/<pkgbase>.git` (bare git repos).
Tests and manual runs must use a temp dir.

`taur serve --bind ADDR [--ssh-bind ADDR]` (default `127.0.0.1:8080`, SSH off) runs the HTTP server and optionally the git SSH server. The public deployment will be `https://taur.weirdbrains.com`.

Man pages for `taur` and `taurctl` are in `man/`.

Failure modes: unwritable home → startup error; bind in use → startup error.
