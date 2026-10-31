# Git clone, push and snapshots

Each package base has a bare repo at `<home>/repos/<pkgbase>.git` (branch `master`).

## Clone (anonymous)
- Smart HTTP: `git clone https://taur.weirdbrains.com/<pkgbase>.git` (via `git http-backend`).
- SSH: `git clone ssh://taur@taur.weirdbrains.com:2222/<pkgbase>.git` (needs a registered key).

## Push
- **HTTP:** HTTP Basic auth with your account name and a session token from
  `taurctl login` / `POST /api/v1/login` as password, e.g.
  `git push https://alice:<token>@taur.weirdbrains.com/foo.git master`
  (or use a git credential helper). A Bearer token header also works.
- **SSH:** register a key ([SSH keys](ssh-keys.md)), then
  `git push ssh://taur@taur.weirdbrains.com:2222/foo.git master`. The SSH user name
  is ignored; the key identifies the account. Start the SSH server with
  `taur serve --ssh-bind 0.0.0.0:2222`; its host key is created at
  `<home>/ssh_host_ed25519_key` (0600) on first start.

Pushing to a name that doesn't exist yet creates the package base; the pusher becomes
maintainer and submitter. Existing package bases accept pushes from the maintainer,
co-maintainers and members of the owning organization (any role).

## Validation (like aurweb's git-update hook)
Every new commit is checked; the push is rejected (message shown as `remote: error: …`) if:
- the ref is not `refs/heads/master`, the push deletes master, or is not fast-forward;
- a commit lacks `PKGBUILD` or `.SRCINFO`, or `.SRCINFO` fails to parse;
- `.SRCINFO`'s pkgbase differs from the repository name;
- the tree contains subdirectories or submodules, or a file over 256 KiB;
- a pkgname already belongs to another package base;
- the pusher lacks permission.

After an accepted push the metadata (versions, dependencies, …) is refreshed from the new
`.SRCINFO`. The hooks run `taur hook pre-receive|post-receive`; they refuse pushes that
don't come through the TAUR server.

## Snapshots
`GET /cgit/aur.git/snapshot/<pkgbase>.tar.gz` (the RPC `URLPath`): `git archive` of HEAD.
