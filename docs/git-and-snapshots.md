# Git clone, push and snapshots

Each package base has a bare repo at `<home>/repos/<pkgbase>.git` (branch `master`).

## Clone (anonymous)
- Smart HTTP: `git clone https://packages.taur.weirdbrains.com/<pkgbase>.git` (via `git http-backend`).
  `git clone https://packages.taur.weirdbrains.com/<pkgbase>` is the same repository. paru requests that
  shorter path; the on-disk repo is still `<pkgbase>.git`.
- SSH: `git clone ssh://taur@packages.taur.weirdbrains.com:2222/<pkgbase>.git` (needs a registered key).

## Push
- **HTTP:** HTTP Basic auth with your account name and a session token from
  `taurctl login` / `POST /api/v1/login` as password, e.g.
  `git push https://alice:<token>@packages.taur.weirdbrains.com/foo.git master`
  (or use a git credential helper). A Bearer token header also works.
- **SSH:** register a key ([SSH keys](ssh-keys.md)), then
  `git push ssh://taur@packages.taur.weirdbrains.com:2222/foo.git master`. The SSH user name
  is ignored; the key identifies the account. Start the SSH server with
  `taur serve --ssh-bind 0.0.0.0:2222` (or `TAUR_SSH_BIND`; see [configuration](configuration.md)); its host key is created at
  `<home>/ssh_host_ed25519_key` (0600) on first start.

Pushing to a name that doesn't exist yet creates the package base; the pusher becomes
maintainer and submitter. Existing package bases accept pushes from the maintainer,
co-maintainers and members of the owning organization (any role).

## Validation (like aurweb's git-update hook)
Every new commit is checked; the push is rejected (message shown as `remote: error: …`) if:
- the ref is not `refs/heads/master`, the push deletes master, or is not fast-forward;
- a commit lacks `PKGBUILD` or `.SRCINFO`, or `.SRCINFO` fails to parse;
- `.SRCINFO`'s pkgbase differs from the repository name;
- an architecture-specific field (such as `depends_x86_64` or `depends_any`) names an
  architecture the pkgbase `arch` lines have not already listed. `arch = any` with
  `depends_x86_64` is the case yay and paru both refuse;
- a simple `pkgbase=`, `pkgver=`, `pkgrel=`, `epoch=` or `pkgname=` line in the PKGBUILD disagrees with `.SRCINFO`. Lines that contain `$` or backticks are not compared. This does not execute the PKGBUILD and does not prove the two files match;
- the tree contains subdirectories or submodules, or a file over 256 KiB;
- a pkgname already belongs to another package base;
- the pusher lacks permission.

After an accepted push the metadata (versions, dependencies, …) is refreshed from the new
`.SRCINFO`. The hooks run `taur hook pre-receive|post-receive`; they refuse pushes that
don't come through the TAUR server.

Git places the incoming pack in a quarantine directory and deletes it when
`pre-receive` refuses the push, so that refusal does not leave the new objects
in the package repository. The branch does not move. A name that has never been
accepted does not keep the empty repository that was created for the attempt.
`import` checks the recipe before it writes a commit, so a refused import does
not create a repository either.

`taur gc` runs `git gc --prune` on each package repository. Unreachable objects
newer than `--grace-secs` (default 14 days) are kept, which covers a push that
has copied objects in but has not updated the branch yet. A repository with a
quarantine directory (`objects/incoming*`), `gc.pid`, or a ref lock is skipped.
`taur serve` runs the same maintenance once a day, after the first day, with
that 14-day grace. This does not delete a package or a branch. It is a local
data-directory command, not a `taurctl` request.

Set `TAUR_PKGBUILD_ISOLATION=bwrap` to also run `makepkg --printsrcinfo` inside bubblewrap
(read-only `/usr`, no home directory, no network, 20 second limit) and compare pkgbase and
version with the committed `.SRCINFO`. Any other value is refused. If `bwrap` is missing,
the push is refused rather than running `makepkg` on the host. Unset, this check is skipped.
`cargo test -p taur-server --test fixture_isolation` is the regression for building and
installing `tests/fixtures/hello-taur` only inside that sandbox. See
[security notes](security.md) for what that sandbox does not prove.

Deleted package bases keep their repository under `<home>/repos/.deleted/`. See
[community](community.md) for who can restore one.

## Snapshots and plain files
`GET /cgit/aur.git/snapshot/<pkgbase>.tar.gz` (the RPC `URLPath`): `git archive` of HEAD.
Optional `id` selects a recorded revision instead of HEAD, for example
`/cgit/aur.git/snapshot/<pkgbase>.tar.gz?id=<commit>`.

Each successful `GET` of that snapshot route increments `DownloadCount` on the
package base, whether the tree is current or historic. The counter starts at 0
for existing packages; earlier downloads are not backfilled. It is a count of
successful snapshot/recipe-archive responses, not of installed binaries or
unique users, and it does not store client addresses. `HEAD` requests, failed
responses, `/cgit/aur.git/plain/` file inspection, RPC info/search, git clones,
and the site-wide metadata dumps are not counted.

`GET /cgit/aur.git/plain/<path>?h=<pkgbase>` returns one file from that package base
(`PKGBUILD`, `.SRCINFO`, …). `h` is the package base name — TAUR keeps a separate
repository per base, rather than aurweb's single repo with git namespaces. Optional
`id` selects a revision (default `HEAD`); `..` path segments are rejected.
