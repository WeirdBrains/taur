# Votes, flags, comments, co-maintainers and requests

AUR community features. All writes need a verified account (Bearer token).

## Votes and popularity
`POST` / `DELETE /api/v1/packages/{pkgbase}/vote` (idempotent); `GET …/vote` →
`{"voted": bool}` for the signed-in user. RPC `NumVotes` is the vote
count; `Popularity` follows aurweb: each vote counts 0.98^(days since it was cast).
Updated on every vote and refreshed hourly by `taur serve`.

## Out-of-date flags
- `POST /api/v1/packages/{pkgbase}/flag` `{"comment"}` (comment required; 409 if
  already flagged). Maintainers and co-maintainers get an email.
- `DELETE …/flag`: the flagger or anyone who can manage the package.
- `GET …/flag` → `{"out_of_date","flagger","comment"}`.
- Pushing/importing a new version clears the flag. RPC `OutOfDate` is the flag time.

## Comments
- `GET /api/v1/packages/{pkgbase}/comments` (public, newest first);
  `POST …/comments` `{"content"}` (1–10000 chars).
- `PATCH /api/v1/comments/{id}` `{"content"}`: author only.
- `DELETE /api/v1/comments/{id}`: author, package managers or site admins (soft delete).
- `POST|DELETE /api/v1/comments/{id}/pin`: package managers or site admins pin/unpin;
  pinned comments are listed first and carry `"pinned": true`.

## Notifications
`POST|DELETE /api/v1/packages/{pkgbase}/notify` subscribes/unsubscribes (verified
accounts); `GET …/notify` returns `{"subscribed": bool}`. Subscribers are emailed when
the package is pushed with a new commit and when someone comments (never about their own
action). Git hooks get the server's mail settings through the environment, so SMTP works
for push notifications too.

## Co-maintainers
`PUT /api/v1/packages/{pkgbase}/comaintainers` `{"users": [...]}` replaces the list (the
maintainer is skipped). Allowed: maintainer, site admins, org Owners/Admins of the owning
org. Co-maintainers may push and manage the package. RPC `CoMaintainers` lists them.

## Requests
- `POST /api/v1/packages/{pkgbase}/requests` `{"kind": "orphan"|"deletion"|"merge",
  "merge_into"?, "comment"}`; maintainers are emailed.
- `GET /api/v1/requests[?all=true]` (signed in; pending only by default),
  `GET /api/v1/requests/{id}`.
- `POST /api/v1/requests/{id}/accept` `{"comment"}`: site admins; carries it out:
  orphan clears the maintainer, deletion removes the package base (repo moved to
  `<home>/repos/.deleted/`), merge moves votes and comments to the target and removes
  the source.
- `POST /api/v1/requests/{id}/reject`: site admins reject; the requester can withdraw
  (status `closed`).

Like aurweb, an orphan request for a package flagged out-of-date for 180 days or more
is accepted automatically when filed: the maintainer is cleared and the request closes
as `accepted` with an explanatory closure comment.

## Site admins
Grant on the server with `taur admin NAME` (revoke with `--revoke`). Site admins can
accept/reject requests and manage any package (like AUR Package Maintainers).

## CLI
```
taurctl pkg vote|unvote PKGBASE
taurctl pkg flag PKGBASE --comment TEXT | pkg unflag PKGBASE
taurctl pkg comments PKGBASE | pkg comment PKGBASE TEXT
taurctl comment edit ID TEXT | comment delete ID
taurctl pkg comaintainers PKGBASE [USER...]
taurctl request file PKGBASE --kind orphan|deletion|merge [--into TARGET] --comment TEXT
taurctl request list [--all] | show ID | accept ID [--comment] | reject ID [--comment]
```
