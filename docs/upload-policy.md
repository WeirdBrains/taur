# Official package registries

This is not implemented. The notes below are the facts that block a safe check, not a
design that TAUR enforces.

## What the server has

A push is a git repository of a recipe (`PKGBUILD` and `.SRCINFO`), not a pacman
archive. Permission is the maintainer, a co-maintainer, an organization member who can
manage the base, or a site admin. There is no table of official repositories, no stored
package signature, and no copy of an upstream maintainer email. Packages that are not
already in this server are created by the first verified pusher. That behavior is
unchanged.

## What the public AUR RPC provides

`https://aur.archlinux.org/rpc/v5/info/yay` (retrieved 2026-11-06) returns `Maintainer`
(`jguer`), `Version`, and the usual package fields. The object has no email and no
signature field. A nickname from that response is not proof that the TAUR account with
the same name is that person.

## What an official Arch repository provides

Pacman repository metadata can include `%VERSION%`, `%PGPSIG%` (a signature over the
built `.pkg.tar` archive), and `%PACKAGER%` (often a display name and an email). Those
bytes are not a signature of the recipe TAUR received. Comparing them to a PKGBUILD, or
treating a matching checksum of a different file as approval, would be a false check.
`%PACKAGER%` is not bound to the authenticated TAUR user.

## The decision that is still open

A check for packages that appear in a newly configured official registry would need all
of the following, and none of them are available together:

- a definition of which registries an operator added, separate from user accounts
- a public nick and email for the upstream maintainer that can be tied to the signed-in TAUR account
- a signature over the same bytes TAUR is being asked to store

Until that exists, TAUR does not reject an upload for "being in AUR" or "being in core",
and it does not accept one because a checksum happened to match. Rejected uploads of
that kind are not queued, because the check itself is not run.
