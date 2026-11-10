---
title: Publish a package
summary: Push a package base with git, from your key to the first commit.
audience: user
order: 0
---

This is how a package you maintain shows up in search.

## Upload from the site

Open [Upload](/upload) after your email is verified. Choose a `.tar.gz`, `.tar`, or `.zip` of the package directory. `PKGBUILD` and `.SRCINFO` sit at the top of that directory, or inside one folder. The archive is at most 8 MiB, with at most 64 files, and each file is at most 256 KiB. Nested directories, symlinks, and paths that leave the directory are refused.

The Owner control lists your account and every organization where you can manage packages (Owner, Admin, or Contributor). The default is your account. Open the list and type in the filter at the top to narrow the options; if nothing matches, the list says so and the current owner stays selected. A new package is created under that owner immediately. An update keeps the current owner. If you name an organization that is not the package's organization, the upload is refused and nothing is moved.

The server checks `.SRCINFO`, the literal `PKGBUILD` assignments, architectures, maintainership, and any registry rule. It does not run the PKGBUILD or `makepkg`. A new package base is adopted by the owner selected above. An update is accepted only when you could also `git push` that base: you maintain it, you co-maintain it, or your organization can manage it. A rejected package stays rejected.

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

To publish a new package under an organization from the site, pick that organization on Upload. From the CLI, use `taurctl pkg upload ./webup.tar.gz --org ORG`, where `ORG` is the organization's exact name. With git, create the organization, push the package, then transfer the package base from the package page. Members who can manage packages in that organization can push updates.

## Upload with taurctl

After you are signed in with a verified account:

```sh
taurctl pkg upload ./webup.tar.gz
taurctl pkg upload ./webup.tar.gz --org brains
```

`--org` uses the exact organization name (3-16 characters of `A-Z` `a-z` `0-9` `.` `_` `-`, starting and ending alphanumeric). You must be an Owner, Admin, or Contributor. Omit the flag to publish a new package under your account. An update without `--org` still succeeds and does not move the package. An update with `--org` is accepted only when that name is already the package's organization.

## What the server checks

`.SRCINFO` must match the `PKGBUILD` fields helpers understand. An architecture-specific field has to agree with `arch`. A rejected package base cannot be pushed until a site admin reverses that rejection. An enforcing registry only accepts a recipe the site owner expected and signed.
