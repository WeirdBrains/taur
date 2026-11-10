#!/usr/bin/env bash
# Install dist/ packages in clean containers and print a pass/fail line each.
# Does not change the host. systemd is used when the container is already
# running it; otherwise `taur serve` runs as the taur user.
set -euo pipefail

die() { echo "smoke: $*" >&2; exit 1; }
dist="${1:-}"
[[ -n "$dist" && -d "$dist" ]] || die "usage: scripts/package-smoke.sh DISTDIR"
dist="$(cd "$dist" && pwd)"
command -v podman >/dev/null 2>&1 || die "podman is required"

shopt -s nullglob
debs=("$dist"/taur_*_amd64.deb)
rpms=("$dist"/taur-*.x86_64.rpm)
pkgs=("$dist"/taur-*.pkg.tar.zst)
shopt -u nullglob
[[ ${#debs[@]} -eq 1 && ${#rpms[@]} -eq 1 && ${#pkgs[@]} -eq 1 ]] || die "need one deb, one rpm, and one Arch package"
deb="$(basename "${debs[0]}")"
rpm="$(basename "${rpms[0]}")"
pkg="$(basename "${pkgs[0]}")"

guest="$(mktemp)"
cat >"$guest" <<'EOF'
set -eu
fail() { echo "FAIL: $*" >&2; exit 1; }
echo "CHECK_START kind=${KIND:-unset}"
ver=$(/usr/bin/taur --version | awk '{print $NF}')
ctl=$(/usr/bin/taurctl --version | awk '{print $NF}')
[ "$ver" = "$ctl" ] || fail "taurctl $ctl != taur $ver"
echo "$ver" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || fail "version $ver"
acme=$(mktemp -d)
TAUR_HOME="$acme" TAUR_TLS=acme TAUR_BIND=127.0.0.1:18080 TAUR_DOMAIN=pkg.example.com \
  TAUR_ACME_CONTACT=mailto:admin@example.com TAUR_ACME_ACCEPT_TERMS=yes \
  TAUR_ACME_HTTP_BIND=127.0.0.1:18081 \
  TAUR_ACME_DIRECTORY=https://127.0.0.1:9/directory \
  /usr/bin/taur serve >"$acme/serve.log" 2>&1 &
acme_pid=$!
acme_ok=0
i=0
while [ "$i" -lt 20 ]; do
  if grep -q CryptoProvider "$acme/serve.log" || grep -q 'panicked at' "$acme/serve.log"; then
    cat "$acme/serve.log" >&2
    kill "$acme_pid" 2>/dev/null || true
    fail "ACME startup panicked"
  fi
  if grep -q 'certificate request failed' "$acme/serve.log"; then
    acme_ok=1
    break
  fi
  i=$((i + 1))
  sleep 0.25
done
kill "$acme_pid" 2>/dev/null || true
[ "$acme_ok" = 1 ] || { cat "$acme/serve.log" >&2; fail "ACME startup did not reach a directory error"; }
echo "ACME no panic"
command -v git >/dev/null || fail "git missing"
getent passwd taur >/dev/null || fail "user missing"
shell=$(getent passwd taur | awk -F: '{print $NF}')
case "$shell" in
  */nologin|*/false) ;;
  *) fail "shell is $shell" ;;
esac
got=$(stat -c '%A %U %G' /var/lib/taur)
[ "$got" = "drwxr-x--- taur taur" ] || fail "data dir $got"
got=$(stat -c '%A %U %G' /etc/taur)
[ "$got" = "drwxr-x--- root taur" ] || fail "etc dir $got"
got=$(stat -c '%A %U %G' /etc/taur/taur.env)
[ "$got" = "-rw-r----- root taur" ] || fail "env $got"
test -f /usr/lib/systemd/system/taur.service || fail "missing unit"
# Some container images refuse to unpack man pages and docs. The package still
# has the files; Debian, Rocky, and Alma install them.
doc_skipped() {
  if grep -Rqs 'path-exclude=/usr/share/man' /etc/dpkg/dpkg.cfg /etc/dpkg/dpkg.cfg.d 2>/dev/null \
    || grep -Rqs 'path-exclude=/usr/share/doc' /etc/dpkg/dpkg.cfg /etc/dpkg/dpkg.cfg.d 2>/dev/null \
    || grep -Rqs 'NoExtract.*usr/share/man' /etc/pacman.conf 2>/dev/null \
    || { command -v rpm >/dev/null 2>&1 && rpm -ql taur 2>/dev/null | grep -q '/usr/share/man/man1/taur.1'; }; then
    return 0
  fi
  return 1
}
if [ ! -f /usr/share/man/man1/taur.1.gz ]; then
  if doc_skipped; then
    echo "man page not unpacked by this image"
  else
    fail "missing man page"
  fi
fi
if [ ! -f /usr/share/doc/taur/README.md ]; then
  if doc_skipped; then
    echo "docs not unpacked by this image"
  else
    fail "missing docs"
  fi
fi
if command -v systemctl >/dev/null 2>&1; then
  state=$(systemctl is-enabled taur 2>/dev/null || true)
  [ "$state" != enabled ] || fail "service was enabled on install"
  active=$(systemctl is-active taur 2>/dev/null || true)
  [ "$active" != active ] || fail "service was started on install"
fi
case "$KIND" in
  deb) dpkg-query -W -f='${Conffiles}\n' taur | grep -q /etc/taur/taur.env || fail "not a conffile" ;;
  rpm) rpm -qc taur | grep -q /etc/taur/taur.env || fail "not an rpm config" ;;
  arch) pacman -Qii taur | grep -q taur.env || fail "not an Arch backup file" ;;
