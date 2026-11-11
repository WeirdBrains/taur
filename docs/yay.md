# Using with yay and paru

TAUR is an alternate AUR endpoint: it speaks the AUR RPC and git layout. It is not a
pacman repository. Do not add a `Server=` line to `pacman.conf`. The package name on
the public instance is `taur`.

A helper clones the recipe, builds it as your unprivileged user, and uses `sudo` only
to install the built package. Do not run yay or paru as root.

The `taur` recipe served by a TAUR server must be the public Arch recipe: `PKGBUILD`,
`.SRCINFO`, and `taur.install`. Its `source` is the published GitHub release tarball
(`taur-VERSION-x86_64-unknown-linux-gnu.tar.gz`) with a verified checksum. A recipe
that lists a local `payload.tar.gz` cannot be built with a helper; that file exists
only in the internal official-package build.

The commands below are for the public instance, `https://pkg.taur.weirdbrains.com`.
For a local or self-hosted server, keep the same flags and replace that origin. Pass
the URLs on the command line (or via a shell alias) so your default helper config
still points at the official AUR. That way other AUR packages are not unexpectedly
sourced from TAUR.

yay and paru do not use the same RPC URL shape. The flags look similar, but the RPC
flag's value and yay's trailing `?` are not interchangeable. The spellings below were
taken from the helper `--help` and man pages, and checked against the binaries.

## Install a helper first

Neither yay nor paru is in the official Arch repositories. Install a helper from the
official AUR (or a binary package from that same AUR recipe) before you point it at
TAUR. You need `git` and the `base-devel` group, and `makepkg` must not be run as
root.

