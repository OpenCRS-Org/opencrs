//! Node.js bindings for the OpenCRS core. Every function is a thin layer
//! over `opencrs`: it takes and returns plain JS values (objects, not
//! JSON strings), and a core error becomes a thrown `Error` carrying the
//! core's message.

use chrono::{DateTime, FixedOffset};
use napi::bindgen_prelude::{AsyncTask, Either, Object, Uint8Array, Unknown};
use napi::bindgen_prelude::{ToNapiValue, TypeName};
use napi::{Env, Error, Result, Task, ValueType, sys};
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
    let now = parse_now(now.as_deref().filter(|n| !n.is_empty())).map_err(fail)?;
    let missing = missing
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or("not fetched");
    to_value(&check_publisher(&domain, &pages, missing, now))
}

/// Limits for `fetchPublisher`. A missing or zero number is the default.
#[napi(object)]
pub struct FetchPublisherOptions {
    /// The most one page may be, in bytes (default 50 MiB).
    pub max_document_bytes: Option<f64>,
    /// The most all pages may be together (default 180 MiB).
    pub max_total_bytes: Option<f64>,
    /// How many times the core is asked (default 5).
    pub max_rounds: Option<u32>,
    /// How many URLs are fetched in all (default 4000).
    pub max_pages: Option<u32>,
    /// For one request, redirects included (default 120,000).
    pub timeout_ms: Option<u32>,
    /// An RFC 3339 time to check at (default: now).
    pub now: Option<String>,
}

fn bytes(v: Option<f64>) -> usize {
    v.filter(|v| *v > 0.0).map_or(0, |v| v as usize)
}

pub struct FetchPublisherTask {
    domain: String,
    opts: opencrs_fetch::FetchOptions,
    now: DateTime<FixedOffset>,
}

#[napi]
impl Task for FetchPublisherTask {
    type Output = Value;
    type JsValue = Json;

    fn compute(&mut self) -> Result<Value> {
        let f = opencrs_fetch::fetch_publisher(&self.domain, self.now, &self.opts);
        if f.rounds_exhausted {
            return Err(fail(format!(
                "the publisher check still wanted URLs after its last round ({} rounds)",
                f.rounds
            )));
        }
        to_value(&f.check)
    }

    fn resolve(&mut self, _env: Env, output: Value) -> Result<Json> {
        Ok(Json(output))
    }
}

/// Runs the publisher check end to end: fetches the publisher's index and
/// the documents it lists, as the command line does (HTTPS only, no
/// non-public address, no proxy, size caps), and resolves with the same
/// result as `publisher`. Runs off the event loop.
#[napi(js_name = "fetchPublisher")]
pub fn fetch_publisher(
    domain: String,
    options: Option<FetchPublisherOptions>,
) -> Result<AsyncTask<FetchPublisherTask>> {
    let o = options.unwrap_or(FetchPublisherOptions {
        max_document_bytes: None,
        max_total_bytes: None,
        max_rounds: None,
        max_pages: None,
        timeout_ms: None,
        now: None,
    });
    let now = parse_now(o.now.as_deref().filter(|n| !n.is_empty())).map_err(fail)?;
    Ok(AsyncTask::new(FetchPublisherTask {
        domain,
        now,
        opts: opencrs_fetch::FetchOptions {
            max_document_bytes: bytes(o.max_document_bytes),
            max_total_bytes: bytes(o.max_total_bytes),
            max_rounds: o.max_rounds.unwrap_or(0) as usize,
            max_pages: o.max_pages.unwrap_or(0) as usize,
            timeout: Duration::from_millis(o.timeout_ms.unwrap_or(0) as u64),
        },
    }))
}

/// Options for `discover`.
#[napi(object)]
pub struct DiscoverOptions {
    /// Bearer tokens for partner documents, by publisher domain.
    pub tokens: Option<HashMap<String, String>>,
    /// Local testing: allow http:// and connections to localhost.
    pub allow_local_http: Option<bool>,
    /// An RFC 3339 time to judge expiry at (default: now).
    pub now: Option<String>,
}

pub struct DiscoverTask {
    domain: String,
    tokens: HashMap<String, String>,
    allow_local_http: bool,
    now: DateTime<FixedOffset>,
}

#[napi]
impl Task for DiscoverTask {
    type Output = Value;
    type JsValue = Json;

    fn compute(&mut self) -> Result<Value> {
        let r =
            opencrs_fetch::discover(&self.domain, &self.tokens, self.allow_local_http, self.now)
                .map_err(fail)?;
        to_value(&r)
    }

    fn resolve(&mut self, _env: Env, output: Value) -> Result<Json> {
        Ok(Json(output))
    }
}

/// Finds a publisher's specs and says which to trust, as `opencrs fetch
/// DOMAIN --json` does (its `result`). Rejects if the index itself can't be
/// fetched or trusted. Runs off the event loop.
#[napi]
pub fn discover(
    domain: String,
    options: Option<DiscoverOptions>,
) -> Result<AsyncTask<DiscoverTask>> {
    let (tokens, allow_local_http, now) = match options {
        Some(o) => (o.tokens, o.allow_local_http, o.now),
        None => (None, None, None),
    };
    Ok(AsyncTask::new(DiscoverTask {
        domain,
        tokens: tokens.unwrap_or_default(),
        allow_local_http: allow_local_http.unwrap_or(false),
        now: parse_now(now.as_deref().filter(|n| !n.is_empty())).map_err(fail)?,
    }))
}
