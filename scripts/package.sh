#!/usr/bin/env bash
# Build a glibc 2.31 tarball, a .deb, an .rpm, and an Arch package for one
# already-published TAUR commit. Does not create or move a git tag.
#
#   scripts/package.sh 1.3.1
#   scripts/package.sh v1.3.1
#   scripts/package.sh a7c9237ba90d12cec5fd8bb9f108e8e8aee323b7
#   scripts/package.sh --stage-only --bin-dir DIR 1.3.1
#
# Binaries are compiled from that commit. Debian, RPM, and Arch packaging
# files come from this checkout, so a newer packaging tree can wrap an older
# tag without changing the tag. See docs/releases.md.
set -euo pipefail

die() { echo "package: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: scripts/package.sh [--output DIR] [--bin-dir DIR] [--stage-only] VERSION

VERSION is 1.2.3, v1.2.3, or a commit. The script checks that commit out in a
temporary worktree, builds taur and taurctl for x86_64-unknown-linux-gnu with
glibc 2.31 (cargo-zigbuild, Zig 0.14.1), and packs:

  taur-VERSION-x86_64-unknown-linux-gnu.tar.gz
  taur_VERSION-1_amd64.deb
  taur-VERSION-1.x86_64.rpm
  taur-VERSION-1-x86_64.pkg.tar.zst
  SHA256SUMS

--bin-dir uses those binaries instead of compiling. --stage-only writes the
filesystem tree and does not pack it. The script does not enable a service,
push a tag, or create a GitHub release.
EOF
}

output=""
bin_dir=""
stage_only=0
version_arg=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      [[ $# -ge 2 ]] || die "--output needs a directory"
      output="$2"
      shift
      ;;
    --bin-dir)
      [[ $# -ge 2 ]] || die "--bin-dir needs a directory"
      bin_dir="$2"
      shift
      ;;
    --stage-only) stage_only=1 ;;
    -h|--help) usage; exit 0 ;;
    --) shift; break ;;
    -*) die "unknown argument: $1" ;;
    *)
      [[ -z "$version_arg" ]] || die "unexpected extra argument: $1"
      version_arg="$1"
      ;;
  esac
  shift
