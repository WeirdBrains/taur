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
URLPath`. `info` adds `Depends, MakeDepends, OptDepends, CheckDepends, Conflicts,
Provides, Replaces, Groups` (omitted when empty) and `License, Keywords, CoMaintainers`.

Errors: `No request type/data specified.`, `Incorrect request type specified.`,
`Incorrect by field specified.`, `Query arg too small.` (name searches < 2 chars),
`Too many package results.` (> 5000), `Invalid version specified.`.

```sh
curl 'http://127.0.0.1:8080/rpc/v5/search/foo?by=name'
```
