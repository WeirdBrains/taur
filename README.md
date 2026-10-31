# TAUR

An AUR-compatible package server in Rust. `yay` and other AUR helpers can search,
inspect and clone packages from it.

```sh
cargo build
taur --home /tmp/taur import path/to/pkgdir   # dir with PKGBUILD + .SRCINFO
taur --home /tmp/taur serve --bind 127.0.0.1:8080
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Ss foo
```

Manage accounts, orgs and packages with `taurctl` (see [docs/cli.md](docs/cli.md)).

See [docs/](docs/README.md), [PLAN.md](PLAN.md) and [AGENTS.md](AGENTS.md).
