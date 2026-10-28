//! Node.js bindings for the OpenCRS core. Every function is a thin layer
//! over `opencrs`: it takes and returns plain JS values (objects, not
//! JSON strings), and a core error becomes a thrown `Error` carrying the
//! core's message.

use chrono::{DateTime, FixedOffset};
use napi::bindgen_prelude::{Either, Object, Uint8Array, Unknown};
use napi::bindgen_prelude::{ToNapiValue, TypeName};
use napi::{Env, Error, Result, ValueType, sys};
use napi_derive::napi;
use opencrs::batch;
use opencrs::calc::{self, ScreenWidth};
use opencrs::fs::{check_dir_text, parse_now, probe_path, run_check};
use opencrs::load::decode_document;
use opencrs::probe::Probe;
use opencrs::probe::probe_bytes;
use opencrs::publisher::{Page, check_publisher};
use opencrs::resolve::parse_ref;
use opencrs::schema::SchemaOptions;
use opencrs::strictest::Strictest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn fail(e: impl std::fmt::Display) -> Error {
    Error::from_reason(e.to_string())
}

fn to_value<T: Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(fail)
}

/// The spec version this package implements.
#[napi]
pub fn version() -> String {
    opencrs::SPEC_VERSION.to_string()
}

/// Checks one OpenCRS document (any kind) against its schema and, for a
/// screens.json, its own consistency.
#[napi]
pub fn lint(text: String) -> Result<Value> {
    let res = batch::lint(text.as_bytes(), None, &SchemaOptions::default()).map_err(fail)?;
    to_value(&res)
}

