# Running TAUR as a service

All service configs live in `packaging/`. They run `taur serve` and take every setting
from the environment variables in [configuration.md](configuration.md), read from an
env file (start from `packaging/taur.env.example` or `.env.example`). The env file may
contain `TAUR_SMTP_URL` with a password, so keep it readable only by root and the
service account. Prerequisites everywhere: the `taur` binary and `git` on `PATH`
(git's `http-backend` serves clones and pushes). TLS goes in a reverse proxy in front
of `TAUR_BIND` (and `TAUR_GRPC_BIND` if public).

Values containing spaces or `<`/`>` (e.g. `TAUR_MAIL_FROM`) must be double-quoted so
that both systemd and `sh` can read the file.

## systemd (Linux)
Files: `packaging/systemd/taur.service`, `packaging/systemd/taur.sysusers`,
`packaging/systemd/taur.tmpfiles`. The unit runs as the `taur` user with
`TAUR_HOME=/var/lib/taur` (`StateDirectory=taur`), `EnvironmentFile=/etc/taur/taur.env`,
`Restart=on-failure` and sandboxing (`NoNewPrivileges`, `ProtectSystem=strict`,
`ProtectHome`, `PrivateTmp`, …).

```sh
sudo install -Dm755 target/release/taur /usr/bin/taur
sudo install -Dm644 packaging/systemd/taur.sysusers /usr/lib/sysusers.d/taur.conf
sudo install -Dm644 packaging/systemd/taur.tmpfiles /usr/lib/tmpfiles.d/taur.conf
sudo systemd-sysusers && sudo systemd-tmpfiles --create
sudo install -Dm640 -g taur packaging/taur.env.example /etc/taur/taur.env   # then edit
sudo install -Dm644 packaging/systemd/taur.service /etc/systemd/system/taur.service
sudo systemctl daemon-reload && sudo systemctl enable --now taur
journalctl -u taur -f
```
Uninstall: `sudo systemctl disable --now taur`, remove the unit, sysusers/tmpfiles
snippets and `/etc/taur`; data stays in `/var/lib/taur`. To bind a port below 1024,
uncomment the `CAP_NET_BIND_SERVICE` lines.

## SysV init (Debian-style `/etc/init.d`)
File: `packaging/sysvinit/taur` (LSB headers; `start|stop|restart|status`). It sources
`/etc/default/taur` (optional, shell syntax) and then `/etc/taur/taur.env`, runs as
`taur` via `start-stop-daemon` (or `su`), writes `/var/run/taur.pid` and logs to
`/var/log/taur.log`.

```sh
sudo useradd --system --home /var/lib/taur --shell /usr/sbin/nologin taur
sudo install -d -o taur -g taur -m750 /var/lib/taur
sudo install -Dm640 -g taur packaging/taur.env.example /etc/taur/taur.env
sudo install -m755 packaging/sysvinit/taur /etc/init.d/taur
sudo update-rc.d taur defaults && sudo service taur start
```
Uninstall: `sudo service taur stop && sudo update-rc.d -f taur remove && sudo rm /etc/init.d/taur`.

## OpenRC (Alpine, Gentoo, Artix)
File: `packaging/openrc/taur`. Loads `/etc/taur/taur.env`, runs as `taur:taur` in the
background with logs in `/var/log/taur.log`.

```sh
sudo install -m755 packaging/openrc/taur /etc/init.d/taur
sudo rc-update add taur default && sudo rc-service taur start
```
Uninstall: `sudo rc-service taur stop && sudo rc-update del taur && sudo rm /etc/init.d/taur`.

## runit (Void, Artix)
Files: `packaging/runit/taur/run`, `packaging/runit/taur/log/run` (svlogd into
`/var/log/taur`).

```sh
sudo cp -r packaging/runit/taur /etc/sv/taur
sudo ln -s /etc/sv/taur /var/service/      # Void; Artix: /run/runit/service
```
Uninstall: remove the symlink, then `/etc/sv/taur`.

## macOS (launchd)
Files: `packaging/launchd/com.weirdbrains.taur.plist` (a LaunchDaemon with `KeepAlive`,
logs in `/usr/local/var/log/taur/taur.log`) and `packaging/launchd/taur-launchd`, a
wrapper that sources `/usr/local/etc/taur/taur.env` before running `taur serve`
(launchd itself can't read env files). It runs as the `_taur` user with
`TAUR_HOME=/usr/local/var/taur`. git comes from the Xcode command line tools or Homebrew.

```sh
# create a hidden service user (pick a free UID/GID, e.g. 450)
sudo dscl . -create /Groups/_taur PrimaryGroupID 450
sudo dscl . -create /Users/_taur UniqueID 450
sudo dscl . -create /Users/_taur PrimaryGroupID 450
sudo dscl . -create /Users/_taur UserShell /usr/bin/false
sudo dscl . -create /Users/_taur NFSHomeDirectory /usr/local/var/taur
sudo install -m755 target/release/taur /usr/local/bin/taur
sudo install -m755 packaging/launchd/taur-launchd /usr/local/libexec/taur-launchd
sudo install -d -o _taur -g _taur /usr/local/var/taur /usr/local/var/log/taur
sudo install -d /usr/local/etc/taur
sudo install -m640 -g _taur packaging/taur.env.example /usr/local/etc/taur/taur.env
sudo install -m644 packaging/launchd/com.weirdbrains.taur.plist /Library/LaunchDaemons/
sudo launchctl bootstrap system /Library/LaunchDaemons/com.weirdbrains.taur.plist
```
Uninstall: `sudo launchctl bootout system/com.weirdbrains.taur`, then remove the plist,
wrapper and `/usr/local/etc/taur`.

## Windows
Files: `packaging/windows/taur-service.xml` (a [WinSW](https://github.com/winsw/winsw)
template), `packaging/windows/install-service.ps1` and
`packaging/windows/uninstall-service.ps1`. `taur.exe` doesn't speak the Windows
service-control protocol itself, so WinSW wraps it. The installer turns each `TAUR_*`
line of the env file into an `<env>` entry and installs the service with automatic start
and restart on failure. Logs go to `C:\ProgramData\TAUR\logs`, data to
`C:\ProgramData\TAUR\data`.

```powershell
# elevated PowerShell; taur.exe in C:\Program Files\TAUR, Git for Windows installed
.\packaging\windows\install-service.ps1 -WinSW C:\path\to\WinSW-x64.exe -EnvFile C:\ProgramData\TAUR\taur.env
.\packaging\windows\uninstall-service.ps1
```

**Windows caveats.** TAUR is compile-checked for Windows: `cargo check`, `cargo clippy
-D warnings` and `cargo build --release` pass for `x86_64-pc-windows-gnu`
(cross-compiled from Linux with mingw-w64). It has not been run or tested on Windows.
To cross-build: `rustup target add x86_64-pc-windows-gnu`, install mingw-w64-gcc, then
`cargo build --release --target x86_64-pc-windows-gnu` (using rustup's toolchain, not a
distro rustc without the Windows standard library).
- The code avoids Unix-only APIs: file permissions are only set on Unix, and the null
  device and child-process environment are chosen per platform. On Windows the outbox,
  SSH host key and taurctl token are protected only by the ACLs of their directories.
- Clones and pushes need Git for Windows. Its `git http-backend` serves them, and push
  hooks are `#!/bin/sh` scripts run by Git for Windows' bundled shell.
- The generated `taur-service.xml` contains the env values, including any SMTP
  password. The installer restricts it to Administrators and SYSTEM.
