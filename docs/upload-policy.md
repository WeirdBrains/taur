# Package registries

A package registry is not a user account. Packages that were already in TAUR, and
uploads that do not name another registry, belong to `local`. That registry is
**exempt**: it keeps the previous upload rules. An administrator creates further
registries with `taur registry create`. Those are **enforcing**.

```sh
taur registry create vendor
taur registry keygen /var/lib/taur/vendor-key
taur registry add-key vendor lab /var/lib/taur/vendor-key/public.hex
taur registry bind-identity vendor alice alice@example.com
taur registry sign --secret-key /var/lib/taur/vendor-key/seed.hex \
  --registry vendor --repository extra --pkgbase demo --version 1.0.0-1 \
  --srcinfo .SRCINFO --pkgbuild PKGBUILD > TAUR-RECIPE.sig
taur registry expect vendor demo --repository extra --version 1.0.0-1 \
  --signature TAUR-RECIPE.sig --key-fingerprint <fingerprint> --nick alice
taur import ./demo --maintainer alice --registry vendor
```

The same records can be written over HTTP by a site admin:
`POST /api/v1/registries`, `.../keys`, `.../identities`, and `.../expectations`.
A moderator cannot. The private seed from `keygen` is mode `0600` and is never
uploaded. `sign` prints a signature and does not send the seed to the server.

## What a signature covers

The signature is Ed25519 over a message that contains the registry name, the
repository, the package base, the version, and the exact `.SRCINFO` and
`PKGBUILD` bytes. A signature for one package, repository, or registry does not
verify for another. The server stores only the public key.

Put these files in the uploaded directory or git tree:

- `TAUR-REGISTRY` — registry name (omit for `local`)
- `TAUR-REPOSITORY` — repository name inside that registry
- `TAUR-RECIPE.sig` — hex signature

## Known packages

A name listed with `expect` is known. On an enforcing registry:

- The signature must verify under a key of that registry, and the repository in
  the signed message must be the recorded one.
- The recorded version and signature may be uploaded again.
- A different valid signature is accepted only when the version is newer and the
  authenticated account is the one bound to that nick and email. Typing the same
  strings somewhere in the recipe is not that binding.
- Anything else is rejected before the package is indexed. `import` does not
  commit. A git push is refused in `pre-receive`, so the branch does not move.
  Git may still have unreachable objects until `git gc`; they are not a package.
- An older signed recipe is rejected, so a previous signature cannot replace a
  newer one.
- A package cannot be moved from one registry to another by naming a different
  registry in the upload.

A name that is not listed keeps the existing rules, including on an enforcing
registry. `local` never applies this check.

## Registration

If an enforcing registry has an operator identity for a nick, registration with
that nick and a different email is refused before any account row is written.
No identity for the nick is not a conflict. Matching nick and email registers
the account and binds it to that identity. Accounts that already exist are left
as they are.

Reversing the audit row does not create the account and does not turn the check
off. A later registration still has to match the identity. Reversing an upload
rejection does not store the recipe and does not skip the next signature check.

## What this is not

Every key, identity, and expectation stored here has provenance `operator`.
That means an administrator of this TAUR instance attested to it. It does not
mean Arch Linux or the AUR checked the package or the person.

The public AUR info object (for example `yay` on 2026-11-06) has `Maintainer`
and `Version` and no email or recipe signature. A pacman repository signature
covers a built archive, not the recipe TAUR stores. This server does not call
those services and does not label operator records as official verification.
