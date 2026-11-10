# Organization and package management API

Authenticated JSON endpoints (`Authorization: Bearer <token>`, see
[accounts](accounts.md)). Permission rules are enforced in `taur-core` (see
[roles](roles.md)).

| Method | Path | Body | Who | Result |
|---|---|---|---|---|
| GET | `/api/v1/orgs` | | any user | `[{"name","role"}]` (your orgs) |
| POST | `/api/v1/orgs` | `{"name"}` | any user (becomes Owner) | 201 org detail |
| GET | `/api/v1/orgs/{org}` | | members | `{"id","name","members":[{"name","role"}]}` |
| DELETE | `/api/v1/orgs/{org}` | | Owner | 204 |
| PUT | `/api/v1/orgs/{org}/members/{user}` | `{"role":"owner\|admin\|contributor"}` | Owner, Admin (not for Owner role/Owners) | 204 (add or change role) |
| DELETE | `/api/v1/orgs/{org}/members/{user}` | | Owner, Admin (not Owners) | 204 |
| GET | `/api/v1/packages` | | any user | `[{"pkgbase","maintainer","org"}]` you can manage |
| POST | `/api/v1/packages/upload` | multipart fields `archive` (`.tar.gz`, `.tar`, or `.zip`) and optional `organization` | verified user. A new package in an organization requires membership and ManagePackages. An update uses the same permission as a push and does not change owner. | 201 create or 200 update: `{"pkgbase","name","version","created"}` |
| GET | `/api/v1/packages/by-name/{name}` | | anyone | package page JSON, including archived packages; `CanManage` when the caller could push |
| POST | `/api/v1/packages/{pkgbase}/archive` | | verified maintainer, co-maintainer, or org member who can manage it | 204; hidden from search, RPC info, and dumps; git history kept |
| DELETE | `/api/v1/packages/{pkgbase}/archive` | | same | 204; restored to search and dumps |
| POST | `/api/v1/packages/{pkgbase}/owner` | `{"org"}` or `{"user"}` | anyone who can manage it (+ ManagePackages in target org) | 204 |
| POST | `/api/v1/packages/{pkgbase}/disown` | | anyone who can manage it | 204 (orphaned; org link kept) |
| POST | `/api/v1/packages/{pkgbase}/adopt` | | anyone if orphaned and not org-owned; org members otherwise | 204 |

Transferring to a user clears the org link and makes that user maintainer. Deleting an
org keeps its packages' maintainers and clears the org link. The last Owner can't be
removed or demoted.

`POST /api/v1/packages/upload` is multipart, not JSON, and still requires
`X-CSRF-Token` equal to the bearer. An absent or empty `organization` field publishes
a new package under the uploader. A name publishes it under that organization in the
same insert; the caller must belong to the organization and be allowed to manage its
packages. Site admin is not a substitute for membership. On an update the field must
be empty or name the package's current organization. A different name is 409 and the
owner is not changed. Selecting your account while updating an organization package
is allowed and also leaves the owner unchanged.

Errors: 400 bad body, 401 no/invalid session, 403 not allowed, 404 unknown org/user/package,
409 conflict (name taken, not orphaned, last Owner, upload owner does not match).

Site-admin and moderator endpoints use the same bearer token and `X-CSRF-Token`.
Moderators may `GET /api/v1/admin/users` (no email) and reject an ordinary account or
package. They cannot grant roles, see email addresses, accept or reverse audit rows,
or manage registries and mirrors.

| Method | Path | Who |
|---|---|---|
| GET | `/api/v1/admin/users?q=` | moderator or site admin |
| POST | `/api/v1/admin/users/{name}/verify` | site admin |
| POST | `/api/v1/admin/users/{name}/resend` | site admin |
| POST | `/api/v1/admin/users/{name}/password-reset` | site admin; the token is only in the mail |
| DELETE | `/api/v1/admin/users/{name}/sessions` | site admin |
| PUT | `/api/v1/admin/users/{name}/admin` | site admin; body `{"on":true\|false}` |
| PUT | `/api/v1/admin/moderators/{name}` | site admin; body `{"on":true\|false}` |
| GET | `/api/v1/sessions` | the signed-in account |
| DELETE | `/api/v1/sessions/{id}` | that account's own session |
| GET | `/api/v1/registries/{name}` | site admin; keys, identities, expectations |
| GET, POST | `/api/v1/mirrors` | site admin |
| GET, DELETE | `/api/v1/mirrors/{name}` | site admin |
| POST | `/api/v1/mirrors/{name}/select\|unselect\|sync\|drop` | site admin |

`taur registry keygen` and `taur registry sign` stay on the machine that holds the
secret seed. `taur gc`, `taur import`, and `taur serve` stay on the server machine.
