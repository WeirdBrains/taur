#!/usr/bin/env python3
"""Check or update TAUR package-version metadata for scripts/release.sh.

PKGBUILD pkgver and packaging/arch/.SRCINFO must match the workspace version.
pkgrel stays 1, matching Debian Version: @VERSION@-1 and RPM Release: 1.
Debian and RPM templates keep @VERSION@ for scripts/package.sh to fill in.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys


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


def check(root: pathlib.Path, version: str) -> None:
    pkgbuild_path = root / "packaging/arch/PKGBUILD"
    srcinfo_path = root / "packaging/arch/.SRCINFO"
    debian_path = root / "packaging/debian/control.in"
    rpm_path = root / "packaging/rpm/taur.spec"
    for path in (pkgbuild_path, srcinfo_path, debian_path, rpm_path):
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

    for key in ("arch", "depends", "source", "sha256sums"):
        pb = words(assign(pkgbuild, key, pkgbuild_path))
        si = srcinfo_list(srcinfo, key, srcinfo_path)
        if pb != si:
            die(f"{srcinfo_path} {key} is {si}, but {pkgbuild_path} has {pb}")

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
    if not path.is_file():
        die(f"missing {path}")
    lines = path.read_text().splitlines(keepends=True)
    out = []
    found = 0
    for line in lines:
        if line.startswith("pkgver="):
            nl = "\n" if line.endswith("\n") else ""
            out.append(f"pkgver={new}{nl}")
            found += 1
            continue
        out.append(line)
    if found != 1:
        die(f"{path} must have exactly one pkgver= line")
    path.write_text("".join(out))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "set-pkgver"))
    parser.add_argument("version")
    parser.add_argument("--root", default=".", type=pathlib.Path)
    args = parser.parse_args()
    root = args.root.resolve()
    if args.command == "check":
        check(root, args.version)
    else:
        set_pkgver(root, args.version)


if __name__ == "__main__":
    main()
