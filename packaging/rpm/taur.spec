Name:           taur
Version:        @VERSION@
Release:        1
Summary:        AUR-compatible package server
License:        MIT OR Apache-2.0
URL:            https://github.com/WeirdBrains/taur
ExclusiveArch:  x86_64
Requires:       git
Requires:       systemd
Requires:       shadow-utils
Requires:       util-linux

# The payload is a prebuilt root. Do not strip or generate a debug package.
%define debug_package %{nil}
%define _enable_debug_package 0
%define __os_install_post %{nil}
%define _binary_payload w9.gzdio
%define _buildhost reproducible

%description
TAUR serves an AUR-compatible package index over HTTP. The package installs
the server and taurctl, man pages, docs, and a systemd unit. It does not
enable or start the service. Enable it with: systemctl enable --now taur
Removing the package keeps /var/lib/taur.

%install
rm -rf %{buildroot}
mkdir -p %{buildroot}
cp -a %{taur_stage}/. %{buildroot}/

%pre
nologin=/usr/sbin/nologin
[ -x "$nologin" ] || nologin=/usr/bin/nologin
[ -x "$nologin" ] || { echo "taur: nologin shell is missing" >&2; exit 1; }
getent group taur >/dev/null || groupadd --system taur
getent passwd taur >/dev/null || \
  useradd --system --gid taur --home-dir /var/lib/taur --no-create-home \
    --shell "$nologin" --comment "TAUR package server" taur

%post
/usr/lib/taur/ensure-account posix
if [ "$1" -eq 1 ]; then
  echo "taur: installed and not started. Enable it with: systemctl enable --now taur" >&2
fi
if [ "$1" -ge 2 ]; then
  systemctl try-restart taur.service >/dev/null 2>&1 || true
fi

%preun
if [ "$1" -eq 0 ]; then
  systemctl disable --now taur.service >/dev/null 2>&1 || true
fi

%postun
if [ "$1" -eq 0 ]; then
  systemctl daemon-reload >/dev/null 2>&1 || true
fi

%files
%defattr(-,root,root,-)
%dir %attr(0750,root,taur) /etc/taur
%config(noreplace) %attr(0640,root,taur) /etc/taur/taur.env
/usr/bin/taur
/usr/bin/taurctl
%dir /usr/lib/taur
/usr/lib/taur/ensure-account
/usr/lib/systemd/system/taur.service
/usr/lib/sysusers.d/taur.conf
/usr/lib/tmpfiles.d/taur.conf
/usr/share/doc/taur
/usr/share/licenses/taur
/usr/share/man/man1

%changelog
* @RPM_DATE@ WeirdBrains <taur@weirdbrains.com> - @VERSION@-1
- Package TAUR @VERSION@.
