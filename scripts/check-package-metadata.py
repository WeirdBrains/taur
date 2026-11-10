#!/usr/bin/env python3
"""Check or update TAUR package-version metadata for scripts/release.sh.

The public Arch recipe is packaging/arch/PKGBUILD. Its source must be the
GitHub release tarball URL for the workspace version. sha256sums is either
the placeholder UPSTREAM_TARBALL_SHA256 or a 64-character lowercase hex
digest of that tarball. A payload.tar.gz source or sha256sums=('SKIP') is
rejected: that local payload belongs to packaging/arch-internal/PKGBUILD.

pkgrel stays 1, matching Debian Version: @VERSION@-1 and RPM Release: 1.
Debian and RPM templates keep @VERSION@ for scripts/package.sh to fill in.

set-pkgver updates pkgver and resets sha256sums to the placeholder so a
previous release hash is never left on a new version. set-tarball-sha256
writes a verified hash after the GitHub tarball exists.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys


PLACEHOLDER = "UPSTREAM_TARBALL_SHA256"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


def die(msg: str) -> None:
    print(f"release: {msg}", file=sys.stderr)
    sys.exit(1)


def words(value: str) -> list[str]:
    value = value.strip()
    if value.startswith("(") and value.endswith(")"):
        value = value[1:-1].strip()
    out: list[str] = []
    for m in re.finditer(r"'([^']*)'|\"([^\"]*)\"|(\S+)", value):
        out.append(next(g for g in m.groups() if g is not None))
    return out


def assign(text: str, key: str, path: pathlib.Path) -> str:
    vals = []
    for line in text.splitlines():
        if line.startswith(key + "="):
            vals.append(line.split("=", 1)[1])
    if len(vals) != 1:
        die(f"{path} must have exactly one {key}= line")
    return vals[0]


def srcinfo_values(text: str, key: str) -> list[str]:
    pat = re.compile(r"^\t?" + re.escape(key) + r" = (.*)$")
    return [m.group(1) for line in text.splitlines() if (m := pat.match(line))]


def srcinfo_list(text: str, key: str, path: pathlib.Path) -> list[str]:
    vals = srcinfo_values(text, key)
    if not vals:
        die(f"{path} is missing {key}")
    return vals


def srcinfo_one(text: str, key: str, path: pathlib.Path) -> str:
    vals = srcinfo_values(text, key)
    if len(vals) != 1:
        die(f"{path} must have exactly one {key} (found {len(vals)})")
    return vals[0]


def field(text: str, key: str, path: pathlib.Path) -> str:
    pat = re.compile(r"^" + re.escape(key) + r":\s*(.*)$")
    vals = [m.group(1).strip() for line in text.splitlines() if (m := pat.match(line))]
    if len(vals) != 1:
        die(f"{path} must have exactly one {key}: line")
    return vals[0]


def tarball_url(version: str) -> str:
    return (
        "https://github.com/WeirdBrains/taur/releases/download/"
        f"v{version}/taur-{version}-x86_64-unknown-linux-gnu.tar.gz"
    )


def tarball_url_template() -> str:
    return (
        "https://github.com/WeirdBrains/taur/releases/download/"
        "v${pkgver}/taur-${pkgver}-x86_64-unknown-linux-gnu.tar.gz"
    )


def recipe_dir(root: pathlib.Path, recipe: pathlib.Path | None) -> pathlib.Path:
    if recipe is not None:
        return recipe.resolve()
    return (root / "packaging/arch").resolve()


def replace_assign(path: pathlib.Path, key: str, value: str) -> None:
    if not path.is_file():
        die(f"missing {path}")
    lines = path.read_text().splitlines(keepends=True)
    out = []
    found = 0
    for line in lines:
        if line.startswith(key + "="):
            nl = "\n" if line.endswith("\n") else ""
            out.append(f"{key}={value}{nl}")
            found += 1
            continue
        out.append(line)
    if found != 1:
        die(f"{path} must have exactly one {key}= line")
    path.write_text("".join(out))


def check(root: pathlib.Path, version: str) -> None:
    pkgbuild_path = root / "packaging/arch/PKGBUILD"
    srcinfo_path = root / "packaging/arch/.SRCINFO"
    debian_path = root / "packaging/debian/control.in"
    rpm_path = root / "packaging/rpm/taur.spec"
    internal_path = root / "packaging/arch-internal/PKGBUILD"
    for path in (pkgbuild_path, srcinfo_path, debian_path, rpm_path, internal_path):
        if not path.is_file():
            die(f"missing {path}")

    pkgbuild = pkgbuild_path.read_text()
    srcinfo = srcinfo_path.read_text()
    pkgver = words(assign(pkgbuild, "pkgver", pkgbuild_path))
    if len(pkgver) != 1:
        die(f"{pkgbuild_path} pkgver must be a single value")
    pkgver = pkgver[0]
    if pkgver != version:
        die(
            f"{pkgbuild_path} pkgver is {pkgver}, expected {version} (workspace version)"
        )
    pkgrel = words(assign(pkgbuild, "pkgrel", pkgbuild_path))
    if pkgrel != ["1"]:
        die(f"{pkgbuild_path} pkgrel is {' '.join(pkgrel)}, expected 1")
    if srcinfo_one(srcinfo, "pkgver", srcinfo_path) != pkgver:
        die(f"{srcinfo_path} pkgver does not match {pkgbuild_path}")
    if srcinfo_one(srcinfo, "pkgrel", srcinfo_path) != "1":
        die(f"{srcinfo_path} pkgrel must be 1")

    source = words(assign(pkgbuild, "source", pkgbuild_path))
    expected = tarball_url(version)
    template = tarball_url_template()
    if source not in ([expected], [template]):
        die(
            f"{pkgbuild_path} source must be the GitHub release tarball "
            f"({template} or {expected}), not {source}"
        )
    if any("payload.tar.gz" in item for item in source):
        die(
            f"{pkgbuild_path} is the public recipe and must not use payload.tar.gz; "
            "that local payload belongs in packaging/arch-internal/PKGBUILD"
        )
    srcinfo_source = srcinfo_list(srcinfo, "source", srcinfo_path)
    if srcinfo_source != [expected]:
        die(f"{srcinfo_path} source is {srcinfo_source}, expected [{expected}]")

    sums = words(assign(pkgbuild, "sha256sums", pkgbuild_path))
    if len(sums) != 1:
        die(f"{pkgbuild_path} sha256sums must have exactly one value")
    digest = sums[0]
    if digest != PLACEHOLDER and not SHA256_RE.fullmatch(digest):
        die(
            f"{pkgbuild_path} sha256sums must be {PLACEHOLDER} or a 64-character "
            f"lowercase hex digest, not {digest!r}"
        )

    for key in ("arch", "depends", "source", "sha256sums"):
        pb = words(assign(pkgbuild, key, pkgbuild_path))
        si = srcinfo_list(srcinfo, key, srcinfo_path)
        if key == "source":
            # PKGBUILD may keep ${pkgver}; .SRCINFO always expands it.
            if si != [expected]:
                die(f"{srcinfo_path} source is {si}, expected [{expected}]")
            continue
        if pb != si:
            die(f"{srcinfo_path} {key} is {si}, but {pkgbuild_path} has {pb}")

    internal = internal_path.read_text()
    internal_source = words(assign(internal, "source", internal_path))
    if internal_source != ["payload.tar.gz"]:
        die(
            f"{internal_path} source must be payload.tar.gz for scripts/package.sh, "
            f"not {internal_source}"
        )

    debian = debian_path.read_text()
    if field(debian, "Version", debian_path) != "@VERSION@-1":
        die(f"{debian_path} must set Version: @VERSION@-1 (template, revision 1)")
    rpm = rpm_path.read_text()
    if field(rpm, "Version", rpm_path) != "@VERSION@":
        die(f"{rpm_path} must set Version: @VERSION@ (template filled by package.sh)")
    if field(rpm, "Release", rpm_path) != "1":
        die(f"{rpm_path} Release must be 1")
    if "@VERSION@-1" not in rpm:
        die(f"{rpm_path} changelog must include @VERSION@-1")


def set_pkgver(root: pathlib.Path, new: str) -> None:
    path = root / "packaging/arch/PKGBUILD"
    replace_assign(path, "pkgver", new)
    # Never carry a previous release's tarball hash onto a new pkgver.
    replace_assign(path, "sha256sums", f"('{PLACEHOLDER}')")


def set_tarball_sha256(directory: pathlib.Path, digest: str) -> None:
    if not SHA256_RE.fullmatch(digest):
        die(f"tarball sha256 must be a 64-character lowercase hex digest, not {digest!r}")
    path = directory / "PKGBUILD"
    replace_assign(path, "sha256sums", f"('{digest}')")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command", choices=("check", "set-pkgver", "set-tarball-sha256")
    )
    parser.add_argument("version_or_hash")
    parser.add_argument("hash", nargs="?")
    parser.add_argument("--root", default=".", type=pathlib.Path)
    parser.add_argument(
        "--recipe-dir",
        type=pathlib.Path,
        default=None,
        help="directory containing a PKGBUILD (set-tarball-sha256 only)",
    )
    args = parser.parse_args()
    root = args.root.resolve()
    if args.command == "check":
        check(root, args.version_or_hash)
    elif args.command == "set-pkgver":
        set_pkgver(root, args.version_or_hash)
    else:
        digest = args.hash if args.hash is not None else args.version_or_hash
        if args.hash is not None:
            # allow: set-tarball-sha256 VERSION HASH
            digest = args.hash
        directory = recipe_dir(root, args.recipe_dir)
        set_tarball_sha256(directory, digest)


if __name__ == "__main__":
    main()
