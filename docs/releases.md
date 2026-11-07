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
