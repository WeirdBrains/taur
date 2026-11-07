# Moderators and the audit

A **moderator** can reject a package base or an ordinary account. That is the only extra
ability. A moderator cannot grant roles, accept requests, manage other people's packages,
or reverse a rejection. Site admins (`taur admin`, or `PUT /api/v1/admin/moderators/{name}`
with `{"on":true}`) grant and remove the role. Moderators cannot suspend a site admin or
another moderator.

Rejection does not delete the row or the git repository. The package disappears from
search, info, suggest, and metadata dumps, and git clone and push of that base are
refused with `package is rejected`. A suspended account cannot sign in; existing
sessions are deleted. The account row stays, so this is not applied to accounts that
already existed before this feature as a bulk action. Nothing here changes accounts
that an operator does not reject.

```sh
taurctl moderation set-moderator alice
taurctl moderation reject-package hello-taur "not the package it claims to be"
taurctl moderation reject-user bob "account is not the person it claims to be"
```

The same calls exist on `POST /api/v1/moderation/packages/{name}/reject` and
`POST /api/v1/moderation/users/{name}/reject`, on gRPC `Moderation`, and as the MCP
tools `reject_package` and `reject_user`. The web UI shows the form on a package or
user page for a moderator or site admin.

## Audit

`GET /api/v1/audit` (web page `/audit`, `taurctl moderation audit`, gRPC `ListAudit`,
MCP `list_audit`) is site-admin only. Each row is a decision: target kind and name,
reason, time, actor name, and source (`moderator` or `admin`). It does not store
package contents, passwords, tokens, or mail.

A site admin reverses one open package or user rejection with a new reason
(`POST /api/v1/audit/{id}/reverse`). That writes another row pointing at the rejection
and clears the suspension if no newer open rejection remains. The person signs in
again; the reverse does not mint a session or skip email verification. Reversing the
same row twice is refused. A rejected **request** is listed and stays closed: the
retry is to file a new request, not to undo the old one into an acceptance.

Rate-limit responses, malformed RPC, and ordinary permission errors are not audit
rows. They are not decisions a moderator or an upload policy made.

## What is not checked

TAUR has no package-registry table. An upload of a new package base follows the
existing push rules. See [upload policy](upload-policy.md) for the official AUR and
Arch comparison that is not implemented, and why.
