#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["packaging"]
# ///
"""Checks the language bindings' versions against a release tag.

  check-binding-versions.py v0.1.2
      package.json, both Cargo.toml files and pyproject.toml under bindings/
      must be 0.1.2 (PEP 440 spelling for pyproject.toml).

  check-binding-versions.py --wheels v0.1.2 dist/*.whl
      Exactly seven wheels, each named for the tag's version.

Run from the repository root.
"""

import json
import re
import sys
import tomllib
from pathlib import Path

TAG_RE = r"v(\d+\.\d+\.\d+)(?:-(alpha|beta|rc)\.?(\d+))?"
WHEELS = 7


def parse_tag(tag):
    m = re.fullmatch(TAG_RE, tag)
    if not m:
        sys.exit(f"{tag!r} isn't a release tag like v1.2.3 or v1.2.3-rc.1")
    return m


def check_files(tag):
    parse_tag(tag)
    from packaging.version import Version

    want = tag[1:]

    def toml(path):
        return tomllib.loads(Path(path).read_text())

    found = {
        "bindings/npm/package.json": json.loads(Path("bindings/npm/package.json").read_text())["version"],
        "bindings/npm/Cargo.toml": toml("bindings/npm/Cargo.toml")["package"]["version"],
        "bindings/python/Cargo.toml": toml("bindings/python/Cargo.toml")["package"]["version"],
    }
    bad = [f"{p} is {v}" for p, v in found.items() if v != want]
    # PyPI spells prereleases the PEP 440 way: 0.2.0rc1 for 0.2.0-rc.1.
    py = toml("bindings/python/pyproject.toml")["project"]["version"]
    if Version(py) != Version(want):
        bad.append(f"bindings/python/pyproject.toml is {py}")
    if bad:
        sys.exit(
            f"Tag {tag} needs binding version {want}, but "
            + "; ".join(bad)
            + ". Bump them (and Cargo.lock) in the release commit, then move the tag."
        )
    print(f"Binding versions match {tag}")


def check_wheels(tag, wheels):
    m = parse_tag(tag)
    pre = {"alpha": "a", "beta": "b", "rc": "rc"}[m[2]] + m[3] if m[2] else ""
    version = m[1] + pre
    names = [Path(w).name for w in wheels]
    if len(names) != WHEELS:
        sys.exit(f"expected {WHEELS} wheels, found {len(names)}: {names}")
    for name in names:
        if not name.startswith(f"opencrs-{version}-"):
            sys.exit(f"{name} isn't version {version}")
    print(f"{WHEELS} wheels at {version}")


if __name__ == "__main__":
    args = sys.argv[1:]
    if len(args) == 1:
        check_files(args[0])
    elif len(args) >= 2 and args[0] == "--wheels":
        check_wheels(args[1], args[2:])
    else:
        sys.exit(__doc__)