done
[[ $# -eq 0 ]] || die "unexpected extra argument: $1"
[[ -n "$version_arg" ]] || die "VERSION is required"
if [[ "$stage_only" -eq 1 && -z "$bin_dir" ]]; then
  die "--stage-only needs --bin-dir"
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$script_dir/.." && pwd)"
cd "$repo"
[[ -d packaging/debian && -d packaging/rpm && -d packaging/arch ]] || die "packaging/ is incomplete"

resolve_ref() {
  local arg="$1"
  if [[ "$arg" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    git rev-parse --verify --quiet "refs/tags/v${arg}^{}" || die "tag v${arg} was not found"
    return
  fi
  if [[ "$arg" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    git rev-parse --verify --quiet "refs/tags/${arg}^{}" || die "tag ${arg} was not found"
    return
  fi
  git rev-parse --verify --quiet "${arg}^{}" || die "commit ${arg} was not found"
}

commit="$(resolve_ref "$version_arg")"
version="$(git show "${commit}:Cargo.toml" | python3 -c '
import sys
section = None
for line in sys.stdin:
    stripped = line.strip()
    if stripped.startswith("[") and stripped.endswith("]"):
        section = stripped
        continue
    if section == "[workspace.package]" and stripped.startswith("version"):
        print(stripped.split("\"", 2)[1])
        raise SystemExit
raise SystemExit("workspace version not found")
')"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "ref ${commit} has version ${version}, not MAJOR.MINOR.PATCH"
if [[ "$version_arg" =~ ^v?([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
  [[ "$version" == "${BASH_REMATCH[1]}" ]] || die "ref ${version_arg} is version ${version}, not ${BASH_REMATCH[1]}"
fi

epoch="$(git log -1 --format=%ct "$commit")"
[[ "$output" ]] || output="$repo/dist"
mkdir -p "$output"
output="$(cd "$output" && pwd)"

worktree=""
stage=""
cleanup() {
  if [[ -n "$worktree" && -d "$worktree" ]]; then
    git -C "$repo" worktree remove --force "$worktree" >/dev/null 2>&1 || rm -rf "$worktree"
    git -C "$repo" worktree prune >/dev/null 2>&1 || true
  fi
  if [[ -n "$stage" && -d "$stage" ]]; then
    rm -rf "$stage"
  fi
}
trap cleanup EXIT
worktree="$(mktemp -d "${TMPDIR:-/tmp}/taur-package.XXXXXX")"
rmdir "$worktree"
git worktree add --detach "$worktree" "$commit" >/dev/null

stage="$(mktemp -d "${TMPDIR:-/tmp}/taur-stage.XXXXXX")"
root="$stage/root"
mkdir -p "$root/usr/bin" "$root/usr/lib/taur" "$root/usr/lib/systemd/system" \
  "$root/usr/lib/sysusers.d" "$root/usr/lib/tmpfiles.d" \
  "$root/usr/share/man/man1" "$root/usr/share/doc/taur" "$root/etc/taur"

if [[ -n "$bin_dir" ]]; then
  [[ -f "$bin_dir/taur" && -f "$bin_dir/taurctl" ]] || die "--bin-dir is missing taur or taurctl"
  install -m 0755 "$bin_dir/taur" "$root/usr/bin/taur"
  install -m 0755 "$bin_dir/taurctl" "$root/usr/bin/taurctl"
else
  zig_version="0.14.1"
  zig_sha="24aeeec8af16c381934a6cd7d95c807a8cb2cf7df9fa40d359aa884195c4716c"
  cache="${XDG_CACHE_HOME:-$HOME/.cache}/taur-package"
  zig_root="$cache/zig-x86_64-linux-${zig_version}"
  if [[ -n "${TAUR_ZIG:-}" ]]; then
    zig="$TAUR_ZIG"
  elif [[ -x "$zig_root/zig" ]]; then
    zig="$zig_root/zig"
  else
    mkdir -p "$cache"
    tarball="$cache/zig-${zig_version}.tar.xz"
    if [[ ! -f "$tarball" ]] || ! echo "${zig_sha}  ${tarball}" | sha256sum -c - >/dev/null 2>&1; then
      curl -fL --retry 3 -o "$tarball.partial" \
        "https://ziglang.org/download/${zig_version}/zig-x86_64-linux-${zig_version}.tar.xz"
      mv "$tarball.partial" "$tarball"
      echo "${zig_sha}  ${tarball}" | sha256sum -c -
    fi
    tar -C "$cache" -xf "$tarball"
    zig="$zig_root/zig"
  fi
  [[ -x "$zig" ]] || die "zig is not executable: $zig"
  export PATH="${HOME}/.cargo/bin:$(dirname "$zig"):$PATH"
  export CARGO_ZIGBUILD_ZIG_PATH="$zig"
  if ! command -v cargo-zigbuild >/dev/null 2>&1; then
    cargo install --locked cargo-zigbuild --version "${TAUR_CARGO_ZIGBUILD_VERSION:-0.24.1}"
  fi
  (
    cd "$worktree"
    # A caller-supplied linker would skip Zig and link the host glibc.
    unset CARGO_TARGET_DIR RUSTFLAGS
    SOURCE_DATE_EPOCH="$epoch" cargo zigbuild --release --locked \
      --target x86_64-unknown-linux-gnu.2.31 \
      --bin taur --bin taurctl
  )
  built="$worktree/target/x86_64-unknown-linux-gnu/release"
  install -m 0755 "$built/taur" "$root/usr/bin/taur"
  install -m 0755 "$built/taurctl" "$root/usr/bin/taurctl"
fi

need_libgcc=0
for bin in "$root/usr/bin/taur" "$root/usr/bin/taurctl"; do
  if ! file "$bin" | grep -q 'ELF 64-bit'; then
    if [[ "$stage_only" -eq 1 ]]; then
      continue
    fi
    die "$bin is not an x86-64 ELF binary"
  fi
  command -v objdump >/dev/null 2>&1 || die "objdump is required to check glibc versions"
  max="$(objdump -T "$bin" | sed -n 's/.*GLIBC_\([0-9][0-9.]*\).*/\1/p' | sort -Vu | tail -1)"
  [[ -n "$max" ]] || die "$bin has no GLIBC versions"
  python3 - "$max" <<'PY'
import sys
def parts(s):
    return [int(x) for x in s.split(".")]
if parts(sys.argv[1]) > parts("2.31"):
    sys.exit(f"package: {sys.argv[1]} is newer than glibc 2.31")
PY
  needed="$(objdump -p "$bin" | awk '/NEEDED/ { print $2 }')"
  if grep -Eq 'libssl|libcrypto' <<<"$needed"; then
    die "$bin links OpenSSL: $needed"
  fi
  if grep -q 'libgcc_s.so.1' <<<"$needed"; then
    need_libgcc=1
  fi
  echo "package: $(basename "$bin") GLIBC_${max} needed: ${needed//$'\n'/ }"
done
mkdir -p "$output/bins"
install -m 0755 "$root/usr/bin/taur" "$output/bins/taur"
install -m 0755 "$root/usr/bin/taurctl" "$output/bins/taurctl"

shopt -s nullglob
for man in "$worktree"/man/*.1; do
  gzip -n -9 -c "$man" > "$root/usr/share/man/man1/$(basename "$man").gz"
done
shopt -u nullglob
cp -a "$worktree/docs/." "$root/usr/share/doc/taur/docs/"
install -m 0644 "$worktree/README.md" "$root/usr/share/doc/taur/README.md"
install -m 0644 "$worktree/CHANGELOG.md" "$root/usr/share/doc/taur/CHANGELOG.md"
install -m 0755 "$repo/packaging/account/ensure-account" "$root/usr/lib/taur/ensure-account"
install -m 0644 "$repo/packaging/systemd/taur.service" "$root/usr/lib/systemd/system/taur.service"
install -m 0644 "$repo/packaging/systemd/taur.sysusers" "$root/usr/lib/sysusers.d/taur.conf"
install -m 0644 "$repo/packaging/systemd/taur.tmpfiles" "$root/usr/lib/tmpfiles.d/taur.conf"
install -m 0640 "$repo/packaging/taur.env.example" "$root/etc/taur/taur.env"
chmod 0750 "$root/etc/taur"
mkdir -p "$root/usr/share/licenses/taur"
install -m 0644 "$repo/packaging/licenses/MIT" "$root/usr/share/licenses/taur/MIT"
install -m 0644 "$repo/packaging/licenses/Apache-2.0" "$root/usr/share/licenses/taur/Apache-2.0"
find "$root" -exec touch -h -d "@${epoch}" {} +

if [[ "$stage_only" -eq 1 ]]; then
  rm -rf "$output/stage"
  mkdir -p "$output"
  mv "$root" "$output/stage"
  echo "package: staged $output/stage from $commit ($version)"
  exit 0
fi

extra_depends=""
arch_depends="('git' 'systemd')"
if [[ "$need_libgcc" -eq 1 ]]; then
  extra_depends=", libgcc-s1"
  arch_depends="('git' 'systemd' 'gcc-libs')"
fi

# Debian
deb_root="$stage/deb"
rm -rf "$deb_root"
mkdir -p "$deb_root/DEBIAN"
cp -a "$root/." "$deb_root/"
install -m 0644 "$repo/packaging/debian/copyright" "$deb_root/usr/share/doc/taur/copyright"
deb_date="$(date -u -d "@${epoch}" "+%a, %d %b %Y %H:%M:%S +0000")"
cat > "$deb_root/usr/share/doc/taur/changelog.Debian" <<EOF
taur (${version}-1) unstable; urgency=medium

  * Package TAUR ${version}.

 -- WeirdBrains <taur@weirdbrains.com>  ${deb_date}
EOF
gzip -n -9 "$deb_root/usr/share/doc/taur/changelog.Debian"
sed -e "s/@VERSION@/${version}/g" -e "s/@EXTRA_DEPENDS@/${extra_depends}/g" \
  "$repo/packaging/debian/control.in" > "$deb_root/DEBIAN/control"
install -m 0755 "$repo/packaging/debian/postinst" "$deb_root/DEBIAN/postinst"
install -m 0755 "$repo/packaging/debian/prerm" "$deb_root/DEBIAN/prerm"
install -m 0755 "$repo/packaging/debian/postrm" "$deb_root/DEBIAN/postrm"
printf '/etc/taur/taur.env\n' > "$deb_root/DEBIAN/conffiles"
find "$deb_root" -exec touch -h -d "@${epoch}" {} +
deb_name="taur_${version}-1_amd64.deb"
if command -v dpkg-deb >/dev/null 2>&1; then
  SOURCE_DATE_EPOCH="$epoch" dpkg-deb --root-owner-group --uniform-compression -Zxz -z9 \
    --build "$deb_root" "$output/$deb_name"
else
  command -v podman >/dev/null 2>&1 || die "dpkg-deb or podman is required to build the .deb"
  podman run --rm --network=none \
    -e SOURCE_DATE_EPOCH="$epoch" \
    -v "$deb_root:/pkg:ro" \
    -v "$output:/out" \
    docker.io/library/debian:12 \
    dpkg-deb --root-owner-group --uniform-compression -Zxz -z9 --build /pkg "/out/$deb_name"
fi

# RPM
rpm_date="$(date -u -d "@${epoch}" "+%a %b %d %Y")"
spec="$stage/taur.spec"
sed -e "s/@VERSION@/${version}/g" -e "s/@RPM_DATE@/${rpm_date}/g" \
  "$repo/packaging/rpm/taur.spec" > "$spec"
rpm_out="$stage/rpm-top"
rm -rf "$rpm_out"
if command -v rpmbuild >/dev/null 2>&1; then
  getent group taur >/dev/null || die "rpmbuild needs a local taur group so the config owner is recorded; use podman by hiding rpmbuild"
  SOURCE_DATE_EPOCH="$epoch" rpmbuild -bb \
    --define "_topdir $rpm_out" \
    --define "taur_stage $root" \
    --define "_buildhost reproducible" \
    "$spec"
else
  command -v podman >/dev/null 2>&1 || die "rpmbuild or podman is required to build the .rpm"
  mkdir -p "$rpm_out"
  podman run --rm \
    -e SOURCE_DATE_EPOCH="$epoch" \
    -v "$root:/stage:ro" \
    -v "$spec:/taur.spec:ro" \
    -v "$rpm_out:/top" \
    docker.io/library/fedora:45 \
    bash -c 'dnf install -y rpm-build >/dev/null && groupadd --system taur && useradd --system --gid taur --shell /usr/sbin/nologin taur && rpmbuild -bb --define "_topdir /top" --define "taur_stage /stage" --define "_buildhost reproducible" /taur.spec'
fi
rpm_built="$(find "$rpm_out" -name "taur-${version}-1*.rpm" -type f | head -1)"
[[ -n "$rpm_built" ]] || die "rpmbuild produced no package"
rpm_name="taur-${version}-1.x86_64.rpm"
cp -a "$rpm_built" "$output/$rpm_name"

# Arch
arch_dir="$stage/arch"
rm -rf "$arch_dir"
mkdir -p "$arch_dir"
tar --sort=name --mtime="@${epoch}" --owner=0 --group=0 --numeric-owner \
  -C "$root" -czf "$arch_dir/payload.tar.gz" .
payload_sum="$(sha256sum "$arch_dir/payload.tar.gz" | awk '{print $1}')"
sed -e "s/^pkgver=.*/pkgver=${version}/" \
  -e "s/^depends=.*/depends=${arch_depends}/" \
  -e "s/^sha256sums=.*/sha256sums=('${payload_sum}')/" \
  "$repo/packaging/arch/PKGBUILD" > "$arch_dir/PKGBUILD"
cp "$repo/packaging/arch/taur.install" "$arch_dir/taur.install"
(
  cd "$arch_dir"
  SOURCE_DATE_EPOCH="$epoch" PACKAGER="WeirdBrains <taur@weirdbrains.com>" \
    makepkg --nodeps --force --noconfirm
)
arch_built="$(find "$arch_dir" -name "taur-${version}-1-x86_64.pkg.tar.*" -type f | head -1)"
[[ -n "$arch_built" ]] || die "makepkg produced no package"
arch_name="taur-${version}-1-x86_64.pkg.tar.zst"
cp -a "$arch_built" "$output/$arch_name"
if command -v namcap >/dev/null 2>&1; then
  namcap "$arch_dir/PKGBUILD" > "$output/namcap-pkgbuild.txt" || true
  namcap "$output/$arch_name" > "$output/namcap-package.txt" || true
  if grep -E '(^| )E:' "$output/namcap-pkgbuild.txt" "$output/namcap-package.txt"; then
    echo "package: namcap errors" >&2
    cat "$output/namcap-pkgbuild.txt" "$output/namcap-package.txt" >&2
    die "namcap reported an error"
  fi
fi

# Tarball of the binaries plus this checkout's packaging files.
tar_root="$stage/tar"
rm -rf "$tar_root"
mkdir -p "$tar_root/taur-${version}"
prefix="$tar_root/taur-${version}"
install -m 0755 "$root/usr/bin/taur" "$prefix/taur"
install -m 0755 "$root/usr/bin/taurctl" "$prefix/taurctl"
install -m 0644 "$worktree/README.md" "$prefix/README.md"
install -m 0644 "$worktree/CHANGELOG.md" "$prefix/CHANGELOG.md"
cp -a "$worktree/docs" "$prefix/docs"
cp -a "$worktree/man" "$prefix/man"
mkdir -p "$prefix/packaging"
cp -a "$repo/packaging/." "$prefix/packaging/"
(
  cd "$prefix"
  find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
)
tar_name="taur-${version}-x86_64-unknown-linux-gnu.tar.gz"
tar --sort=name --mtime="@${epoch}" --owner=0 --group=0 --numeric-owner \
  -C "$tar_root" -czf "$output/$tar_name" "taur-${version}"

(
  cd "$output"
  sha256sum "$tar_name" "$deb_name" "$rpm_name" "$arch_name" > SHA256SUMS
)
echo "package: $version $commit"
echo "package: wrote $output"
cat "$output/SHA256SUMS"
