/** The spec version this package implements, such as "0.1". */
export function version(): string;

export type DocumentKind = "index" | "screens" | "creative" | "report" | "rule-pack";

export interface SchemaError {
  /** JSON pointer into the document. */
  path: string;
  message: string;
}

/** `not_checked` is also used, for a finding such as CRS-TOO-COMPLEX. */
export type Severity = "error" | "warning" | "info" | "not_checked";

export interface Check {
  code: string;
  severity: Severity;
  message: string;
  rect_px?: [number, number, number, number];
  element?: string;
  /** Where in a screens.json a finding is caused: reported once, with no `screens`. */
  network?: string;
  venue?: string;
  format?: string;
  /** At most 1,000 screen ids, in document order. */
  screens?: string[];
  /** How many screens the finding applies to, set when `screens` is cut short. */
  screens_total?: number;
  source?: string;
}

export interface LintResult {
  kind: DocumentKind;
  schema_errors?: SchemaError[];
  /** A screens.json's own problems, found once it matches the schema. */
  document_checks?: Check[];
}

/**
 * Checks one OpenCRS document (any kind) against its schema and, for a
 * screens.json, its own consistency. Throws `Error` if the document can't be
 * read at all.
 */
export function lint(text: string): LintResult;

/** What could be read from a creative file itself. */
export interface Probe {
  /** Lower-case hex. */
  sha256: string;
  bytes: number;
  /** Sniffed from the file's signature; "" if unknown. */
  media_type: string;
  /** Absent if the header couldn't be read. */
  pixels?: [number, number];
  duration_s?: number;
}

/** Probes a file on disk, reading only the ranges it needs. */
export function probe(path: string): Probe;
/** Probes bytes already in memory. */
export function probeBytes(bytes: Uint8Array): Probe;

export interface Derived {
  min_key_cap_height?: { mm: number; px: number; basis: string };
  min_legal_cap_height?: { mm: number; px: number; basis: string };
  max_words?: { value: number; basis: string };
}

export interface Region {
  name: string;
  rect_px: [number, number, number, number];
  reason?: string;
}

/** The requirements for one screen (or a format with no screens). */
export interface Requirements {
  screen?: string;
  format: string;
  venue?: string;
  pixels: [number, number];
  physical_mm?: [number, number];
  media: Record<string, unknown>;
  safe_area?: { top: number; right: number; bottom: number; left: number };
  blocked_regions?: Region[];
  segments?: Record<string, unknown>[];
  viewing?: Record<string, unknown>;
  rules: Record<string, unknown>;
  derived?: Derived;
}

/**
 * Resolves `<publisher>/<network>/<format>` or
 * `<publisher>/<network>/screen/<screen>` against a screens.json. Throws
 * `Error` if the spec or the reference is bad.
 */
export function resolve(spec: string, ref: string): Requirements[];

/**
 * The requirements for everything a reference covers, combined so that a file
 * that meets them passes on every screen. `sources` names the screen that set
 * each value; `notes` says where no single file can pass everywhere.
 */
export interface StrictestRequirements {
  ref: string;
  screens: number;
  format: string;
  pixels: [number, number];
  physical_mm?: [number, number];
  media: Record<string, unknown>;
  safe_area?: { top: number; right: number; bottom: number; left: number };
  blocked_regions?: Region[];
  segments?: Record<string, unknown>[];
  viewing?: Record<string, unknown>;
  min_cap_height_px?: Record<string, number>;
  rules?: Record<string, unknown>;
  conditional_elements_left_out?: {
    role: string;
    when: string;
    min_cap_height_px?: number;
    within_rect_px?: [number, number, number, number];
    notes?: string;
  }[];
  notes?: string[];
  sources: Record<string, string>;
}

/**
 * Combines the screens a `<publisher>/<network>/<format>` or screen
 * reference covers into one set of requirements. Throws `Error` if the spec
 * or the reference is bad, or the screens differ in pixels.
 */
export function resolveStrictest(spec: string, ref: string): StrictestRequirements;

export type Outcome = "pass" | "warn" | "fail" | "incomplete";

export interface Report {
  opencrs: string;
  validator: { name: string; version: string };
  checked_at?: string;
  target: string;
  file?: string;
  file_sha256?: string;
  spec_updated_at?: string;
  result: Outcome;
  checks: Check[];
}

export interface CheckResult {
  reports: Report[];
  notes: string[];
}