esac
printf '%s\n' '# taur-smoke-marker' >> /etc/taur/taur.env
how=foreground
if [ -d /run/systemd/system ] && systemctl is-system-running >/dev/null 2>&1; then
  systemctl enable --now taur
  how=systemd
else
  su -s /bin/sh taur -c 'set -a; . /etc/taur/taur.env; set +a; exec /usr/bin/taur serve' >/tmp/taur-serve.log 2>&1 &
  echo $! >/tmp/taur-serve.pid
fi
ok=0
i=0
while [ "$i" -lt 40 ]; do
  if curl -fsS http://127.0.0.1:8080/api/v1/server | grep -q "\"version\":\"$ver\""; then
    ok=1
    break
  fi
  i=$((i + 1))
  sleep 0.5
done
if [ "$ok" != 1 ]; then
  echo "---- serve log ----" >&2
  cat /tmp/taur-serve.log >&2 || true
  fail "API did not answer ($how)"
fi
if [ "$how" = systemd ]; then
  systemctl disable --now taur >/dev/null 2>&1 || true
else
  kill "$(cat /tmp/taur-serve.pid)" 2>/dev/null || true
fi
echo "SERVE $how"
echo kept >/var/lib/taur/kept
EOF
chmod 0755 "$guest"

run_image() {
  local name="$1" image="$2" kind="$3" install="$4" again="$5"
  local log="/tmp/taur-smoke-${name}.log"
  if [[ -n "${TAUR_SMOKE_ONLY:-}" && "$TAUR_SMOKE_ONLY" != *"$name"* ]]; then
    return 0
  fi
  echo "smoke: $name"
  podman rm -f "taur-smoke-$name" >/dev/null 2>&1 || true
  if podman run --rm --name "taur-smoke-$name" \
    -e "KIND=$kind" \
    -v "$dist:/pkg:ro" \
    -v "$guest:/check.sh:ro" \
    "$image" \
    sh -c "$install && . /check.sh && $again" >"$log" 2>&1; then
    echo "PASS $name $(tr '\n' ' ' <"$log" | sed 's/  */ /g')"
  else
    echo "FAIL $name (log $log)"
    tail -50 "$log" || true
    return 1
  fi
}

failed=0
run_image debian-12 docker.io/library/debian:12 deb \
  'export DEBIAN_FRONTEND=noninteractive; apt-get update >/tmp/apt.log && apt-get install -y curl login /pkg/'"$deb"'; echo APT_EXIT:$?' \
  "apt-get install -y --reinstall /pkg/$deb >/tmp/reinstall.log && grep -q '# taur-smoke-marker' /etc/taur/taur.env && test -f /var/lib/taur/kept && apt-get remove -y taur >/tmp/remove.log && test -f /var/lib/taur/kept && test -f /etc/taur/taur.env && apt-get purge -y taur >/tmp/purge.log && test ! -f /etc/taur/taur.env && test -f /var/lib/taur/kept && echo SMOKE_OK" \
  || failed=1
