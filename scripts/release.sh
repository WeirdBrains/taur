#!/usr/bin/env bash
# Bump the TAUR workspace version, commit it, tag vMAJOR.MINOR.PATCH, and push
# that commit and that tag. Does not upload GitHub release assets.
#
#   scripts/release.sh                  # minor: 1.2.0 -> 1.3.0, tag v1.3.0
#   scripts/release.sh --patch          # 1.2.0 -> 1.2.1
#   scripts/release.sh --major          # 1.2.0 -> 2.0.0
#   scripts/release.sh --version 1.4.0  # explicit stable core, must increase
#   scripts/release.sh --dry-run        # print the plan; change nothing
#
# Run from a clean checkout of main that matches origin/main.
# See docs/releases.md.
set -euo pipefail

die() { echo "release: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: scripts/release.sh [--dry-run] [--major | --patch | --version MAJOR.MINOR.PATCH]

Default bump is minor (patch resets to 0). --major resets minor and patch to 0.
--patch increments patch. --version sets an explicit stable SemVer core and
must be greater than the current version. Those three modes are mutually
exclusive. Tags are vMAJOR.MINOR.PATCH with no leading zeros, prerelease, or
build metadata.

The script updates the workspace version, lockfile, changelog heading, and man
pages, runs fmt, clippy, tests, and a release build, then commits, creates an
annotated tag, and pushes main and that tag together. It does not push other
tags and it does not create a GitHub release.
EOF
}

mode=""
explicit=""
dry=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --major|--patch)
      [[ -n "$mode" ]] && die "--major, --patch, and --version are mutually exclusive"
      mode="${1#--}"
      ;;
    --version)
      [[ -n "$mode" ]] && die "--major, --patch, and --version are mutually exclusive"
      [[ $# -ge 2 ]] || die "--version needs MAJOR.MINOR.PATCH (no v prefix, prerelease, or build)"
      mode="explicit"
      explicit="$2"
      shift
      ;;
    --dry-run) dry=1 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
  shift
done
[[ -n "$mode" ]] || mode="minor"

root="$(git rev-parse --show-toplevel 2>/dev/null)" || die "run this inside the TAUR git checkout"
cd "$root"

[[ -f Cargo.toml && -f Cargo.lock && -f CHANGELOG.md ]] || die "Cargo.toml, Cargo.lock, and CHANGELOG.md are required"
branch="$(git branch --show-current)"
[[ "$branch" == "main" ]] || die "refusing to release from branch '${branch:-detached}'; checkout main"
[[ -z "$(git status --porcelain)" ]] || die "working tree is not clean; commit or stash first"
git diff --cached --quiet || die "staged changes are present; the release commit must contain only the version bump"
git rev-parse --verify --quiet refs/remotes/origin/main >/dev/null || die "origin/main is not available; fetch origin first"
git fetch --quiet origin main
local_head="$(git rev-parse HEAD)"
remote_head="$(git rev-parse refs/remotes/origin/main)"
[[ "$local_head" == "$remote_head" ]] || die "main is not up to date with origin/main ($local_head != $remote_head)"

current="$(
  python3 - <<'PY'
import pathlib, sys
text = pathlib.Path("Cargo.toml").read_text()
section = None
for line in text.splitlines():
    stripped = line.strip()
    if stripped.startswith("[") and stripped.endswith("]"):
        section = stripped
        continue
    if section == "[workspace.package]" and stripped.startswith("version"):
        value = stripped.split("=", 1)[1].strip().strip('"').strip("'")
        print(value)
        sys.exit(0)
sys.exit("workspace.package version not found")
PY
)" || die "could not read [workspace.package] version"

