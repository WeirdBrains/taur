# Install a TAUR server

x86_64 packages for TAUR 1.3.2. This release fixes a startup panic with `TAUR_TLS=acme`: rustls is linked with both the ring and aws-lc-rs providers, and the process now selects ring before any ACME or TLS client is built. The package tests point that client at `https://127.0.0.1:9/directory` and require a connection error, not a `CryptoProvider` panic. No request is sent to Let's Encrypt.

The binaries are built for glibc 2.31. `objdump -T` on the packaged `taur` and `taurctl` shows no GLIBC symbol newer than **2.30**, and both binaries printed `1.3.2` on Ubuntu 20.04 (glibc 2.31). They do not link OpenSSL.

`apt install` / `dnf install` / `pacman -U` of these packages was run in containers for:

| System | What was run |
|---|---|
| Ubuntu 20.04 (glibc 2.31) | `taur --version` and `taurctl --version` only |
| Ubuntu 22.04 (glibc 2.35) and 24.04 (glibc 2.39) | `apt install` of the `.deb` |
| Debian 12 (glibc 2.36) | `apt install` of the same `.deb` |
| Fedora 45, Rocky Linux 9, AlmaLinux 9 | `dnf install` of the `.rpm` |
| Arch | `pacman -U` of the `.pkg.tar.zst` |

The `.deb` was not installed on Ubuntu 20.04. Only the two binaries were executed there.

## What these containers did not do

The containers were not booted with systemd as pid 1. After install, `systemctl is-enabled taur` printed `disabled` on Debian 12. `systemctl is-active taur` printed `System has not been booted with systemd as init system`. `systemctl enable --now`, `systemctl restart`, `systemctl edit`, and `journalctl` were not run.

`ufw status` on Debian 12 failed with `iptables ... Permission denied`. `firewall-cmd --state` on Fedora 45 failed with `Failed to connect to socket /run/dbus/system_bus_socket`. nftables and cloud-provider firewalls were not touched. The firewall commands below are the usual ones for those tools and are **untested**.

No ACME account was created and no reverse proxy was installed. SELinux in the Fedora 45 container was `Disabled` (`sestatus`); an enforcing policy was not tested. The package does not install an SELinux module.

## Files the packages install

Checked after install:

