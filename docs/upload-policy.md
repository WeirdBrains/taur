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

An administrator can accept one of those rejections so the same attempt can be
submitted again. The commands are:

```sh
taurctl moderation audit
taurctl moderation accept ID "this exact attempt is allowed"
taurctl moderation revoke ACCEPTANCE_ID "withdrawn"
```

The same decision is `POST /api/v1/audit/{id}/accept` and `POST /api/v1/audit/{id}/revoke`
with `{"reason":"..."}`, the Accept button on the audit page, gRPC `AcceptRejection`
and `RevokeAcceptance`, and the MCP tools `accept_rejection` and `revoke_acceptance`.
A moderator cannot accept, revoke, or grant roles.

Acceptance does not create an account, store the recipe, or publish the package.
`import` and a completed git push still have to send the files again. The gate
looks up the latest `reject`, `accept`, or `revoke` for that scope and skips the
business signature or identity rule only when the latest one is `accept`. There
is no clock expiry. A newer rejection of the same scope, or a revocation, wins.
Accepting the older rejection is then refused until that newer rejection is the
one being accepted. Repeating an acceptance that is still in force returns the
same audit id.

An upload scope is the registry, repository, package base, version, the SHA-256
of `.SRCINFO`, the SHA-256 of `PKGBUILD`, and the applicant's account id. A
registration scope is the registry whose operator identity conflicted, the nick,
and the SHA-256 of the normalized email. The email is not stored. A public nick
by itself is not enough, and the decision does not apply to another user,
package, version, repository, registry, or changed file. Accepting one registry's
registration conflict does not waive a different registry.

The overridden check is not run again for that scope. Sign-in, email
verification on git push, suspension, `.SRCINFO` parsing, path checks, and an
unrelated rejection still apply. Acceptance does not rewrite the operator
expectation, so a later valid recipe is still judged by the recorded signature.
A git `pre-receive` refusal does not move the branch. Git may keep unreachable
objects until `git gc`; those objects are not a package. Reversing the audit
row records a review and does not permit the retry.

## What this is not

Every key, identity, and expectation stored here has provenance `operator`.
That means an administrator of this TAUR instance attested to it. It does not
mean Arch Linux or the AUR checked the package or the person.

The public AUR info object (for example `yay` on 2026-11-06) has `Maintainer`
and `Version` and no email or recipe signature. A pacman repository signature
covers a built archive, not the recipe TAUR stores. This server does not call
those services and does not label operator records as official verification.
