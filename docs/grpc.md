# gRPC API

`taur serve --grpc-bind 127.0.0.1:50051` serves a tonic gRPC API that mirrors the JSON
API. With `TAUR_TLS=off` the listener is plaintext HTTP/2 (terminate TLS in a proxy if
you publish it). With `TAUR_TLS=acme` the same certificate as HTTPS is used and the
listener is TLS; do not pass `-plaintext` to clients in that mode. Schema:
[`crates/taur-server/proto/taur/v1/taur.proto`](../crates/taur-server/proto/taur/v1/taur.proto)
(package `taur.v1`). It is compiled at build time with a bundled `protoc`
(`protoc-bin-vendored`), so no system protobuf is needed.

## Auth
Send metadata `authorization: Bearer <token>`; get a token from `Accounts.Login` or the
HTTP API. Write calls need a verified email; permissions are identical to the JSON API.

## Services
- `Packages` (public): `Search` (`by` defaults to `name-desc`), `Info`, `Comments`, `GetFlag`.
- `Accounts`: `Register`, `Login`, `Logout`, `Whoami`, `Unregister`, `Verify`,
  `ResendVerification`, `RequestPasswordReset`, `ResetPassword`.
- `Orgs`: `List`, `Get`, `Create`, `Delete`, `SetMember`, `RemoveMember`.
- `Manage`: `MyPackages`, `Transfer` (to org or user), `Disown`, `Adopt`, `SetComaintainers`.
- `Community`: `Vote`/`VoteStatus`, `Notify`/`NotifyStatus`, `FlagOutOfDate`, `Unflag`,
  `AddComment`, `EditComment`, `DeleteComment`, `PinComment`.
- `Requests`: `File`, `List`, `Get`, `Accept`, `Reject`.
- `SshKeys`: `List`, `Add`, `Remove`.

## Errors
`UNAUTHENTICATED` (missing/invalid token), `PERMISSION_DENIED` (forbidden or
unverified), `NOT_FOUND`, `ALREADY_EXISTS`, `INVALID_ARGUMENT`,
`RESOURCE_EXHAUSTED` (rate limit; metadata `retry-after`), `INTERNAL`.

Account methods (`Register`, `Login`, `Verify`, `ResendVerification`,
`RequestPasswordReset`, `ResetPassword`) use `TAUR_AUTH_RATE_LIMIT`. Other methods use
`TAUR_GRPC_RATE_LIMIT`.

Example: `grpcurl -plaintext -import-path crates/taur-server/proto -proto taur/v1/taur.proto
-d '{"query":"hello"}' 127.0.0.1:50051 taur.v1.Packages/Search`
