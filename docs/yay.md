# Using with yay and paru

Pass the server URL on the command line. That leaves the real yay and paru config files
untouched. Do not pass `--save`.

```sh
taur --home /tmp/taur-dev import path/to/pkgdir
taur --home /tmp/taur-dev serve --bind 127.0.0.1:8080

yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Ss hello
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Si hello-taur
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -G hello-taur

paru --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Ss hello
paru --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Si hello-taur
paru --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -G hello-taur
```

Quote `--aurrpcurl` in zsh (`?` is a glob). yay GETs that URL and clones
`/{pkgbase}.git`. paru POSTs the RPC form and clones `/{pkgbase}` (no `.git` suffix).
`-G` only retrieves the PKGBUILD; it does not build or install.

yay and paru both refuse a recipe whose `.SRCINFO` pairs `arch = any` with an
architecture-specific field such as `depends_x86_64`. TAUR rejects that recipe on
import and on push. See [import](import.md).

## Automated tests

`cargo test -p taur-server --test helpers` boots TAUR on `127.0.0.1` with a temporary
data directory, imports `tests/fixtures/hello-taur`, and runs every helper it can
find. The harness gives each helper a private `HOME`, `XDG_CONFIG_HOME`,
`XDG_CACHE_HOME`, `XDG_DATA_HOME`, and pacman root. `makepkg` and `sudo` are replaced
with commands that fail if invoked, so the test cannot install packages or execute
the fixture PKGBUILD.

```sh
cargo test -p taur-server --test helpers
TAUR_YAY=/usr/bin/yay TAUR_PARU=/usr/bin/paru TAUR_REQUIRE_HELPERS=yay,paru \
  cargo test -p taur-server --test helpers
```

`TAUR_YAY` and `TAUR_PARU` select binaries. Without those variables the test uses
`PATH`. A missing helper is skipped unless it is listed in `TAUR_REQUIRE_HELPERS`.
The test prints each helper's version. It checks search (`-Ss`), info (`-Sii`), and
clone (`-G`) of `hello-taur`, including `PKGBUILD` and `.SRCINFO`.
