import os
from typing import Any, Literal, TypedDict

from typing_extensions import NotRequired

class OpenCRSError(ValueError): ...

def version() -> str: ...

DocumentKind = Literal["index", "screens", "creative", "report", "rule-pack"]
Severity = Literal["error", "warning", "info", "not_checked"]
Outcome = Literal["pass", "warn", "fail", "incomplete"]

class SchemaError(TypedDict):
    path: str
    message: str

class Check(TypedDict):
    code: str
    severity: Severity
    message: str
    rect_px: NotRequired[list[int]]
    element: NotRequired[str]
    network: NotRequired[str]
    venue: NotRequired[str]
    format: NotRequired[str]
    screens: NotRequired[list[str]]
    screens_total: NotRequired[int]
    source: NotRequired[str]

class LintResult(TypedDict):
    kind: DocumentKind
    schema_errors: NotRequired[list[SchemaError]]
    document_checks: NotRequired[list[Check]]

def lint(text: str) -> LintResult: ...

class Probe(TypedDict):
    sha256: str
    bytes: int
    media_type: str
    pixels: NotRequired[list[int]]
    duration_s: NotRequired[float]

def probe(path: str | os.PathLike[str]) -> Probe: ...
def probe_bytes(data: bytes) -> Probe: ...

class Region(TypedDict):
    name: str
    rect_px: list[int]
    reason: NotRequired[str]

class Requirements(TypedDict):
    screen: NotRequired[str]
    format: str
    venue: NotRequired[str]
    pixels: list[int]
    physical_mm: NotRequired[list[float]]
    media: dict[str, Any]
    safe_area: NotRequired[dict[str, int]]
    blocked_regions: NotRequired[list[Region]]
    segments: NotRequired[list[dict[str, Any]]]
    viewing: NotRequired[dict[str, Any]]
    rules: dict[str, Any]
    derived: NotRequired[dict[str, Any]]

def resolve(spec: str, ref: str) -> list[Requirements]: ...

class Report(TypedDict):
    opencrs: str
    validator: dict[str, str]
    checked_at: NotRequired[str]
    target: str
    file: NotRequired[str]
    file_sha256: NotRequired[str]
    spec_updated_at: NotRequired[str]
    result: Outcome
    checks: list[Check]

class CheckResult(TypedDict):
    reports: list[Report]
    notes: list[str]

def check(
    specs: list[str],
    manifest: str,
    probes: dict[str, Probe],
    now: str | None = None,
) -> CheckResult:
    """Raises OpenCRSError for a bad spec, manifest, probe or `now`."""

class CheckDirResult(CheckResult):
    missing: list[str]
    """The manifest's files not found under `dir`, judged on the manifest's
    own declared values rather than the files (all of them if `dir` doesn't
    exist)."""

def check_dir(
    dir: str | os.PathLike[str],
    specs: list[str],
    manifest: str,
    now: str | None = None,
) -> CheckDirResult:
    """As `check`, probing the manifest's files under `dir`. A file that isn't
    there is listed in `missing`, and its declared values are used."""
