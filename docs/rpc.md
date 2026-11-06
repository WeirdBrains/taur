# AUR RPC v5

Responses match aur.archlinux.org: `{"resultcount", "results", "type", "version": 5}`;
errors add `"error"` with `"type": "error"`.

| Request | Forms |
|---|---|
| info | `/rpc/v5/info?arg[]=a&arg[]=b`, `/rpc/v5/info/a`, `POST /rpc/v5/info` (form), `/rpc?v=5&type=info&arg[]=a` (`multiinfo` too) |
| search | `/rpc/v5/search/ARG?by=FIELD`, `/rpc?v=5&type=search&by=FIELD&arg=ARG` |
| msearch | `/rpc?v=5&type=msearch&arg=USER` (empty arg = orphans) |
| suggest | `/rpc/v5/suggest/PREFIX` → JSON array of ≤20 package names |
| suggest-pkgbase | `/rpc/v5/suggest-pkgbase/PREFIX` → JSON array of ≤20 pkgbase names |

`by` fields: `name`, `name-desc` (default), `maintainer`, `submitter`, `depends`,
`makedepends`, `optdepends`, `checkdepends`, `provides`, `conflicts`, `replaces`,
`groups`, `keywords`, `comaintainers`. Dependency fields match the name without version
constraints or optdepends descriptions.

Package fields: `ID, Name, PackageBaseID, PackageBase, Version, Description, URL,
NumVotes, Popularity, OutOfDate, Maintainer, Submitter, FirstSubmitted, LastModified,
URLPath`, plus the TAUR extension `Organization` (owning organization or null; AUR
clients ignore it). `info` adds `Depends, MakeDepends, OptDepends, CheckDepends, Conflicts,
Provides, Replaces, Groups` (omitted when empty) and `License, Keywords, CoMaintainers`.

Errors: `No request type/data specified.`, `Incorrect request type specified.`,
`Incorrect by field specified.`, `Query arg too small.` (name searches < 2 chars),
`Too many package results.` (> 5000), `Invalid version specified.`,
`Please specify an API version.`, `Invalid callback name.`.

`TAUR_RPC_RATE_LIMIT` (default `0`, off) counts RPC calls per TCP peer in a
`TAUR_RPC_RATE_WINDOW` (default 60 seconds). Over the limit the response is HTTP 429 and
`{"error":"Rate limit reached",...}`. The counter uses the socket address.
`X-Forwarded-For` and similar headers are ignored. In-process calls with no socket share
one bucket. Several server processes do not share counters.

`callback` (query or form) returns JSONP: `text/javascript` body `/**/callback(…)`.
The name must match aurweb's `^[A-Za-z0-9()_.]{1,128}$`.

On `/rpc` and `/rpc/v5/{type}` (no name in the path), argument handling follows aurweb:
a final `arg` is the only argument; a final `arg[]` keeps only the trailing `arg[]`
values, last to first. `/rpc/v5/info/{name}` keeps the path name first and then query
arguments in URL order.

`POST /rpc/v5/info` and `POST /rpc/v5/search` also accept `application/json`. `info`
requires `"arg"` to be an array (`the 'arg' parameter must be of array type`, HTTP 400).
`search` requires string `"arg"` and `"by"`.

yay GETs `/rpc?v=5&type=…` (the `--aurrpcurl` value already ends in `?`). paru POSTs the
same fields as `application/x-www-form-urlencoded` to `/rpc`.

```sh
curl 'http://127.0.0.1:8080/rpc/v5/search/foo?by=name'
```
