"""Python bindings for OpenCRS, the Open Creative Requirements Specification.

A thin layer over the Rust core: lint documents, probe creative files and
resolve screen requirements. Results are plain dicts and lists; errors raise
`OpenCRSError`.
"""

from __future__ import annotations

import json
import os
from typing import Any

from . import _native
from ._native import OpenCRSError

__all__ = [
    "OpenCRSError",
    "check",
    "check_dir",
    "lint",
    "probe",
    "probe_bytes",
    "resolve",
    "version",
]


def version() -> str:
    return _native.version()


def lint(text: str) -> dict[str, Any]:
    return json.loads(_native.lint(text))


def resolve(spec: str, ref: str) -> list[dict[str, Any]]:
    return json.loads(_native.resolve(spec, ref))


def probe(path: str | os.PathLike[str]) -> dict[str, Any]:
    return json.loads(_native.probe(os.fspath(path)))


def probe_bytes(data: bytes) -> dict[str, Any]:
    if not isinstance(data, (bytes, bytearray, memoryview)):
        raise TypeError("probe_bytes needs bytes, bytearray or memoryview")
    return json.loads(_native.probe_bytes_json(bytes(data)))


def check(
    specs: list[str],
    manifest: str,
    probes: dict[str, dict[str, Any]],
    now: str | None = None,
) -> dict[str, Any]:
    try:
        probes_json = json.dumps(probes, allow_nan=False)
    except (TypeError, ValueError) as e:
        raise OpenCRSError(f"probes: {e}") from e
    return json.loads(_native.check(specs, manifest, probes_json, now))


def check_dir(
    dir: str | os.PathLike[str],
    specs: list[str],
    manifest: str,
    now: str | None = None,
) -> dict[str, Any]:
    return json.loads(_native.check_dir(os.fspath(dir), specs, manifest, now))