run_image ubuntu-22.04 docker.io/library/ubuntu:22.04 deb \
  'export DEBIAN_FRONTEND=noninteractive; apt-get update >/tmp/apt.log && apt-get install -y curl login /pkg/'"$deb"'; echo APT_EXIT:$?' \
  "apt-get install -y --reinstall /pkg/$deb >/tmp/reinstall.log && grep -q '# taur-smoke-marker' /etc/taur/taur.env && test -f /var/lib/taur/kept && apt-get remove -y taur >/tmp/remove.log && test -f /var/lib/taur/kept && test -f /etc/taur/taur.env && apt-get purge -y taur >/tmp/purge.log && test ! -f /etc/taur/taur.env && test -f /var/lib/taur/kept && echo SMOKE_OK" \
  || failed=1
run_image ubuntu-24.04 docker.io/library/ubuntu:24.04 deb \
  'export DEBIAN_FRONTEND=noninteractive; apt-get update >/tmp/apt.log && apt-get install -y curl login /pkg/'"$deb"'; echo APT_EXIT:$?' \
  "apt-get install -y --reinstall /pkg/$deb >/tmp/reinstall.log && grep -q '# taur-smoke-marker' /etc/taur/taur.env && test -f /var/lib/taur/kept && apt-get remove -y taur >/tmp/remove.log && test -f /var/lib/taur/kept && test -f /etc/taur/taur.env && apt-get purge -y taur >/tmp/purge.log && test ! -f /etc/taur/taur.env && test -f /var/lib/taur/kept && echo SMOKE_OK" \
  || failed=1
run_image fedora docker.io/library/fedora:latest rpm \
  "if ! command -v curl >/dev/null; then dnf install -y curl; fi; dnf install -y /pkg/$rpm" \
  "rpm -Uvh --nosignature --replacepkgs /pkg/$rpm >/tmp/reinstall.log && grep -q '# taur-smoke-marker' /etc/taur/taur.env && test -f /var/lib/taur/kept && dnf remove -y taur >/tmp/remove.log && test -f /var/lib/taur/kept && echo SMOKE_OK" \
  || failed=1
run_image rockylinux-9 docker.io/library/rockylinux:9 rpm \
  "if ! command -v curl >/dev/null; then dnf install -y curl; fi; dnf install -y /pkg/$rpm" \
  "rpm -Uvh --nosignature --replacepkgs /pkg/$rpm >/tmp/reinstall.log && grep -q '# taur-smoke-marker' /etc/taur/taur.env && test -f /var/lib/taur/kept && dnf remove -y taur >/tmp/remove.log && test -f /var/lib/taur/kept && echo SMOKE_OK" \
  || failed=1
run_image almalinux-9 docker.io/library/almalinux:9 rpm \
  "if ! command -v curl >/dev/null; then dnf install -y curl; fi; dnf install -y /pkg/$rpm" \
  "rpm -Uvh --nosignature --replacepkgs /pkg/$rpm >/tmp/reinstall.log && grep -q '# taur-smoke-marker' /etc/taur/taur.env && test -f /var/lib/taur/kept && dnf remove -y taur >/tmp/remove.log && test -f /var/lib/taur/kept && echo SMOKE_OK" \
  || failed=1
run_image arch docker.io/library/archlinux:latest arch \
  "pacman -S --needed --noconfirm curl && pacman -U --noconfirm /pkg/$pkg" \
  "pacman -U --noconfirm /pkg/$pkg >/tmp/reinstall.log && grep -q '# taur-smoke-marker' /etc/taur/taur.env && test -f /var/lib/taur/kept && pacman -R --noconfirm taur >/tmp/remove.log && test -f /var/lib/taur/kept && echo SMOKE_OK" \
  || failed=1

rm -f "$guest"
exit "$failed"
