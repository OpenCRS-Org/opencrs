"""Python bindings for OpenCRS, the Open Creative Requirements Specification.

A thin layer over the Rust core: lint documents, probe creative files,
resolve screen requirements, check creative, work out legibility and dwell,
and check a publisher's published specs, fetching them if you ask. Results
are plain dicts and lists; errors raise `OpenCRSError`.
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
    "discover",
    "dwell",
    "fetch_publisher",
    "legibility",
    "lint",
    "probe",
    "probe_bytes",
    "publisher",
    "resolve",
    "resolve_strictest",
    "rule_packs",
    "version",
]


def version() -> str:
    return _native.version()


def lint(text: str) -> dict[str, Any]:
    return json.loads(_native.lint(text))


def resolve(spec: str, ref: str) -> list[dict[str, Any]]:
    return json.loads(_native.resolve(spec, ref))


def resolve_strictest(spec: str, ref: str) -> dict[str, Any]:
    return json.loads(_native.resolve_strictest(spec, ref))


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


def _screen(screen: dict[str, Any] | None) -> dict[str, Any]:
    if screen is None:
        return {}
    try:
        return {"screen_mm": screen["width_mm"], "screen_px": screen["width_px"]}
    except (KeyError, TypeError) as e:
        raise OpenCRSError('screen must be {"width_mm": ..., "width_px": ...}') from e


def legibility(
    distance_m: tuple[float, float] | list[float],
    *,
    ratio: float | None = None,
    screen: dict[str, Any] | None = None,
    dwell_s: float | None = None,
    rate: float | None = None,
) -> dict[str, Any]:
    try:
        near, far = distance_m
    except (TypeError, ValueError) as e:
        raise OpenCRSError("distance_m must be [near, far]") from e
    return json.loads(
        _native.legibility(
            near, far, ratio=ratio, dwell_s=dwell_s, rate=rate, **_screen(screen)
        )
    )


def dwell(
    speed_kmh: float,
    distance_m: tuple[float, float] | list[float],
    *,
    ratio: float | None = None,
    rate: float | None = None,
    screen: dict[str, Any] | None = None,
    rule_packs: list[str] | tuple[str, ...] = (),
) -> dict[str, Any]:
    try:
        near, far = distance_m
    except (TypeError, ValueError) as e:
        raise OpenCRSError("distance_m must be [near, far]") from e
    return json.loads(
        _native.dwell(
            speed_kmh,
            near,
            far,
            ratio=ratio,
            rate=rate,
            rule_packs=list(rule_packs),
            **_screen(screen),
        )
    )


def rule_packs() -> dict[str, Any]:
    return json.loads(_native.rule_packs())


def publisher(
    domain: str,
    pages: dict[str, dict[str, Any]],
    now: str | None = None,
    missing: str | None = None,
) -> dict[str, Any]:
    heads: list[dict[str, Any]] = []
    bodies: list[bytes] = []
    for url, page in pages.items():
        head = dict(page)
        body = head.pop("body", b"")
        if isinstance(body, str):
            body = body.encode("utf-8")
        elif not isinstance(body, (bytes, bytearray, memoryview)):
            raise TypeError(f"pages[{url!r}]['body'] needs str or bytes")
        heads.append({"url": url, **head})
        bodies.append(bytes(body))
    try:
        heads_json = json.dumps(heads, allow_nan=False)
    except (TypeError, ValueError) as e:
        raise OpenCRSError(f"pages: {e}") from e
    return json.loads(_native.publisher(domain, heads_json, bodies, now, missing))


def fetch_publisher(
    domain: str,
    *,
    max_document_bytes: int = 0,
    max_total_bytes: int = 0,
    max_rounds: int = 0,
    max_pages: int = 0,
    timeout: float = 0.0,
    total_timeout: float = 0.0,
    now: str | None = None,
) -> dict[str, Any]:
    return json.loads(
        _native.fetch_publisher(
            domain,
            max_document_bytes,
            max_total_bytes,
            max_rounds,
            max_pages,
            timeout,
            total_timeout,
            now,
        )
    )


def discover(
    domain: str,
    *,
    tokens: dict[str, str] | None = None,
    unsafe_allow_local_http: bool = False,
    timeout: float = 0.0,
    total_timeout: float = 0.0,
    now: str | None = None,
) -> dict[str, Any]:
    return json.loads(
        _native.discover(
            domain, tokens or {}, unsafe_allow_local_http, timeout, total_timeout, now
        )
    )
