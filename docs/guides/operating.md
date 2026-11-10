---
title: Operating the server
summary: Install, TLS, mail, backups, trust records, and upgrades.
audience: owner
order: 100
---

Site owner is the top role. The first account on a server with no admin and no owner becomes owner and admin, and is verified, unless `TAUR_ADMINS` or `TAUR_OWNERS` is set. An existing server that already has an admin does not gain an owner until you run:

```sh
sudo -u taur taur owner YOUR-NAME --home /var/lib/taur
```

`TAUR_OWNERS` grants the same flag at startup, and only if the account already exists. Removing the variable does not revoke. `taur owner NAME --revoke` leaves the admin flag and refuses to remove the last owner. Admins cannot demote an owner.

## Install and upgrades

Native packages and the upgrade steps are in the install guide shipped with the repository (`docs/install.md`). After a package upgrade, start the process again. Do not delete `/var/lib/taur`; that directory is the database and the git repositories.

## TLS

`TAUR_TLS=off` is plaintext, for a local process or a proxy that terminates TLS. `TAUR_TLS=acme` requests a certificate. It needs a domain, a contact, and an explicit terms acceptance. The HTTP-01 listener is separate from the HTTPS port. Staging certificates are not publicly trusted.

## Mail

Without `TAUR_SMTP_URL`, messages are files in the data directory `outbox`. With SMTP, percent-encode `/ ? # % : @` in the username and password. The server tries every address it resolves, so an IPv6 miss falls through to IPv4. A failed verification resend is `503` with `mail server unreachable`.

## Backups

Copy the data directory while you can afford a short consistent snapshot: `taur.db` plus `repos/`. WAL mode allows a reader during normal operation. A copy taken in the middle of a push can be finished with the repository's own git files. Keep the outbox if you still need undelivered mail. Do not publish `taur.db`; it holds password hashes and session hashes.

## Registries and trust

**Registries** is owner-only. The built-in `local` registry stays on the previous upload rules. A registry you create is enforcing. Add an Ed25519 public key, bind a nick to an email, and record the signed recipe you expect.

Generate the key and sign the recipe on a machine that keeps the secret seed:

```sh
taur registry keygen ./keys
taur registry sign --secret-key ./keys/seed --registry vendor --repository extra --pkgbase hello --version 1.0.0-1 --srcinfo .SRCINFO --pkgbuild PKGBUILD
```

Paste the public key and the signature into the registry page. The server does not want the seed. These records are operator attestations, not an Arch or AUR endorsement.

## Releases

Version tags and the release workflow are described in `docs/releases.md`. Publishing a release is a git and GitHub action on the operator's checkout, not a button in this UI.
