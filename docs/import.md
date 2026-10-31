# Importing packages

`taur import DIR [--maintainer USER]`

Reads `DIR/.SRCINFO` (generate with `makepkg --printsrcinfo > .SRCINFO`), commits the
directory into `repos/<pkgbase>.git` (branch `master`) and updates package metadata.
Re-importing updates the existing package base (new commit, `LastModified` bumped).
Without `--maintainer` the package base is orphaned.

`.SRCINFO` handling: package sections override pkgbase values per key (an empty value
clears it); architecture-specific keys (`depends_x86_64`) are merged into the plain list;
version is `[epoch:]pkgver-pkgrel`.

Failure modes: missing `.SRCINFO`; parse errors (line number reported); invalid names;
unknown maintainer; a pkgname already owned by another package base (conflict).