/**
 * Checks a creative manifest against specs. `probes` maps each file name to
 * its `Probe`. `now` is an RFC 3339 time and defaults to the host's current
 * time with its offset. Throws `Error` for a bad spec, manifest, probe or `now`.
 */
export function check(
  specs: string[],
  manifest: string,
  probes: Record<string, Probe>,
  now?: string,
): CheckResult;

export interface CheckDirResult extends CheckResult {
  /**
   * The manifest's files that weren't found under `dir`, and so were judged
   * on the manifest's own declared values (pixels, size, duration) rather
   * than the files. All of them if `dir` doesn't exist.
   */
  missing: string[];
}

/**
 * As `check`, probing the manifest's files under `dir`. A file that isn't
 * there is listed in `missing`, and its declared values are used.
 */
export function checkDir(
  dir: string,
  specs: string[],
  manifest: string,
  now?: string,
): CheckDirResult;

/** The screen's width in millimetres and in pixels, for converting sizes. */
export interface ScreenWidth {
  width_mm: number;
  width_px: number;
}

/** A minimum text size and its working. */
export interface TextSize {
  /** Rounded to 0.1 mm. */
  mm: number;
  /** Whole pixels, rounded up; absent without a screen width. */
  px?: number;
  /** The exact pixel size before rounding up, to two places. */
  px_exact?: number;
  /** How it was worked out, such as "at 6 m, ratio 150". */
  basis: string;
}

export interface LegibilityRequest {
  /** Viewing distances in metres, near then far. */
  distance_m: [number, number];
  /** Text is legible at this many times its height (default 150). */
  ratio?: number;
  screen?: ScreenWidth;
  /** Dwell time in seconds, for the word limit. */
  dwell_s?: number;
  /** Reading rate in words per second (default 2). */
  rate?: number;
}

export interface Legibility {
  ratio: number;
  /** The key message, read from the far end of the range. */
  key: TextSize;
  /** Legal text, read from the near end. */
  legal: TextSize;
  /** Words in the dwell time, when one is given. */
  words?: { value: number; basis: string };
}

/**
 * Minimum text sizes for a viewing distance, as the legibility check works
 * them out. Throws `Error` for inputs that make no sense.
 */
export function legibility(request: LegibilityRequest): Legibility;

export interface DwellRequest {
  speed_kmh: number;
  /** Viewing distances in metres, near then far. */
  distance_m: [number, number];
  ratio?: number;
  rate?: number;
  screen?: ScreenWidth;
  /** Built-in rule packs, as `name@version`; see `rulePacks`. */
  rule_packs?: string[];
}

/** One rule pack's limits for a sign at a given speed. */
export interface PackLimits {
  ref: string;
  title: string;
  status: string;
  status_note?: string;
  source_title?: string;
  source_url?: string;
  /** The minimum dwell at this speed, with the band it came from. */
  min_dwell_s?: number;
  dwell_basis?: string;
  /** Set when the pack sets dwell by speed but has no band for this one. */
  dwell_unknown?: string;
  max_transition_s?: number;
  motion_allowed?: boolean;
  no_flashing: boolean;
  max_words_at_glance: boolean;
  no_sequencing: boolean;
  no_traffic_device_imagery: boolean;
  no_device_interaction: boolean;
  failure_state?: string;
  school_zone?: string;
}

export interface Dwell {
  speed_kmh: number;
  ratio: number;
  rate: number;
  glance: {
    speed_m_s: number;
    /** Seconds spent crossing the viewing range. */
    crossing_s: number;
    /** Reading time: one glance, or the crossing time if shorter. */
    seconds: number;
    words: number;
    /** Where reading must start to finish by the near end. */
    read_from_m: number;
  };
  /** Key text must be legible from where reading starts. */
  key: TextSize;
  packs: PackLimits[];
}

/**
 * What a driver at a speed can read of a sign, and the limits each named
 * rule pack sets at that speed. Throws `Error` for an unknown pack or inputs
 * that make no sense.
 */
export function dwell(request: DwellRequest): Dwell;

export interface RulePackSummary {
  ref: string;
  title: string;
  status: string;
  source_url?: string;
}

/**
 * The rule packs built in, in `ref` order. The result has the same shape as
 * the WebAssembly module's, so it is an object with the list in `rule_packs`.
 */
export function rulePacks(): { rule_packs: RulePackSummary[] };

