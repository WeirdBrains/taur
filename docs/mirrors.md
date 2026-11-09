# Recipe mirrors

A mirror copies **recipes** from a Git source that looks like the AUR or like another TAUR server: one repository per package base, branch `master`, with `.SRCINFO` and `PKGBUILD` at the top of the tree.

```sh
taur mirror add aur --url https://aur.archlinux.org
taur mirror select aur yay
taur mirror sync aur
taur mirror status aur
```

Nothing is fetched by `add` or `select`. `sync` copies only the names you selected, at most 200 of them. This server does not mirror the whole AUR unless an administrator names every package and runs `sync`. A fresh install has no mirror configured.

The copy is stored in a registry of the same name with enforcement `mirror` and provenance `mirror`. That is not the exempt `local` registry and it is not an enforcing registry. Upload, import, and git push into it are refused. The refusal is not an acceptance and it does not record an operator signature.

## What can be a source

The base URL is `{url}/{pkgbase}.git`.

- Public sources are `https` only.
- `http` and `git://` are accepted only for `localhost`, `127.0.0.1`, or `::1`, so a test fixture can run on this machine.
- The URL must not contain a username, a password, a query, or a fragment.
- These are refused: `file://`, `ssh://`, `ext::`, a URL that already ends in `.git`, and a pacman database or built package (`.db`, `.db.tar.gz`, `.db.tar.zst`, `.files`, `.pkg.tar.zst`, and the same for `.xz`).

A pacman `Server=` repository is a set of built archives. It is not a recipe, and TAUR will not import it as one.

Git is run without the operator's git config, without a credential helper, without redirect following, and without hooks. The fetch is depth 1, one minute, and the tree may be at most 1 MiB and 64 regular files. Symlinks, subdirectories, and submodules are refused. `PKGBUILD` is not executed.

Each copied tree includes `TAUR-MIRROR` with the source URL and the upstream commit. `taur mirror status` prints that revision. The note says the copy is not an operator signature and not an Arch or AUR verification.

## Updates, failures, and deletions

- The same upstream commit is `unchanged` and does not add another commit.
- A refused fetch keeps the last good copy. The status is `failed`, or `upstream-missing` when this package had a copy and the repository is gone.
- A package that already lives in another registry, including `local`, is `blocked` and is not overwritten.
- A rejected (suspended) package is `blocked`. Sync does not change its files and does not clear the rejection. Reversing the rejection is still a separate admin action. Accepting an upload or registration does not apply to a mirror.
- `unselect` stops further updates and keeps the last copy.
- `drop` deletes a package only when it belongs to this mirror.
- `remove` deletes the source only after those packages are gone.
- A copy whose last success is older than 7 days is marked stale. It is not deleted.

`sync` still prints a line for each selected package when one of them fails. The command exits non-zero if any selected package was not updated. The packages that did update stay updated.
