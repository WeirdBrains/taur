# TAUR

An AUR-compatible package server in Rust. `yay` and other AUR helpers can search,
inspect and clone packages from it.

```sh
cargo build
taur --home /tmp/taur import path/to/pkgdir   # dir with PKGBUILD + .SRCINFO
taur --home /tmp/taur serve --bind 127.0.0.1:8080
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Ss foo
```

Configure the server with environment variables: copy [.env.example](.env.example) to `.env`
(see [docs/configuration.md](docs/configuration.md)).

Manage accounts, orgs and packages with `taurctl` (see [docs/cli.md](docs/cli.md)).
Git over SSH works with the built-in server or OpenSSH (`taur ssh-auth` /
`taur git-serve`; see [docs/ssh-keys.md](docs/ssh-keys.md)). Maintainer pages live at
`/users/<name>`.

See [docs/](docs/README.md), [PLAN.md](PLAN.md) and [AGENTS.md](AGENTS.md).
