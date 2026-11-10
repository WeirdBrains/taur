---
title: Publish a package
summary: Push a package base with git, from your key to the first commit.
audience: user
order: 0
---

This is how a package you maintain shows up in search.

## Upload from the site

Open [Upload](/upload) after your email is verified. Choose a `.tar.gz`, `.tar`, or `.zip` of the package directory. `PKGBUILD` and `.SRCINFO` sit at the top of that directory, or inside one folder. The archive is at most 8 MiB, with at most 64 files, and each file is at most 256 KiB. Nested directories, symlinks, and paths that leave the directory are refused.

The server checks `.SRCINFO`, the literal `PKGBUILD` assignments, architectures, maintainership, and any registry rule. It does not run the PKGBUILD or `makepkg`. A new package base is adopted to you. An update is accepted only when you could also `git push` that base: you maintain it, you co-maintain it, or your organization can manage it. A rejected package stays rejected.

Git and SSH pushes are unchanged.

## Archive

On a package you maintain, **Archive** hides it from search, name completion, RPC info, and the metadata dumps. The package page stays, labeled Archived, and the git repository is kept. **Unarchive** puts it back. Archiving asks for confirmation. This is not rejection and not disowning: you remain the maintainer.

## Out of date

**Mark out of date** on your package, or **Flag out-of-date** on someone else's, records that a newer upstream exists. **Clear out-of-date flag** removes it. A new upload of a different version also clears it.

## What you need

- A verified account. Open the link in the verification mail, or use **Resend verification** on your account page. A site admin can also mark you verified.
- An SSH public key on your account. Generate one with `ssh-keygen -t ed25519` and paste the `.pub` line on your account page.
- A git tree whose top level has both `PKGBUILD` and `.SRCINFO`.

## Create the repository

```sh
mkdir hello && cd hello
# write PKGBUILD, then:
makepkg --printsrcinfo > .SRCINFO
git init
git add PKGBUILD .SRCINFO
git commit -m "Initial import"
```

The remote name is the package base, not the git host's user:

```sh
git remote add origin ssh://taur@YOUR-SERVER:2222/hello.git
git push -u origin master
```

Use the SSH clone line on the server's package page if the port is not 2222. The branch that is published is `master`.

## If the name already exists

An orphan can be adopted from its package page. If you are not a site admin, that files an adoption request. A site admin accepts it.

To publish under an organization, create the organization, then transfer the package base to it from the package page. Members with a role in that organization can push updates.

## What the server checks

`.SRCINFO` must match the `PKGBUILD` fields helpers understand. An architecture-specific field has to agree with `arch`. A rejected package base cannot be pushed until a site admin reverses that rejection. An enforcing registry only accepts a recipe the site owner expected and signed.