/// Resolves a format or screen reference to the requirements of each of its
/// screens.
#[napi]
pub fn resolve(spec: String, r#ref: String) -> Result<Value> {
    let doc = decode_document(spec.as_bytes()).map_err(fail)?;
    let r = parse_ref(&r#ref).map_err(fail)?;
    let reqs = doc.resolve(&r).map_err(fail)?;
    let views: Vec<_> = reqs.iter().map(|r| r.view()).collect();
    to_value(&views)
}

/// Resolves a format or screen reference to one set of requirements for
/// everything it covers: a file that meets it passes on every screen.
#[napi(js_name = "resolveStrictest")]
pub fn resolve_strictest(spec: String, r#ref: String) -> Result<Value> {
    let s = opencrs::budget::ensure(spec.len(), || Strictest::from_text(spec.as_bytes(), &r#ref))
        .map_err(fail)?;
    serde_json::from_slice(&s.to_json()).map_err(fail)
}

/// What can be read from a file's own bytes: size, hash, type, pixels and
/// duration. Reads only the ranges it needs after hashing the file.
#[napi]
pub fn probe(path: String) -> Result<Value> {
    let p = probe_path(std::path::Path::new(&path)).map_err(|e| fail(format!("{path}: {e}")))?;
    to_value(&p)
}

/// `probe` for bytes already in memory.
#[napi(js_name = "probeBytes")]
pub fn probe_bytes_js(bytes: Uint8Array) -> Result<Value> {
    to_value(&probe_bytes(&bytes))
}

/// Checks creative against specs. `probes` maps file names to `probe` results.
#[napi]
pub fn check(
    specs: Vec<String>,
    manifest: String,
    probes: Value,
    now: Option<String>,
) -> Result<Value> {
    let probes: HashMap<String, Probe> = serde_json::from_value(probes).map_err(fail)?;
    run_check(&specs, &manifest, &probes, now.as_deref()).map_err(fail)
}

/// Like `check`, probing the files named in the manifest under `dir`. The
/// result's `missing` lists the files that weren't there, judged on the
/// manifest's declared values instead.
#[napi(js_name = "checkDir")]
pub fn check_dir(
    dir: String,
    specs: Vec<String>,
    manifest: String,
    now: Option<String>,
) -> Result<Value> {
    check_dir_text(&dir, &specs, &manifest, now.as_deref()).map_err(fail)
}

#[derive(Deserialize)]
struct ScreenReq {
    width_mm: f64,
    width_px: i64,
}

impl ScreenReq {
    fn width(&self) -> ScreenWidth {
        ScreenWidth {
            mm: self.width_mm,
            px: self.width_px,
        }
    }
}

#[derive(Deserialize)]
struct LegibilityReq {
    distance_m: [f64; 2],
    ratio: Option<f64>,
    screen: Option<ScreenReq>,
    dwell_s: Option<f64>,
    rate: Option<f64>,
}

#[derive(Deserialize)]
struct DwellReq {
    speed_kmh: f64,
    distance_m: [f64; 2],
    ratio: Option<f64>,
    rate: Option<f64>,
    screen: Option<ScreenReq>,
    #[serde(default)]
    rule_packs: Vec<String>,
}

fn request<T: serde::de::DeserializeOwned>(req: Value, usage: &str) -> Result<T> {
    serde_json::from_value(req).map_err(|e| fail(format!("usage: {usage}: {e}")))
}

/// Minimum text sizes for a viewing distance, and the word limit for a
/// dwell time, worked out as the legibility check does. `req` is
/// `{distance_m: [near, far], ratio?, screen?: {width_mm, width_px}, dwell_s?, rate?}`.
#[napi]
pub fn legibility(req: Value) -> Result<Value> {
    let r: LegibilityReq = request(
        req,
        r#"{"distance_m": [near, far], "ratio"?, "screen"?: {"width_mm", "width_px"}, "dwell_s"?, "rate"?}"#,
    )?;
    let s = r.screen.as_ref().map(ScreenReq::width);
    let l = calc::legibility(r.distance_m, r.ratio, s, r.dwell_s, r.rate).map_err(fail)?;
    to_value(&l)
}

/// What a driver can read of a sign at a speed, and each named built-in
/// rule pack's limits at it. `req` is `{speed_kmh, distance_m: [near, far],
/// ratio?, rate?, screen?, rule_packs?: [ref, ...]}`.
#[napi]
pub fn dwell(req: Value) -> Result<Value> {
    let r: DwellReq = request(
        req,
        r#"{"speed_kmh", "distance_m": [near, far], "ratio"?, "rate"?, "screen"?, "rule_packs"?: [...]}"#,
    )?;
    let s = r.screen.as_ref().map(ScreenReq::width);
    let d =
        calc::dwell(r.speed_kmh, r.distance_m, r.ratio, r.rate, s, &r.rule_packs).map_err(fail)?;
    to_value(&d)
}

/// The rule packs built into the validator, as `{rule_packs: [{ref, title,
/// status, source_url?}, ...]}`.
#[napi(js_name = "rulePacks")]
pub fn rule_packs() -> Result<Value> {
    to_value(&serde_json::json!({ "rule_packs": calc::pack_list() }))
}

/// A result as a JS object, for the promises (`Value` can't be a task's
/// result directly).
pub struct Json(Value);

impl TypeName for Json {
    fn type_name() -> &'static str {
        "object"
    }

    fn value_type() -> ValueType {
        ValueType::Object
    }
}

impl ToNapiValue for Json {
    unsafe fn to_napi_value(env: sys::napi_env, val: Json) -> Result<sys::napi_value> {
        unsafe { Value::to_napi_value(env, val.0) }
    }
}

/// An RFC 3339 time, or the host's now for none. Parsed where the work runs,
/// so a bad one rejects the promise.
fn parse_time(now: Option<&str>) -> Result<DateTime<FixedOffset>> {
    parse_now(now.filter(|n| !n.is_empty())).map_err(fail)
}

fn string_property(page: &Object, name: &str) -> Result<String> {
    Ok(page.get::<String>(name)?.unwrap_or_default())
}

/// Checks a publisher's published specs from pages already fetched, keyed
/// by URL: `{status, body, error?, etag?, cache_control?, last_modified?,
/// content_type?, content_encoding?, allow_origin?}`. A body is a string or
/// bytes. `now` is an RFC 3339 time (default: now); `missing` is the reason
/// shown for a URL that isn't in `pages` (default "not fetched").
#[napi]
pub fn publisher(
    domain: String,
    pages: Object,
    now: Option<String>,
    missing: Option<String>,
) -> Result<Value> {
    let mut bodies: Vec<Either<String, Uint8Array>> = Vec::new();
    let mut heads: Vec<(String, Page<'static>)> = Vec::new();
    for url in Object::keys(&pages)? {
        let not_object = || fail(format!("pages[{url:?}] must be an object"));
        let page: Unknown = pages.get(&url)?.ok_or_else(not_object)?;
        if page.get_type()? != ValueType::Object {
            return Err(not_object());
        }
        let page: Object = unsafe { page.cast()? };
        let head = Page {
            status: page.get::<u32>("status")?.unwrap_or(0) as u16,
            error: string_property(&page, "error")?,
            etag: string_property(&page, "etag")?,
            cache_control: string_property(&page, "cache_control")?,
            last_modified: string_property(&page, "last_modified")?,
            content_type: string_property(&page, "content_type")?,
            content_encoding: string_property(&page, "content_encoding")?,
            allow_origin: string_property(&page, "allow_origin")?,
            body: &[],
        };
        bodies.push(
            page.get::<Either<String, Uint8Array>>("body")?
                .unwrap_or_else(|| Either::A(String::new())),
        );
        heads.push((url, head));
    }
    // A page borrows its own body, which stays where it arrived.
    let pages: BTreeMap<String, Page<'_>> = heads
        .into_iter()
        .zip(&bodies)
        .map(|((url, head), body)| {
            let body: &[u8] = match body {
                Either::A(s) => s.as_bytes(),
                Either::B(b) => b,
            };
            (url, Page { body, ..head })
        })
        .collect();
    let now = parse_time(now.as_deref())?;
    let missing = missing
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or("not fetched");
    to_value(&check_publisher(&domain, &pages, missing, now))
}

/// A way to stop a `fetchPublisher` or `discover` already under way: the
/// JavaScript wrapper makes one for an `AbortSignal` and calls `cancel()`
/// when it aborts.
#[napi]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

#[napi]
impl CancelToken {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> CancelToken {
        CancelToken {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Stops the work the token was given to, between pages and within a
    /// read; its promise rejects.
    #[napi]
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }
}

/// Runs `work` on a thread of its own and returns a promise for it. The
/// libuv pool is not used: it is four threads shared with `fs`, `dns`,
/// `crypto` and `zlib` for the whole process, and a few calls waiting on
/// slow servers would starve them all, however limited their time. A panic
/// is a rejection, not a crash.
fn spawn<'env>(
    env: &'env Env,
    work: impl FnOnce() -> Result<Value> + Send + 'static,
) -> Result<Object<'env>> {
    let (deferred, promise) = env.create_deferred::<Json, _>()?;
    std::thread::spawn(move || {
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err(fail("internal error: the check panicked")));
        match out {
            Ok(v) => deferred.resolve(move |_| Ok(Json(v))),
            Err(e) => deferred.reject(e),
        }
    });
    Ok(promise)
}

/// Limits for `fetchPublisher`. A missing or zero number is the default, and
/// one above the most allowed is the most allowed.
#[napi(object)]
pub struct FetchPublisherOptions {
    /// The most one page may be, in bytes (default and most 50 MiB).
    pub max_document_bytes: Option<f64>,
    /// The most all pages may be together, counting bytes read from the
    /// network and again decompressed (default 180 MiB, most 512 MiB).
    pub max_total_bytes: Option<f64>,
    /// How many times the core is asked (default 5, most 10).
    pub max_rounds: Option<f64>,
    /// How many URLs are fetched in all (default 4000, most 10000).
    pub max_pages: Option<f64>,
    /// For one request, redirects included (default 120000, most 3600000).
    pub timeout_ms: Option<f64>,
    /// For the whole check (default 300000, most 3600000).
    pub total_timeout_ms: Option<f64>,
    /// An RFC 3339 time to check at (default: now).
    pub now: Option<String>,
}

/// A count or size from JavaScript: anything that isn't a positive number is
/// the default (zero); infinity and the like saturate and are clamped later.
fn count(v: Option<f64>) -> usize {
    v.filter(|v| *v > 0.0).map_or(0, |v| v as usize)
}

/// A number of milliseconds; zero is the default.
fn millis(v: Option<f64>) -> Duration {
    v.filter(|v| *v > 0.0).map_or(Duration::ZERO, |v| {
        Duration::try_from_secs_f64(v / 1000.0).unwrap_or(Duration::MAX)
    })
}

fn cancel_flag(token: Option<&CancelToken>) -> Option<Arc<AtomicBool>> {
    token.map(|t| t.flag.clone())
}

/// Runs the publisher check end to end: fetches the publisher's index and
/// the documents it lists, as the command line does (HTTPS only, no
/// non-public address, no proxy, size caps), and resolves with the same
/// result as `publisher`. Runs on a thread of its own, and is held to
/// `totalTimeoutMs`. The index.js wrapper adds `signal`, an `AbortSignal`.
#[napi(js_name = "fetchPublisher")]
pub fn fetch_publisher<'env>(
    env: &'env Env,
    domain: String,
    options: Option<FetchPublisherOptions>,
    token: Option<&CancelToken>,
) -> Result<Object<'env>> {
    let o = options.unwrap_or(FetchPublisherOptions {
        max_document_bytes: None,
        max_total_bytes: None,
        max_rounds: None,
        max_pages: None,
        timeout_ms: None,
        total_timeout_ms: None,
        now: None,
    });
    let opts = opencrs_fetch::FetchOptions {
        max_document_bytes: count(o.max_document_bytes),
        max_total_bytes: count(o.max_total_bytes),
        max_rounds: count(o.max_rounds),
        max_pages: count(o.max_pages),
        timeout: millis(o.timeout_ms),
        total_timeout: millis(o.total_timeout_ms),
        cancel: cancel_flag(token),
    };
    let now = o.now;
    spawn(env, move || {
        let now = parse_time(now.as_deref())?;
        let f = opencrs_fetch::fetch_publisher(&domain, now, &opts);
        if f.cancelled {
            return Err(fail("cancelled"));
        }
        if f.rounds_exhausted {
            return Err(fail(format!(
                "the publisher check still wanted URLs after its last round ({} rounds)",
                f.rounds
            )));
        }
        to_value(&f.check)
    })
}

