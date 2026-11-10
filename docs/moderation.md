# Moderators and the audit

A **moderator** can reject a package base or an ordinary account. That is the only extra
ability. A moderator cannot grant roles, accept requests, manage other people's packages,
or reverse a rejection. Site admins (`taur admin`, or `PUT /api/v1/admin/users/{name}/admin`
and `PUT /api/v1/admin/moderators/{name}` with `{"on":true}`) grant and remove roles.
Moderators cannot suspend a site admin or another moderator. The last site admin cannot
be removed from the web.

On a fresh database, with `TAUR_ADMINS` and `TAUR_OWNERS` unset and no site admin
or site owner yet, the first successful registration is the site owner and a site
admin, and is verified. That decision is an audit row with source `bootstrap`.
Either variable, or an existing admin or owner (`taur admin`, `taur owner`), turns
this off. An existing server that already has an admin does not gain an owner until
`taur owner`.

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
reason, time, actor name, and source (`moderator`, `admin`, or `registry` when the
upload or registration gate rejected it). It does not store
package contents, passwords, tokens, or mail.

A site admin reverses one open package or user rejection with a new reason
(`POST /api/v1/audit/{id}/reverse`). That writes another row pointing at the rejection
and clears the suspension if no newer open rejection remains. The person signs in
again; the reverse does not mint a session or skip email verification. Reversing the
same row twice is refused. A rejected **request** is listed and stays closed: the
retry is to file a new request, not to undo the old one into an acceptance.

Registry and registration rejections are in the same audit (`registry_upload`,
`registration`).

```sh
taurctl moderation accept 12 "this exact attempt is allowed"
taurctl moderation revoke 13 "withdrawn"
```

**Accept** (`POST /api/v1/audit/{id}/accept`, gRPC `AcceptRejection`, MCP
`accept_rejection`, and the Accept button on `/audit`) lets that exact attempt be
retried. The upload and registration gates read the latest decision for its
scope. Only a current `accept` skips the business signature or identity rule
that rejected it. The recipe bytes and the password are not kept; the retry
sends them again. A changed file, version, repository, package, registry,
applicant, or registration email does not match, and neither does another
registry's identity. The decision stays until a newer rejection of that same
scope or **Revoke** (`POST /api/v1/audit/{id}/revoke`, gRPC `RevokeAcceptance`,
MCP `revoke_acceptance`). **Reverse** on these rows records a review and does
not permit the retry. A moderator cannot accept or revoke. See
[package registries](upload-policy.md) for the scope and for what stays in force.

Rate-limit responses, malformed RPC, and ordinary permission errors are not audit
rows. They are not decisions a moderator or an upload policy made.

## What is not checked

`local` and a package name with no operator expectation keep the previous upload
rules. Operator signatures are not Arch Linux or AUR verification. See
[package registries](upload-policy.md).
