# Importing packages

A verified user can also upload that directory as a `.tar.gz`, `.tar`, or `.zip` at `/upload`
or with `taurctl pkg upload ARCHIVE [--org ORG]`
(`POST /api/v1/packages/upload`, multipart field `archive`, optional field
`organization`). The same `.SRCINFO` and `PKGBUILD` checks run. The PKGBUILD is not
executed. With `organization` or `--org` empty or absent, a new package base is adopted by the
uploader. A name adopts it to that organization in the same insert. The name is the
organization's exact name (3-16 characters of `A-Z` `a-z` `0-9` `.` `_` `-`, starting
and ending alphanumeric), and the uploader must be an Owner, Admin, or Contributor of
that organization. An existing package base is updated only when
that user could push it, and its owner is left unchanged. A non-empty `organization`
that is not the current owner is rejected. The archive is at most 8 MiB. See
[Publish a package](guides/publish.md).

`taur import DIR [--maintainer USER]`

Reads `DIR/.SRCINFO` (generate with `makepkg --printsrcinfo > .SRCINFO`), commits the
directory into `repos/<pkgbase>.git` (branch `master`) and updates package metadata.
Re-importing updates the existing package base (new commit, `LastModified` bumped).
Without `--maintainer` the package base is orphaned.

`.SRCINFO` handling: package sections override pkgbase values per key (an empty value
clears it); architecture-specific keys (`depends_x86_64`) are merged into the plain list;
version is `[epoch:]pkgver-pkgrel`.

An architecture-specific field is accepted only when the pkgbase section has already
listed that architecture with `arch`. `makepkg --printsrcinfo` writes `arch` first.
`arch = any` does not allow `depends_x86_64`, and `depends_any` is never allowed. A
`pkgname` section's own `arch` does not change this. yay and paru use the same rule
and refuse to install a recipe that breaks it.

Failure modes: missing `.SRCINFO`; parse errors (line number reported); invalid names;
an architecture-specific field for an architecture the pkgbase `arch` lines have not
already listed; unknown maintainer; a pkgname already owned by another package base
(conflict).
