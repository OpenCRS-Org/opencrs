/** The spec version this package implements, such as "0.1". */
export function version(): string;

export type DocumentKind = "index" | "screens" | "creative" | "report" | "rule-pack";

export interface SchemaError {
  /** JSON pointer into the document. */
  path: string;
  message: string;
}

export type Severity = "error" | "warning" | "info";

export interface Check {
  code: string;
  severity: Severity;
  message: string;
  rect_px?: [number, number, number, number];
  element?: string;
  screens?: string[];
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
