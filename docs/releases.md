# Making a release

`scripts/release.sh` bumps the workspace version, checks the tree, commits, tags, and pushes. It does not build the public archive and it does not create a GitHub Release. Those assets are a separate step after the tag is on `main`.

Run it from a clean `main` that matches `origin/main`. The default bump is the next minor, and patch goes back to 0.

```sh
scripts/release.sh --dry-run
scripts/release.sh
```

`1.2.0` becomes `1.3.0`. The commit message is `release: v1.3.0`. The annotated tag is `v1.3.0`. The push is one atomic update of `main` and that tag:

```sh
git push --atomic origin HEAD:main refs/tags/v1.3.0
```

Other local tags are not pushed. An existing `v1.3.0` tag, locally or on `origin`, stops the script. The push is not forced.

## Which number changes

```sh
scripts/release.sh --patch          # 1.2.0 -> 1.2.1, tag v1.2.1
scripts/release.sh --major          # 1.2.0 -> 2.0.0, tag v2.0.0
scripts/release.sh --version 1.4.0  # explicit stable core, tag v1.4.0
```

`--major`, `--patch`, and `--version` cannot be combined. `--major` sets minor and patch to 0. A minor bump sets patch to 0. Each component is `0` or a number without a leading zero. `1.2`, `01.2.0`, `1.2.3-rc.1`, `1.2.3+ghi`, and `v1.2.3` are rejected. The new version must be greater than the version in `[workspace.package]`.

`--dry-run` prints the plan and does not edit files, commit, tag, or push. It still requires a clean `main` that matches `origin/main`.

## What the script changes

- `Cargo.toml` workspace version, which the `taur` and `taurctl` binaries report
- `Cargo.lock` versions for `taur-cli`, `taur-core`, and `taur-server`
- `CHANGELOG.md`, by dating `## [Unreleased]` as `## [X.Y.Z] - YYYY-MM-DD` when that heading is not already there
- man pages, regenerated from the CLI so they show the same version

Before the commit it runs `cargo fmt --all -- --check`, `cargo clippy --all-targets --offline -- -D warnings`, `cargo test --offline`, and `cargo build --release --offline`. A failed check restores the checkout to the commit you started from. A failed push leaves that release commit and its tag only on this machine; `origin` is unchanged and the tag is not overwritten.

## GitHub release assets

The script stops at the git tag. Publishing binaries is separate and is not done for every tag. The v1.1.0 archive is the pattern: one `taur-<version>-x86_64-unknown-linux-gnu.tar.gz` built on that host, plus `SHA256SUMS` of the archive. The tarball contains `taur`, `taurctl`, `README.md`, `CHANGELOG.md`, `docs/`, `man/`, `packaging/`, and an internal `SHA256SUMS` of those files. It does not contain a `LICENSE` file; the crate license is MIT OR Apache-2.0. No other architecture is added unless that build was actually produced and tested.

## Distribution packages

`scripts/package.sh VERSION` builds the public tarball and three x86_64 packages from a tag or commit that already exists. It does not bump the version, move a tag, or create a GitHub release.

```sh
scripts/package.sh 1.3.1
```

`1.3.1` and `v1.3.1` both mean that tag. A commit id is also accepted, and the package version is the workspace version in that commit. The binaries are compiled from that commit in a temporary worktree. Debian, RPM, and Arch recipes come from the checkout you run the script in, so packaging can be newer than the tag.

The compile uses `cargo-zigbuild` 0.24.1 and Zig 0.14.1 with target `x86_64-unknown-linux-gnu.2.31`. The `.deb` is packed with Debian 12's `dpkg-deb` and the `.rpm` with Fedora 45's `rpmbuild`, through podman when those tools are not on the host. The script refuses a binary that needs a glibc newer than 2.31 or that links OpenSSL. `SOURCE_DATE_EPOCH` is the commit time. Zig is downloaded into the cache directory and checked against a fixed SHA-256. Set `TAUR_ZIG` to use a copy you already have.

Artifacts land in `dist/` (or `--output`):

- `taur-VERSION-x86_64-unknown-linux-gnu.tar.gz`
- `taur_VERSION-1_amd64.deb`
- `taur-VERSION-1.x86_64.rpm`
- `taur-VERSION-1-x86_64.pkg.tar.zst`
- `SHA256SUMS` of those four files

The packages install `/usr/bin/taur`, `/usr/bin/taurctl`, man pages, docs, the systemd unit, and `/etc/taur/taur.env`. They depend on git. They do not enable or start the service. Install steps are in [services.md](services.md).

`scripts/package-smoke.sh dist` installs the three packages in Debian 12, Ubuntu 22.04, Ubuntu 24.04, Fedora, Rocky Linux 9, AlmaLinux 9, and Arch containers. It checks the account, the config file, a running server, upgrade, and removal.
