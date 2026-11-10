# Importing packages

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
