# Git clone and snapshots

Each package base has a bare repo at `<home>/repos/<pkgbase>.git`, served over git's
dumb HTTP protocol: `git clone http://HOST/<pkgbase>.git`. Only `HEAD`, `info/*`,
`objects/*`, `refs/*` and `packed-refs` are served; other paths return 404.

Snapshots: `GET /cgit/aur.git/snapshot/<pkgbase>.tar.gz` (this is the RPC `URLPath`),
a `git archive` of HEAD with prefix `<pkgbase>/`.

Uses the system `git` with system/global config disabled. Push (smart HTTP/ssh) is
planned for phase 2. Unknown package → 404.
