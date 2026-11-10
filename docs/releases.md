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
- `packaging/arch/PKGBUILD` `pkgver`, set to the same workspace version (`pkgrel` stays 1)
- `packaging/arch/.SRCINFO`, regenerated with `makepkg --printsrcinfo` (or an Arch container if `makepkg` is not on PATH)
- man pages, regenerated from the CLI so they show the same version

Debian `control.in` and the RPM spec keep `@VERSION@` placeholders, which `scripts/package.sh` fills in. The release script checks those markers and that the Arch `pkgver` already matches the current workspace version; a missing file or mismatch stops the release. `.SRCINFO` is source-recipe metadata: it is included under `packaging/arch/` in the tree and in the release tarball, and it is not installed as a runtime file.

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

## GitHub Actions on alien-blue

CI and release builds run only on a self-hosted runner on this machine. The workflows do not use a GitHub-hosted runner.

The runner is `alien-blue`, installed for `m3talsmith` (not root) at `~/actions-runner-taur`, version 2.338.2. Its labels are `self-hosted`, `Linux`, `X64`, `alien-blue`, and `taur`. Both workflow files require all of those labels. A user systemd service keeps it running while this user is logged in:

```sh
systemctl --user status actions-runner-taur.service
systemctl --user restart actions-runner-taur.service
```

The unit is `~/.config/systemd/user/actions-runner-taur.service` and it is enabled. It does not start at boot until lingering is turned on, which needs a password on this machine:

```sh
sudo loginctl enable-linger m3talsmith
```

That command has not been run. `Linger=no` until it is.

`scripts/svc.sh` from the runner install wants root to write `/etc/systemd/system`. The user unit above is the service that is actually running. It executes `~/actions-runner-taur/bin/runsvc.sh`.

### What is allowed to start a job

`.github/workflows/ci.yml` runs on `workflow_dispatch` only. `.github/workflows/release.yml` runs on a push of a `v*` tag and on `workflow_dispatch`. There is no `pull_request` or `pull_request_target` trigger, so a fork pull request cannot run code here. A push to `main` does not start either workflow.

Each job sets `permissions:`. CI is `contents: read`. The release packaging job is `contents: read`. Only the upload job is `contents: write`, and that job is skipped when the manual `upload` input is false. A tag push uploads only when the release has no assets yet. A 404 from the releases API is treated as a missing release, not as existing assets. Replacing assets that are already there requires the manual `overwrite` input. The v1.3.2 assets are left in place by a dry run.

The repository Actions settings, set through the API:

- Fork pull requests from outside collaborators require approval (`approval_policy` `all_external_contributors`). It was previously `first_time_contributors`.
- The default workflow token is read-only and cannot approve pull requests. A job that needs more asks for it.

The organization Actions policy was not changed. The token used here is not an organization admin (`admin:org` is required).
