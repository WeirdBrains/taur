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
| POST | `/api/v1/packages/{pkgbase}/owner` | `{"org"}` or `{"user"}` | anyone who can manage it (+ ManagePackages in target org) | 204 |
| POST | `/api/v1/packages/{pkgbase}/disown` | | anyone who can manage it | 204 (orphaned; org link kept) |
| POST | `/api/v1/packages/{pkgbase}/adopt` | | anyone if orphaned and not org-owned; org members otherwise | 204 |

Transferring to a user clears the org link and makes that user maintainer. Deleting an
org keeps its packages' maintainers and clears the org link. The last Owner can't be
removed or demoted.

Errors: 400 bad body, 401 no/invalid session, 403 not allowed, 404 unknown org/user/package,
409 conflict (name taken, not orphaned, last Owner).
