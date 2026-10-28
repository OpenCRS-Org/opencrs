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

class StrictestRequirements(TypedDict):
    ref: str
    screens: int
    format: str
    pixels: list[int]
    physical_mm: NotRequired[list[float]]
    media: dict[str, Any]
    safe_area: NotRequired[dict[str, int]]
    blocked_regions: NotRequired[list[Region]]
    segments: NotRequired[list[dict[str, Any]]]
    viewing: NotRequired[dict[str, Any]]
    min_cap_height_px: NotRequired[dict[str, int]]
    rules: NotRequired[dict[str, Any]]
    conditional_elements_left_out: NotRequired[list[dict[str, Any]]]
    notes: NotRequired[list[str]]
    sources: dict[str, str]

def resolve_strictest(spec: str, ref: str) -> StrictestRequirements: ...

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


class ScreenWidth(TypedDict):
    width_mm: float
    width_px: int

class TextSize(TypedDict):
    mm: float
    """Rounded to 0.1 mm."""
    px: NotRequired[int]
    """Whole pixels, rounded up; absent without a screen width."""
    px_exact: NotRequired[float]
    basis: str
    """How it was worked out, such as "at 6 m, ratio 150"."""

class WordLimit(TypedDict):
    value: int
    basis: str

class Legibility(TypedDict):
    ratio: float
    key: TextSize
    """The key message, read from the far end of the range."""
    legal: TextSize
    """Legal text, read from the near end."""
    words: NotRequired[WordLimit]

def legibility(
    distance_m: tuple[float, float] | list[float],
    *,
    ratio: float | None = None,
    screen: ScreenWidth | None = None,
    dwell_s: float | None = None,
    rate: float | None = None,
) -> Legibility:
    """Minimum text sizes for a viewing distance `[near, far]` in metres, as the
    legibility check works them out. `ratio` defaults to 150 and `rate` to 2
    words per second. Raises OpenCRSError for inputs that make no sense."""

class PackLimits(TypedDict):
    ref: str
    title: str
    status: str
    status_note: NotRequired[str]
    source_title: NotRequired[str]
    source_url: NotRequired[str]
    min_dwell_s: NotRequired[float]
    dwell_basis: NotRequired[str]
    dwell_unknown: NotRequired[str]
    max_transition_s: NotRequired[float]
    motion_allowed: NotRequired[bool]
    no_flashing: bool
    max_words_at_glance: bool
    no_sequencing: bool
    no_traffic_device_imagery: bool
    no_device_interaction: bool
    failure_state: NotRequired[str]
    school_zone: NotRequired[str]

class GlanceView(TypedDict):
    speed_m_s: float
    crossing_s: float
    seconds: float
    words: int
    read_from_m: float

class Dwell(TypedDict):
    speed_kmh: float
    ratio: float
    rate: float
    glance: GlanceView
    key: TextSize
    packs: list[PackLimits]

def dwell(
    speed_kmh: float,
    distance_m: tuple[float, float] | list[float],
    *,
    ratio: float | None = None,
    rate: float | None = None,
    screen: ScreenWidth | None = None,
    rule_packs: list[str] | tuple[str, ...] = (),
) -> Dwell:
    """What a driver at `speed_kmh` can read of a sign seen from `distance_m`
    `[near, far]`, and the limits each named built-in rule pack sets at that
    speed. Raises OpenCRSError for an unknown pack or inputs that make no sense."""

class RulePackSummary(TypedDict):
    ref: str
    title: str
    status: str
    source_url: NotRequired[str]

class RulePacks(TypedDict):
    rule_packs: list[RulePackSummary]

def rule_packs() -> RulePacks:
    """The built-in rule packs, in `ref` order. The result has the shape the
    WebAssembly module gives, so the list is under `rule_packs`."""

class Page(TypedDict):
    status: int
    """The HTTP status; 0 when the fetch itself failed."""
    body: NotRequired[str | bytes]
    error: NotRequired[str]
    etag: NotRequired[str]
    cache_control: NotRequired[str]
    last_modified: NotRequired[str]
    content_type: NotRequired[str]
    content_encoding: NotRequired[str]
    allow_origin: NotRequired[str]
    """`Access-Control-Allow-Origin`."""

class Advice(TypedDict):
    severity: Severity
    url: str
    message: str

class WantedUrl(TypedDict):
    url: str
    kind: Literal["index", "document"]
    same_host: bool

class DiscoveredDocument(TypedDict):
    url: str
    network: NotRequired[str]
    visibility: NotRequired[str]
    listed_by: str
    delegated_by: NotRequired[str]
    publisher: NotRequired[str]
    networks: NotRequired[list[str]]
    screens: NotRequired[int]
    updated_at: NotRequired[str]
    expires_at: NotRequired[str]
    etag: NotRequired[str]
    cache_control: NotRequired[str]
    last_modified: NotRequired[str]
    status: str
    """trusted, expired, rejected, skipped or failed."""
    reason: NotRequired[str]

class DiscoveryResult(TypedDict):
    domain: str
    index_url: str
    index: dict[str, Any]
    documents: list[DiscoveredDocument]
    delegates: NotRequired[list[dict[str, Any]]]
    limits: NotRequired[list[str]]

class PublisherResult(TypedDict):
    domain: str
    index_url: str
    error: NotRequired[str]
    """Why the index couldn't be used. Nothing else is checked then."""
    index_lint: NotRequired[LintResult]
    discovery: NotRequired[DiscoveryResult]
    lints: NotRequired[dict[str, LintResult]]
    advice: list[Advice]
    errors: int
    warnings: int
    wanted: list[str]
    """The URLs still needed, not in `pages`. When empty the result is final."""
    wanted_detail: list[WantedUrl]

def publisher(
    domain: str,
    pages: dict[str, Page],
    now: str | None = None,
    missing: str | None = None,
) -> PublisherResult:
    """Checks a publisher's published specs from pages already fetched, keyed by
    URL. `wanted` in the result lists the URLs still needed; the publisher chose
    them, so fetch them with care (see `fetch_publisher`), add them to `pages`
    under exactly those strings and call again. `now` is an RFC 3339 time
    (default: now); `missing` is the reason shown for a URL not in `pages`."""

def fetch_publisher(
    domain: str,
    *,
    max_document_bytes: int = 0,
    max_total_bytes: int = 0,
    max_rounds: int = 0,
    max_pages: int = 0,
    timeout: float = 0.0,
    now: str | None = None,
) -> PublisherResult:
    """Runs the publisher check end to end: fetches the publisher's index and the
    documents it lists and returns what `publisher` would. HTTPS only; an address
    that isn't public is refused after DNS and on every redirect; proxy settings
    are ignored. Zero limits mean the defaults: 50 MiB a page, 180 MiB in all, 5
    rounds, 4000 pages, 120 seconds a request. The GIL is released while it
    waits on the network. Raises OpenCRSError if the check still wanted URLs
    after its last round."""

def discover(
    domain: str,
    *,
    tokens: dict[str, str] | None = None,
    allow_local_http: bool = False,
    now: str | None = None,
) -> DiscoveryResult:
    """Finds a publisher's specs and says which to trust, as `opencrs fetch`
    does (its `result`). `tokens` maps a domain to the bearer token for its
    partner documents. `allow_local_http` is for local testing: it allows
    http:// and connections to localhost. Raises OpenCRSError if the index
    itself can't be fetched or trusted."""