- `/usr/bin/taur` and `/usr/bin/taurctl`
- `/etc/taur/` mode `0750`, owner `root:taur`
- `/etc/taur/taur.env` mode `0640`, owner `root:taur`, copied from `packaging/taur.env.example`
- `/var/lib/taur/` mode `0750`, owner `taur:taur`
- user `taur`, shell `/usr/sbin/nologin` on Debian (Fedora's `util-linux` makes `/usr/sbin/nologin` the same file as `/usr/bin/nologin`)
- systemd unit `/usr/lib/systemd/system/taur.service`

The unit sets `NoNewPrivileges=yes` and an empty `CapabilityBoundingSet=`. The lines that would add `CAP_NET_BIND_SERVICE` are comments in the packaged unit. Do not edit that file; a drop-in is below.

The service is not enabled or started by the package. The package prints `taur: installed and not started. Enable it with: systemctl enable --now taur`.

Data checked under `/var/lib/taur`: `taur.db` and `outbox/`. With `TAUR_SMTP_URL` unset, the server logged `no SMTP configured; writing mail to /var/lib/taur/outbox` and a registration wrote a `.eml` file there. Delivery to a real SMTP server was not tested.

## Download and check

The 1.3.2 assets are on the [v1.3.2 release](https://github.com/WeirdBrains/taur/releases/tag/v1.3.2). `sha256sum -c SHA256SUMS` was run in the directory that contains all four files, and each single-file check below was run the same way. Downloading them from GitHub is checked again when that release is published.

```sh
cd /tmp
curl -fLO https://github.com/WeirdBrains/taur/releases/download/v1.3.2/SHA256SUMS
```

Ubuntu or Debian, then Fedora, then Arch. Download only the package you will install:

```sh
curl -fLO https://github.com/WeirdBrains/taur/releases/download/v1.3.2/taur_1.3.2-1_amd64.deb
grep 'taur_1.3.2-1_amd64.deb' SHA256SUMS | sha256sum -c -
```

```sh
curl -fLO https://github.com/WeirdBrains/taur/releases/download/v1.3.2/taur-1.3.2-1.x86_64.rpm
grep 'taur-1.3.2-1.x86_64.rpm' SHA256SUMS | sha256sum -c -
```

```sh
curl -fLO https://github.com/WeirdBrains/taur/releases/download/v1.3.2/taur-1.3.2-1-x86_64.pkg.tar.zst
grep 'taur-1.3.2-1-x86_64.pkg.tar.zst' SHA256SUMS | sha256sum -c -
```

`SHA256SUMS` also lists `taur-1.3.2-x86_64-unknown-linux-gnu.tar.gz` (the same binaries plus docs, man pages, and `packaging/`). A line must print `OK`.

## Ubuntu

Tested with `apt-get install -y ./taur_1.3.2-1_amd64.deb` on Ubuntu 22.04, Ubuntu 24.04, and Debian 12. `apt` pulls `git` and `adduser`. The Ubuntu container images ship a dpkg rule that drops `/usr/share/man` and `/usr/share/doc`; Debian 12 installed `/usr/share/man/man1/taur.1.gz` from the same `.deb`. A normal Ubuntu system without that rule unpacks them too. That normal-system case was not booted.

```sh
sudo apt update
sudo apt install ./taur_1.3.2-1_amd64.deb
taur --version
taurctl --version
systemctl is-enabled taur
```

`taur --version` and `taurctl --version` printed `1.3.2` on these installs. On Debian 12, `systemctl is-enabled taur` printed `disabled`. The Ubuntu runs asserted that the unit was not `enabled`; the exact word was not recorded there.

### Listen address

The shipped `/etc/taur/taur.env` has `TAUR_TLS=off` and `TAUR_BIND=127.0.0.1:8080`. The API check used that:

```sh
curl -fsS http://127.0.0.1:8080/api/v1/server
```

The body was `{"version":"1.3.2",...}`.

**Reverse proxy.** Leave `TAUR_TLS=off` and `TAUR_BIND=127.0.0.1:8080`, and point the proxy at that address. No proxy was installed in these tests. Details of what `TAUR_TLS=off` advertises are in [tls.md](tls.md).

**ACME on ports 80 and 443.** Not requested in these tests. The process would need a DNS name, ports 80 and 443, and the variables in [tls.md](tls.md). Example, edited in `/etc/taur/taur.env` (mode stays `0640`):

```sh
TAUR_TLS=acme
TAUR_BIND=0.0.0.0:443
TAUR_DOMAIN=packages.example.com
TAUR_BASE_URL=https://packages.example.com
TAUR_ACME_HTTP_BIND=0.0.0.0:80
TAUR_ACME_CONTACT=mailto:admin@example.com
TAUR_ACME_ACCEPT_TERMS=yes
```

`TAUR_ACME_ACCEPT_TERMS` must be the word `yes`. Read the CA agreement first. Nothing here requested a certificate.

Port 80 and 443 need `CAP_NET_BIND_SERVICE`. As user `taur`, with the shipped unit's empty capability set, `TAUR_BIND=127.0.0.1:80` logged `Permission denied (os error 13)`. The same process with `CAP_NET_BIND_SERVICE` accepted `curl` on port 80. The packaged unit also sets `NoNewPrivileges=yes`. On Debian 12 this drop-in was written and `systemd-analyze verify taur.service` succeeded. `systemctl edit` was not run (no systemd bus, and it expects a terminal).

```sh
sudo systemctl edit taur
```

Put this in the editor. It is the file that was verified, `/etc/systemd/system/taur.service.d/low-ports.conf`:

```ini
[Service]
NoNewPrivileges=no
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
```

`systemctl daemon-reload` was not run.

### Firewall

**Untested.** In the Debian container `ufw` could not read the iptables rules (`Permission denied`).

```sh
sudo apt install ufw
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw allow 2222/tcp
sudo ufw status
```

Port 2222 is `TAUR_SSH_BIND` in the shipped env file. Comment that line out if you do not want the built-in SSH server. A cloud security group was not configured.

### Start, admin, mail

`systemctl enable --now taur` and `journalctl -u taur` were not run. What was run is `taur serve` as user `taur` after `set -a; . /etc/taur/taur.env; set +a`.

`TAUR_ADMINS` grants site admin only to an account that already exists. On Debian 12, with `TAUR_ADMINS=alice` and no such user, the log said `TAUR_ADMINS: user alice does not exist yet; restart after they register`. This registration was then accepted:

```sh
curl -fsS -H 'content-type: application/json' \
  -d '{"name":"alice","email":"alice@example.com","password":"choose-a-long-password"}' \
  http://127.0.0.1:8080/api/v1/register
```

The response had `"is_admin":false`. After the process was started again with the same env file, `users.is_admin` for `alice` was `1` and the warning was gone.

`TAUR_SMTP_URL` is commented out in the shipped file. Leave it that way and mail stays in `/var/lib/taur/outbox`. A real SMTP server was not used. `TAUR_MAIL_FROM` is already set; values with spaces must stay double-quoted. If you set `TAUR_SMTP_URL`, percent-encode `/ ? # % : @` in the username and password. An unencoded `/`, `?`, or `#` cuts the userinfo short and the process dials the wrong host. Startup names that parsed host and refuses it when it is not a hostname or an IP address. The password is not included in the message.

### Upgrade and removal

`apt-get install --reinstall ./taur_1.3.2-1_amd64.deb` kept a line that had been appended to `/etc/taur/taur.env`. A newer version than 1.3.2 was not installed. For a later package the same install command is the one to use, then start the process again. `systemctl restart taur` was not run.

`apt-get remove taur` kept `/etc/taur/taur.env` and `/var/lib/taur`. `apt-get purge taur` deleted `/etc/taur/taur.env` and left `/var/lib/taur` and the `taur` user. Checked on Debian 12 and on Ubuntu 22.04 and 24.04.

```sh
sudo apt install ./taur_1.3.2-1_amd64.deb
sudo apt remove taur
sudo apt purge taur
```

## Fedora

Tested with `dnf install -y ./taur-1.3.2-1.x86_64.rpm` on Fedora 45, Rocky Linux 9, and AlmaLinux 9. The package requires `git`, `systemd`, `shadow-utils`, and `util-linux` (that last one provides `nologin`; without it the Fedora 45 install stopped in `%pre` with `nologin shell is missing`).

```sh
sudo dnf install ./taur-1.3.2-1.x86_64.rpm
taur --version
taurctl --version
systemctl is-enabled taur
```

Rocky Linux 9 and AlmaLinux 9 installed `/usr/share/man/man1/taur.1.gz`. The Fedora 45 container image did not unpack documentation files; `rpm -ql taur` still listed that man page.

The listen address, ACME variables, drop-in, admin, and mail steps are the same as Ubuntu. The API check, the port 80 capability check, `TAUR_ADMINS`, and the outbox check were done on Debian with this package's binaries and the same env file, not a second time on Fedora.

### SELinux

On Fedora 45, before and after installing `policycoreutils`, there was no `/sys/fs/selinux` and `sestatus` reported `SELinux status: disabled`. An enforcing system was not tested. The rpm does not install a policy module. If a later enforcing host denies the service, that has not been reproduced here.

### Firewall

**Untested.** `firewall-cmd --state` failed because the container had no system bus.

```sh
sudo firewall-cmd --permanent --add-service=http
sudo firewall-cmd --permanent --add-service=https
sudo firewall-cmd --permanent --add-port=2222/tcp
sudo firewall-cmd --reload
```

A cloud security group was not configured. Rocky and Alma use firewalld as well; those commands were not run there either.

### Upgrade and removal

`rpm -Uvh --nosignature --replacepkgs ./taur-1.3.2-1.x86_64.rpm` on Fedora 45 kept an edited `/etc/taur/taur.env`. A newer version was not installed. `systemctl restart` was not run.

`rpm -e taur` on Fedora 45 printed ` /etc/taur/taur.env saved as /etc/taur/taur.env.rpmsave`, removed `/etc/taur/taur.env`, and left `/var/lib/taur`. The `.rpmsave` file was still mode `0640` and owner `root:taur`. Rocky and Alma `dnf remove` left `/var/lib/taur`; their config-file rename was not listed.

```sh
sudo dnf install ./taur-1.3.2-1.x86_64.rpm
sudo dnf remove taur
```

## Arch

Tested with `pacman -Sy` and `pacman -U --noconfirm ./taur-1.3.2-1-x86_64.pkg.tar.zst`. The package depends on `git` and `systemd`. The Arch container image's `pacman.conf` has `NoExtract` for `/usr/share/man` and `/usr/share/doc`, so those files were not unpacked there. The package archive itself contains `usr/share/man/man1/taur.1.gz`. A normal Arch system without that `NoExtract` was not used.

```sh
sudo pacman -Sy
sudo pacman -U ./taur-1.3.2-1-x86_64.pkg.tar.zst
taur --version
taurctl --version
```

The install script printed `taur: installed and not started. Enable it with: systemctl enable --now taur`. `pacman -Qii taur` lists `etc/taur/taur.env` as a backup file. The listen address, ACME variables, drop-in, admin, and mail steps are the same as Ubuntu; those checks ran on Debian.

### Firewall

**Untested.** Arch does not enable firewalld in that image. No nftables rules were loaded. If you use nftables, open TCP 80, 443, and 2222 yourself. A cloud security group was not configured.

### Upgrade and removal

`pacman -U` of the same package kept an edited `/etc/taur/taur.env`. A newer version was not installed.

`pacman -R taur` printed `/etc/taur/taur.env saved as /etc/taur/taur.env.pacsave`, removed `/etc/taur/taur.env`, and left `/var/lib/taur`. The `.pacsave` file was mode `0640`, owner `root:taur`.

```sh
sudo pacman -U ./taur-1.3.2-1-x86_64.pkg.tar.zst
sudo pacman -R taur
```

## Start the service

This command was not executed. `systemctl is-enabled taur` was `disabled`, and the unit file contains `WantedBy=multi-user.target` under `[Install]`.

```sh
sudo systemctl enable --now taur
sudo systemctl status taur
sudo journalctl -u taur -e
curl -fsS http://127.0.0.1:8080/api/v1/server
```

`journalctl` and `systemctl status` were not run. The `curl` check was run against `taur serve` started by hand as user `taur`.