valid_core() {
  local v="$1" part
  [[ "$v" != *-* && "$v" != *+* && "$v" != v* ]] || return 1
  [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || return 1
  IFS='.' read -r -a parts <<<"$v"
  [[ "${#parts[@]}" -eq 3 ]] || return 1
  for part in "${parts[@]}"; do
    [[ "$part" =~ ^(0|[1-9][0-9]*)$ ]] || return 1
  done
}

valid_core "$current" || die "current version '$current' is not a stable MAJOR.MINOR.PATCH core"

reject_explicit() {
  local v="$1"
  if [[ "$v" == *-* || "$v" == *+* ]]; then
    die "rejecting '$v': prerelease and build metadata are not used; the tag is vMAJOR.MINOR.PATCH"
  fi
  if [[ "$v" == v* ]]; then
    die "rejecting '$v': pass MAJOR.MINOR.PATCH without a v prefix"
  fi
  if [[ ! "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    die "rejecting '$v': version must be MAJOR.MINOR.PATCH"
  fi
  local part
  local -a parts
  IFS='.' read -r -a parts <<<"$v"
  for part in "${parts[@]}"; do
    [[ "$part" =~ ^(0|[1-9][0-9]*)$ ]] || die "rejecting '$v': '$part' has a leading zero or is not a number"
  done
}

version_gt() {
  local IFS='.'
  local -a a b
  read -r -a a <<<"$1"
  read -r -a b <<<"$2"
  local i
  for i in 0 1 2; do
    if (( 10#${a[$i]} > 10#${b[$i]} )); then return 0; fi
    if (( 10#${a[$i]} < 10#${b[$i]} )); then return 1; fi
  done
  return 1
}

IFS='.' read -r cur_major cur_minor cur_patch <<<"$current"
case "$mode" in
  major) new="$((cur_major + 1)).0.0" ;;
  minor) new="$cur_major.$((cur_minor + 1)).0" ;;
  patch) new="$cur_major.$cur_minor.$((cur_patch + 1))" ;;
  explicit)
    reject_explicit "$explicit"
    new="$explicit"
    ;;
  *) die "internal mode '$mode'" ;;
esac
valid_core "$new" || die "refusing to produce version '$new'"
version_gt "$new" "$current" || die "version must increase ($current -> $new)"

tag="v$new"
[[ "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || die "tag '$tag' is not vMAJOR.MINOR.PATCH"
if git show-ref --verify --quiet "refs/tags/$tag"; then
  die "local tag $tag already exists and will not be overwritten"
fi
if [[ -n "$(git ls-remote origin "refs/tags/$tag")" ]]; then
  die "origin already has $tag; refusing to overwrite it"
fi

plan() {
  cat <<EOF
release plan
  current:  $current
  new:      $new
  tag:      $tag
  commit:   release: $tag
  push:     git push --atomic origin HEAD:main refs/tags/$tag
  files:    Cargo.toml, Cargo.lock, CHANGELOG.md, man pages when present
  checks:   cargo fmt, clippy -D warnings, cargo test, cargo build --release
  github:   this script does not create a GitHub release or upload binaries
EOF
}

if [[ "$dry" -eq 1 ]]; then
  plan
  echo "dry-run: no files, commit, tag, or push"
  exit 0
fi

start_rev="$(git rev-parse HEAD)"
edited=0
committed=0
on_exit() {
  local status=$?
  trap - EXIT
  if [[ "$edited" -eq 1 && "$committed" -eq 0 ]]; then
    git reset --hard "$start_rev" >/dev/null 2>&1 || true
    if [[ -d man ]]; then
      git clean -fd -- man >/dev/null 2>&1 || true
    fi
    echo "release: stopped before commit; restored $start_rev" >&2
  fi
  exit "$status"
}
trap on_exit EXIT

export NEW_VERSION="$new"
python3 - <<'PY'
import os, pathlib, sys
new = os.environ["NEW_VERSION"]
path = pathlib.Path("Cargo.toml")
lines = path.read_text().splitlines(keepends=True)
section = None
found = False
out = []
for line in lines:
    stripped = line.strip()
    if stripped.startswith("[") and stripped.endswith("]"):
        section = stripped
    if section == "[workspace.package]" and not found and stripped.startswith("version"):
        nl = "\n" if line.endswith("\n") else ""
        out.append(f'version = "{new}"{nl}')
        found = True
        continue
    out.append(line)
if not found:
    sys.exit("workspace.package version line not found")
path.write_text("".join(out))

lock = pathlib.Path("Cargo.lock")
names = {"taur-cli", "taur-core", "taur-server"}
lines = lock.read_text().splitlines(keepends=True)
out = []
i = 0
seen = set()
while i < len(lines):
    line = lines[i]
    name = None
    if line.startswith("name = "):
        name = line.split("=", 1)[1].strip().strip('"')
    if name in names and i + 1 < len(lines) and lines[i + 1].startswith("version = "):
        out.append(line)
        nl = "\n" if lines[i + 1].endswith("\n") else ""
        out.append(f'version = "{new}"{nl}')
        seen.add(name)
        i += 2
        continue
    out.append(line)
    i += 1
missing = names - seen
if missing:
    sys.exit("Cargo.lock missing package version for " + ", ".join(sorted(missing)))
lock.write_text("".join(out))

changelog = pathlib.Path("CHANGELOG.md")
text = changelog.read_text()
heading = f"## [{new}]"
if heading not in text:
    old = "## [Unreleased]"
    if old not in text:
        sys.exit("CHANGELOG.md has neither [Unreleased] nor the new version heading")
    import datetime
    today = datetime.date.today().isoformat()
    text = text.replace(old, f"## [{new}] - {today}", 1)
    changelog.write_text(text)
PY
edited=1

if [[ -d man && -f crates/taur-cli/tests/cli.rs ]]; then
  TAUR_UPDATE_MAN=1 cargo test --offline -p taur-cli --test cli man_pages_up_to_date -- --exact
  TAUR_UPDATE_MAN=1 cargo test --offline -p taur-server --test http taur_man_pages_up_to_date -- --exact
fi

cargo fmt --all -- --check
cargo clippy --all-targets --offline -- -D warnings
cargo test --offline
cargo build --release --offline

git add -- Cargo.toml Cargo.lock CHANGELOG.md
if [[ -d man ]]; then
  git add -- man
fi
if ! git diff --cached --quiet; then
  git commit -m "release: $tag"
  committed=1
else
  die "version bump produced no changes to commit"
fi
git tag -a "$tag" -m "$tag"
git push --atomic origin HEAD:main "refs/tags/$tag"
trap - EXIT
plan
echo "pushed $tag"