/// Options for `discover`.
#[napi(object)]
pub struct DiscoverOptions {
    /// Bearer tokens for partner documents, by publisher domain.
    pub tokens: Option<HashMap<String, String>>,
    /// For local testing only: let a publisher that is itself on localhost
    /// use `http://` and reach localhost. Anything else about a public
    /// publisher is unchanged: it never gets to reach loopback. Do not set
    /// this in code that checks publishers other people name.
    pub unsafe_allow_local_http: Option<bool>,
    /// For one request, redirects included (default 30000, most 3600000).
    pub timeout_ms: Option<f64>,
    /// For the whole run (default 300000, most 3600000).
    pub total_timeout_ms: Option<f64>,
    /// An RFC 3339 time to judge expiry at (default: now).
    pub now: Option<String>,
}

/// Finds a publisher's specs and says which to trust, as `opencrs fetch
/// DOMAIN --json` does (its `result`). Rejects if the index itself can't be
/// fetched or trusted. Runs on a thread of its own, held to
/// `totalTimeoutMs`.
#[napi]
pub fn discover<'env>(
    env: &'env Env,
    domain: String,
    options: Option<DiscoverOptions>,
    token: Option<&CancelToken>,
) -> Result<Object<'env>> {
    let o = options.unwrap_or(DiscoverOptions {
        tokens: None,
        unsafe_allow_local_http: None,
        timeout_ms: None,
        total_timeout_ms: None,
        now: None,
    });
    let opts = opencrs_fetch::DiscoverOptions {
        tokens: o.tokens.unwrap_or_default(),
        allow_local_http: o.unsafe_allow_local_http.unwrap_or(false),
        timeout: millis(o.timeout_ms),
        total_timeout: millis(o.total_timeout_ms),
        cancel: cancel_flag(token),
    };
    let now = o.now;
    spawn(env, move || {
        let now = parse_time(now.as_deref())?;
        let d = opencrs_fetch::discover(&domain, now, &opts).map_err(fail)?;
        if d.cancelled {
            return Err(fail("cancelled"));
        }
        to_value(&d.result)
    })
}
