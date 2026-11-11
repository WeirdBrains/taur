---
title: Search and install with yay or paru
summary: Point a helper at this server and install the way you would from the AUR.
audience: user
order: 10
---

TAUR is an alternate AUR endpoint, not a pacman repository. Do not add a `Server=`
line to `pacman.conf`. Helpers clone the recipe, build it unprivileged, and use
`sudo` only to install the built package. The package name on the public instance is
`taur`.

Neither yay nor paru is in the official Arch repositories. Install a helper from the
official AUR first (you need `git` and `base-devel`; do not run `makepkg` as root):

```sh
sudo pacman -S --needed git base-devel
git clone https://aur.archlinux.org/yay.git
cd yay
makepkg -si
```

```sh
sudo pacman -S --needed git base-devel
git clone https://aur.archlinux.org/paru.git
cd paru
makepkg -si
```

Binary alternatives are `yay-bin` and `paru-bin` on the official AUR. Full setup
notes are in the [yay](https://github.com/Jguer/yay) and
[paru](https://github.com/Morganamilo/paru) READMEs.

## One-shot commands for the public instance

Pass the URLs on the command line so your default helper config still uses the
official AUR. Other AUR packages are then not sourced from TAUR. yay and paru are
not identical: yay's RPC URL keeps a trailing `?`; paru's does not. paru's flag is
`--aurrpcurl` (with the final `l`), even if `paru --help` truncates the name.

```sh
yay --aururl https://pkg.taur.weirdbrains.com --aurrpcurl 'https://pkg.taur.weirdbrains.com/rpc?' -Ss taur
yay --aururl https://pkg.taur.weirdbrains.com --aurrpcurl 'https://pkg.taur.weirdbrains.com/rpc?' -Si taur
yay --aururl https://pkg.taur.weirdbrains.com --aurrpcurl 'https://pkg.taur.weirdbrains.com/rpc?' -S taur
```

```sh
paru --aururl https://pkg.taur.weirdbrains.com --aurrpcurl https://pkg.taur.weirdbrains.com/rpc -Ss taur
paru --aururl https://pkg.taur.weirdbrains.com --aurrpcurl https://pkg.taur.weirdbrains.com/rpc -Si taur
paru --aururl https://pkg.taur.weirdbrains.com --aurrpcurl https://pkg.taur.weirdbrains.com/rpc -S taur
```

Quote yay's RPC URL in zsh (`?` is a glob). The same `-S taur` line installs or
updates `taur` only. For another TAUR, keep the flags and replace the origin.

Do not persist these URLs unless you want every AUR operation to use TAUR. To
restore official defaults, or to remove only a stale `taur` cache directory, see
[Using with yay and paru](https://github.com/WeirdBrains/taur/blob/main/docs/yay.md).
A package page on this site is the URL the helper shows as the AUR page.