yay, from the [yay README](https://github.com/Jguer/yay) (retrieved 10 Nov 2026):

```sh
sudo pacman -S --needed git base-devel
git clone https://aur.archlinux.org/yay.git
cd yay
makepkg -si
```

To skip compiling yay, the same README documents `yay-bin`:

```sh
sudo pacman -S --needed git base-devel
git clone https://aur.archlinux.org/yay-bin.git
cd yay-bin
makepkg -si
```

paru, from the [paru README](https://github.com/Morganamilo/paru) (retrieved 10 Nov 2026).
That README's `pacman` line lists `base-devel` only; install `git` as well if it is not
already present:

```sh
sudo pacman -S --needed git base-devel
git clone https://aur.archlinux.org/paru.git
cd paru
makepkg -si
```

A prebuilt alternative is the AUR package `paru-bin` (`https://aur.archlinux.org/paru-bin.git`),
linked from the paru README badges.

## One-shot search, info, install, and update

### yay

yay 13.0.1 (`yay --help`, `yay(8)`) uses `--aururl` and `--aurrpcurl`. An empty
`aurrpcurl` becomes `https://aur.archlinux.org/rpc?` (`yay -Pg`). yay GETs query
parameters after that RPC URL. Use the trailing `?` to match that official default.

Quote the RPC URL in zsh (`?` is a glob).

```sh
yay --aururl https://pkg.taur.weirdbrains.com --aurrpcurl 'https://pkg.taur.weirdbrains.com/rpc?' -Ss taur
yay --aururl https://pkg.taur.weirdbrains.com --aurrpcurl 'https://pkg.taur.weirdbrains.com/rpc?' -Si taur
yay --aururl https://pkg.taur.weirdbrains.com --aurrpcurl 'https://pkg.taur.weirdbrains.com/rpc?' -S taur
```

The same `-S taur` invocation installs or updates `taur` only. Do not pass these URL
flags to `yay -Syu` unless you want every AUR upgrade to come from TAUR.

`-G taur` only retrieves the PKGBUILD tree into the current directory; it does not
build or install. yay clones `/{pkgbase}.git`.

### paru

paru 2.2.0 (`paru --help`, `paru(8)`, `paru.conf(5)`) also uses `--aururl` and
`--aurrpcurl`. The flag is spelled `--aurrpcurl` (with the final `l`), matching the
man page. In a narrow terminal, `paru --help` may visually truncate that flag to
`--aurrpcur`; that truncated spelling is an unknown option.

paru POSTs the RPC form to the configured URL as-is. With only `--aururl` set, paru
2.2.0 requests `{aururl}/rpc` with no `?` (source: `aur_url.join("rpc")`). Set both
flags explicitly, and omit the trailing `?`:

```sh
paru --aururl https://pkg.taur.weirdbrains.com --aurrpcurl https://pkg.taur.weirdbrains.com/rpc -Ss taur
paru --aururl https://pkg.taur.weirdbrains.com --aurrpcurl https://pkg.taur.weirdbrains.com/rpc -Si taur
paru --aururl https://pkg.taur.weirdbrains.com --aurrpcurl https://pkg.taur.weirdbrains.com/rpc -S taur
```

The same `-S taur` invocation installs or updates `taur` only. `-G taur` only retrieves
the tree (paru clones `/{pkgbase}`, with no `.git` suffix).

## Persistent configuration

Prefer the one-shot flags above, or a shell alias, so other AUR packages keep using
`https://aur.archlinux.org`. If you persist TAUR as the helper's AUR URL, every AUR
search, install, and upgrade goes to TAUR, and packages that exist only on the official
AUR will not be found.

### yay

`--save` writes `$XDG_CONFIG_HOME/yay/config.json` or `~/.config/yay/config.json`
(`yay(8)`). Edit that file through yay, not by hand.

```sh
# Points every AUR operation at TAUR. Usually not what you want.
yay --save --aururl https://pkg.taur.weirdbrains.com --aurrpcurl 'https://pkg.taur.weirdbrains.com/rpc?'
```

Restore the official AUR (`yay -Pd` defaults `aururl` to `https://aur.archlinux.org`
and `aurrpcurl` to empty; `yay -Pg` then shows `https://aur.archlinux.org/rpc?`):

```sh
yay --save --aururl https://aur.archlinux.org --aurrpcurl 'https://aur.archlinux.org/rpc?'
```

Safer than `--save`:

```sh
alias yay-taur="yay --aururl https://pkg.taur.weirdbrains.com --aurrpcurl 'https://pkg.taur.weirdbrains.com/rpc?'"
yay-taur -Ss taur
yay-taur -S taur
```

### paru

paru has no `--save` flag. Persistent settings are `AurUrl` and `AurRpcUrl` in
`$PARU_CONF`, else `$XDG_CONFIG_HOME/paru/paru.conf`, else `~/.config/paru/paru.conf`,
else `/etc/paru.conf` (`paru.conf(5)`). Prefer a user file or `PARU_CONF` over
`/etc/paru.conf`.

```ini
# Only if you intend every AUR operation to use TAUR.
[options]
AurUrl = https://pkg.taur.weirdbrains.com
AurRpcUrl = https://pkg.taur.weirdbrains.com/rpc
```

Restore defaults by deleting those two lines (unset, paru uses
`https://aur.archlinux.org` and `{aururl}/rpc`), or by setting:

```ini
[options]
AurUrl = https://aur.archlinux.org
AurRpcUrl = https://aur.archlinux.org/rpc
```

Safer than changing the default config:

```sh
alias paru-taur='paru --aururl https://pkg.taur.weirdbrains.com --aurrpcurl https://pkg.taur.weirdbrains.com/rpc'
paru-taur -Ss taur
paru-taur -S taur
```

## Stale cached `taur` recipe

If a previous clone of `taur` is stale, remove only that package's cache directory.
Do not delete the rest of the helper cache.

yay stores builds in `$XDG_CACHE_HOME/yay/` or `~/.cache/yay/` (`yay(8)`):

```sh
rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/yay/taur"
```

paru 2.2.0 clones into `$XDG_CACHE_HOME/paru/clone/` or `~/.cache/paru/clone/`
(`build_dir = cache.join("clone")` in paru v2.2.0). If you set `--clonedir`,
`CloneDir`, or `AURDEST`, remove `<that-directory>/taur` instead:

```sh
rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/paru/clone/taur"
```

You can also pass `--redownload` on a one-shot `-S taur` to fetch the PKGBUILD again.

## Local development server

```sh
taur --home /tmp/taur-dev import path/to/pkgdir
taur --home /tmp/taur-dev serve --bind 127.0.0.1:8080

yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Ss hello
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Si hello-taur
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -G hello-taur

paru --aururl http://127.0.0.1:8080 --aurrpcurl http://127.0.0.1:8080/rpc -Ss hello
paru --aururl http://127.0.0.1:8080 --aurrpcurl http://127.0.0.1:8080/rpc -Si hello-taur
paru --aururl http://127.0.0.1:8080 --aurrpcurl http://127.0.0.1:8080/rpc -G hello-taur
```

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
clone (`-G`) of `hello-taur`, including `PKGBUILD` and `.SRCINFO`. The harness passes
yay `--aurrpcurl …/rpc?` and paru `--aurrpcurl …/rpc`.
