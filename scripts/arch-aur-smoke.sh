#!/usr/bin/env bash
# Build the public AUR recipe in a disposable Arch container, starting from
# only the files yay clones (PKGBUILD, .SRCINFO, taur.install). Downloads the
# published GitHub tarball named in the recipe. Does not install on the host
# and does not use a local payload.tar.gz directory.
set -euo pipefail

die() { echo "arch-aur-smoke: $*" >&2; exit 1; }
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$script_dir/.." && pwd)"
recipe="${1:-$repo/packaging/arch}"
[[ -f "$recipe/PKGBUILD" && -f "$recipe/.SRCINFO" && -f "$recipe/taur.install" ]] \
  || die "usage: scripts/arch-aur-smoke.sh [RECIPE_DIR] (needs PKGBUILD, .SRCINFO, taur.install)"
command -v podman >/dev/null 2>&1 || die "podman is required"
source_lines=$(awk '/^source=/ || /^\tsource =/' "$recipe/PKGBUILD" "$recipe/.SRCINFO")
sum_lines=$(awk '/^sha256sums=/ || /^\tsha256sums =/' "$recipe/PKGBUILD" "$recipe/.SRCINFO")
printf "%s\n" "$source_lines" | grep -q 'payload\.tar\.gz' \
  && die "public recipe still references payload.tar.gz"
printf "%s\n" "$sum_lines" | grep -Eq "SKIP|UPSTREAM_TARBALL_SHA256" \
  && die "public recipe sha256sums is still SKIP or UPSTREAM_TARBALL_SHA256"
printf "%s\n" "$sum_lines" | grep -Eq '[0-9a-f]{64}' \
  || die "public recipe sha256sums must be the published tarball digest"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Copy only the yay clone set. Do not include any sibling payload or dist dir.
cp "$recipe/PKGBUILD" "$recipe/.SRCINFO" "$recipe/taur.install" "$work/"
# Guard: the work dir must not already contain a payload or tarball.
# Include .SRCINFO (a dotfile). Do not match . or ...
shopt -s dotglob nullglob
extras=("$work"/*)
shopt -u dotglob nullglob
count="${#extras[@]}"
[[ "$count" -eq 3 ]] || die "expected only PKGBUILD, .SRCINFO, and taur.install in the clone dir (found $count: ${extras[*]})"
[[ -f "$work/PKGBUILD" && -f "$work/.SRCINFO" && -f "$work/taur.install" ]]   || die "clone dir is missing PKGBUILD, .SRCINFO, or taur.install"

log="$work/build.log"
# The container is disposable. Network is required to fetch the GitHub source.
podman run --rm \
  -v "$work:/recipe:z" \
  -w /recipe \
  docker.io/library/archlinux:latest \
  bash -ceu '
    set -euo pipefail
    fail() { echo "FAIL: $*" >&2; exit 1; }
    pacman-key --init >/dev/null
    pacman -Syu --noconfirm --needed base-devel git systemd >/dev/null
    useradd -m build
    install -d -o build -g build /work
    cp /recipe/PKGBUILD /recipe/.SRCINFO /recipe/taur.install /work/
    # Confirm no payload was provided to the build user.
    if ls /work/payload.tar.gz >/dev/null 2>&1; then
      fail "payload.tar.gz must not be present in the yay clone directory"
    fi
    cd /work
    su -s /bin/bash build -c "makepkg --noconfirm --syncdeps --cleanbuild"
    pkg="$(echo taur-*-x86_64.pkg.tar.*)"
    [[ -f "$pkg" ]] || fail "makepkg produced no package"
    pacman -U --noconfirm "$pkg"
    [ -x /usr/bin/taur ] || fail "missing /usr/bin/taur"
    [ -x /usr/bin/taurctl ] || fail "missing /usr/bin/taurctl"
    out=$(cd /tmp && env -i PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" HOME="/tmp" \
      /bin/sh -c "command -v taur && command -v taurctl && taur --version && taurctl --version") \
      || fail "taur and taurctl are not both on PATH from /tmp"
    taur_path=$(printf "%s\n" "$out" | sed -n "1p")
    ctl_path=$(printf "%s\n" "$out" | sed -n "2p")
    [ "$(readlink -f "$taur_path")" = "$(readlink -f /usr/bin/taur)" ] \
      || fail "taur resolved to $taur_path, which is not /usr/bin/taur"
    [ "$(readlink -f "$ctl_path")" = "$(readlink -f /usr/bin/taurctl)" ] \
      || fail "taurctl resolved to $ctl_path, which is not /usr/bin/taurctl"
    ver=$(printf "%s\n" "$out" | sed -n "3p" | awk "{print \$NF}")
    ctl=$(printf "%s\n" "$out" | sed -n "4p" | awk "{print \$NF}")
    [ -n "$ver" ] && [ "$ver" = "$ctl" ] || fail "taurctl $ctl != taur $ver"
    echo "$ver" | grep -Eq "^[0-9]+\\.[0-9]+\\.[0-9]+$" || fail "version $ver"
    echo "SMOKE_OK $ver"
  ' | tee "$log"
grep -q "SMOKE_OK " "$log" || die "container build did not print SMOKE_OK"
echo "arch-aur-smoke: ok"