/** One page fetched by the caller, with the headers the check looks at. */
export interface PublisherPage {
  /** The HTTP status; 0 when the fetch itself failed. */
  status: number;
  body?: string | Uint8Array;
  /** Why the fetch failed, when it did. */
  error?: string;
  etag?: string;
  cache_control?: string;
  last_modified?: string;
  content_type?: string;
  content_encoding?: string;
  /** `Access-Control-Allow-Origin`. */
  allow_origin?: string;
}

export interface Advice {
  severity: Severity;
  /** The URL it's about. */
  url: string;
  message: string;
}

/** A URL the publisher check wants fetched, and what for. */
export interface WantedUrl {
  url: string;
  /** `index` for a publisher's or delegate's `/.well-known/opencrs.json`. */
  kind: "index" | "document";
  /** The fetch must not follow a redirect to another host. */
  same_host: boolean;
}

export interface DiscoveredDocument {
  url: string;
  network?: string;
  visibility?: string;
  listed_by: string;
  delegated_by?: string;
  publisher?: string;
  networks?: string[];
  screens?: number;
  updated_at?: string;
  expires_at?: string;
  cache_control?: string;
  etag?: string;
  last_modified?: string;
  /** trusted, expired, rejected, skipped or failed. */
  status: string;
  reason?: string;
}

/** What discovery found for one publisher. */
export interface DiscoveryResult {
  domain: string;
  index_url: string;
  index: Record<string, unknown>;
  documents: DiscoveredDocument[];
  delegates?: { domain: string; scope?: string; error?: string }[];
  /** Where discovery stopped short of what the indexes list, and why. */
  limits?: string[];
}

/** What the publisher check found. */
export interface PublisherResult {
  domain: string;
  index_url: string;
  /** Why the index couldn't be used, if it couldn't. Nothing else is checked then. */
  error?: string;
  index_lint?: LintResult;
  discovery?: DiscoveryResult;
  /** Each trusted document's own problems, by URL. */
  lints?: Record<string, LintResult>;
  advice: Advice[];
  errors: number;
  warnings: number;
  /** The URLs discovery asked for that aren't in the pages given. When empty the result is final. */
  wanted: string[];
  wanted_detail: WantedUrl[];
}

/**
 * Checks a publisher's published specs from pages already fetched, keyed by
 * URL. `wanted` in the result lists the URLs it still needs: the publisher
 * chose them, so fetch them with care (see `fetchPublisher`), add them to
 * `pages` under exactly those strings and call again. `now` is an RFC 3339
 * time (default: now); `missing` is the reason shown for a URL that isn't in
 * `pages`.
 */
export function publisher(
  domain: string,
  pages: Record<string, PublisherPage>,
  now?: string,
  missing?: string,
): PublisherResult;

export interface FetchPublisherOptions {
  /** The most one page may be, in bytes (default 50 MiB). */
  maxDocumentBytes?: number;
  /** The most all pages may be together (default 180 MiB). */
  maxTotalBytes?: number;
  /** How many times the core is asked (default 5). */
  maxRounds?: number;
  /** How many URLs are fetched in all (default 4000). */
  maxPages?: number;
  /** For one request, redirects included (default 120000). */
  timeoutMs?: number;
  /** An RFC 3339 time to check at (default: now). */
  now?: string;
}

/**
 * Runs the publisher check end to end: fetches the publisher's index and the
 * documents it lists, and resolves with the same result as `publisher`. HTTPS
 * only; an address that isn't public is refused after DNS and on every
 * redirect; proxy settings are ignored. Runs off the event loop. Rejects if
 * the check still wanted URLs after its last round.
 */
export function fetchPublisher(
  domain: string,
  options?: FetchPublisherOptions,
): Promise<PublisherResult>;

export interface DiscoverOptions {
  /** Bearer tokens for partner documents, by publisher domain. */
  tokens?: Record<string, string>;
  /** Local testing: allow http:// and connections to localhost. */
  allowLocalHttp?: boolean;
  /** An RFC 3339 time to judge expiry at (default: now). */
  now?: string;
}

/**
 * Finds a publisher's specs and says which to trust, as `opencrs fetch`
 * does; resolves with its `result`. Rejects if the index itself can't be
 * fetched or trusted. Fetches as carefully as `fetchPublisher`.
 */
export function discover(domain: string, options?: DiscoverOptions): Promise<DiscoveryResult>;
