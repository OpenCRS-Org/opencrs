//! Python bindings for the OpenCRS core. Every function is a thin layer over
//! `opencrs`. Results cross the boundary as JSON text and
//! `python/opencrs/__init__.py` turns them into dicts and lists; a core error
//! becomes `opencrs.OpenCRSError` (a `ValueError`) carrying the core's message.

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
use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::marker::Ungil;
use pyo3::prelude::*;
use pyo3::pybacked::PyBackedBytes;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

create_exception!(
    _native,
    OpenCRSError,
    PyValueError,
    "A document, reference or file could not be read or understood."
);

fn fail(e: impl std::fmt::Display) -> PyErr {
    OpenCRSError::new_err(e.to_string())
}

fn to_json<T: Serialize>(v: &T) -> PyResult<String> {
    serde_json::to_string(v).map_err(fail)
}

#[pyfunction]
fn version() -> String {
    opencrs::SPEC_VERSION.to_string()
}

#[pyfunction]
fn lint(text: &str) -> PyResult<String> {
    let res = batch::lint(text.as_bytes(), None, &SchemaOptions::default()).map_err(fail)?;
    to_json(&res)
}

#[pyfunction]
fn resolve(spec: &str, r#ref: &str) -> PyResult<String> {
    let doc = decode_document(spec.as_bytes()).map_err(fail)?;
    let r = parse_ref(r#ref).map_err(fail)?;
    let reqs = doc.resolve(&r).map_err(fail)?;
    let views: Vec<_> = reqs.iter().map(|r| r.view()).collect();
    to_json(&views)
}

#[pyfunction]
fn resolve_strictest(spec: &str, r#ref: &str) -> PyResult<String> {
    let s = opencrs::budget::ensure(spec.len(), || Strictest::from_text(spec.as_bytes(), r#ref))
        .map_err(fail)?;
    String::from_utf8(s.to_json()).map_err(fail)
}

#[pyfunction]
fn probe(py: Python<'_>, path: &str) -> PyResult<String> {
    let p = py
        .detach(|| probe_path(std::path::Path::new(path)))
        .map_err(|e| fail(format!("{path}: {e}")))?;
    to_json(&p)
}

#[pyfunction]
fn probe_bytes_json(data: &[u8]) -> PyResult<String> {
    to_json(&probe_bytes(data))
}

#[pyfunction]
#[pyo3(signature = (specs, manifest, probes, now=None))]
fn check(specs: Vec<String>, manifest: &str, probes: &str, now: Option<&str>) -> PyResult<String> {
    let probes: HashMap<String, Probe> = serde_json::from_str(probes).map_err(fail)?;
    to_json(&run_check(&specs, manifest, &probes, now).map_err(fail)?)
}

#[pyfunction]
#[pyo3(signature = (dir, specs, manifest, now=None))]
fn check_dir(
    py: Python<'_>,
    dir: &str,
    specs: Vec<String>,
    manifest: &str,
    now: Option<&str>,
) -> PyResult<String> {
    let res = py
        .detach(|| check_dir_text(dir, &specs, manifest, now))
        .map_err(fail)?;
    to_json(&res)
}

/// Serialised as the WebAssembly module serialises its replies, so the
/// text is what `js/opencrs.js` returns for the same call.
fn reply<T: Serialize>(v: &T) -> PyResult<String> {
    let v = serde_json::to_value(v).map_err(fail)?;
    Ok(opencrs::json::to_string(&v))
}

fn screen(mm: Option<f64>, px: Option<i64>) -> PyResult<Option<ScreenWidth>> {
    match (mm, px) {
        (Some(mm), Some(px)) => Ok(Some(ScreenWidth { mm, px })),
        (None, None) => Ok(None),
        _ => Err(fail("screen needs both width_mm and width_px")),
    }
}

#[pyfunction]
#[pyo3(signature = (near, far, ratio=None, screen_mm=None, screen_px=None, dwell_s=None, rate=None))]
fn legibility(
    near: f64,
    far: f64,
    ratio: Option<f64>,
    screen_mm: Option<f64>,
    screen_px: Option<i64>,
    dwell_s: Option<f64>,
    rate: Option<f64>,
) -> PyResult<String> {
    let s = screen(screen_mm, screen_px)?;
    reply(&calc::legibility([near, far], ratio, s, dwell_s, rate).map_err(fail)?)
}

#[pyfunction]
#[pyo3(signature = (speed_kmh, near, far, ratio=None, rate=None, screen_mm=None, screen_px=None, rule_packs=Vec::new()))]
#[allow(clippy::too_many_arguments)]
fn dwell(
    speed_kmh: f64,
    near: f64,
    far: f64,
    ratio: Option<f64>,
    rate: Option<f64>,
    screen_mm: Option<f64>,
    screen_px: Option<i64>,
    rule_packs: Vec<String>,
) -> PyResult<String> {
    let s = screen(screen_mm, screen_px)?;
    reply(&calc::dwell(speed_kmh, [near, far], ratio, rate, s, &rule_packs).map_err(fail)?)
}

#[pyfunction]
fn rule_packs() -> PyResult<String> {
    reply(&serde_json::json!({ "rule_packs": calc::pack_list() }))
}

/// A page's headers, by the URL it was fetched from; its body is the item of
/// `bodies` at the same position.
#[derive(Deserialize)]
struct PageEntry {
    url: String,
    #[serde(flatten)]
    page: Page<'static>,
}

#[pyfunction]
#[pyo3(signature = (domain, pages, bodies, now=None, missing=None))]
fn publisher(
    domain: &str,
    pages: &str,
    bodies: Vec<PyBackedBytes>,
    now: Option<&str>,
    missing: Option<&str>,
) -> PyResult<String> {
    let heads: Vec<PageEntry> = serde_json::from_str(pages).map_err(fail)?;
    if heads.len() != bodies.len() {
        return Err(fail("publisher needs one body for each page, in order"));
    }
    // A page borrows its own body, which stays where it arrived.
    let pages: BTreeMap<String, Page<'_>> = heads
        .into_iter()
        .zip(&bodies)
        .map(|(e, body)| {
            (
                e.url,
                Page {
                    body: body.as_ref(),
                    ..e.page
                },
            )
        })
        .collect();
    let now = parse_now(now.filter(|n| !n.is_empty())).map_err(fail)?;
    let missing = missing.filter(|m| !m.is_empty()).unwrap_or("not fetched");
    reply(&check_publisher(domain, &pages, missing, now))
}

/// A number of seconds from Python: a finite number, zero or more. One past
/// an hour is an hour.
fn seconds(name: &str, v: f64) -> PyResult<Duration> {
    if !v.is_finite() || v < 0.0 {
        return Err(fail(format!(
            "{name} must be a number of seconds, zero or more"
        )));
    }
    Duration::try_from_secs_f64(v.min(3600.0)).map_err(fail)
}

/// A count or size from Python: zero or more. One too big for a machine word
/// is the largest, and the fetching clamps it to what it allows.
fn size(name: &str, v: i128) -> PyResult<usize> {
    if v < 0 {
        return Err(fail(format!("{name} must be zero or more")));
    }
    Ok(usize::try_from(v).unwrap_or(usize::MAX))
}

/// Runs `work` with the GIL released, and turns a panic in it into an
/// ordinary error.
fn detached<T: Ungil + Send>(
    py: Python<'_>,
    work: impl FnOnce() -> T + Ungil + Send,
) -> PyResult<T> {
    py.detach(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)))
        .map_err(|_| PyRuntimeError::new_err("internal error: the check panicked"))
}

#[pyfunction]
#[pyo3(signature = (domain, max_document_bytes=0, max_total_bytes=0, max_rounds=0, max_pages=0, timeout_s=0.0, total_timeout_s=0.0, now=None))]
#[allow(clippy::too_many_arguments)]
fn fetch_publisher(
    py: Python<'_>,
    domain: &str,
    max_document_bytes: i128,
    max_total_bytes: i128,
    max_rounds: i128,
    max_pages: i128,
    timeout_s: f64,
    total_timeout_s: f64,
    now: Option<&str>,
) -> PyResult<String> {
    let now = parse_now(now.filter(|n| !n.is_empty())).map_err(fail)?;
    let opts = opencrs_fetch::FetchOptions {
        max_document_bytes: size("max_document_bytes", max_document_bytes)?,
        max_total_bytes: size("max_total_bytes", max_total_bytes)?,
        max_rounds: size("max_rounds", max_rounds)?,
        max_pages: size("max_pages", max_pages)?,
        timeout: seconds("timeout", timeout_s)?,
        total_timeout: seconds("total_timeout", total_timeout_s)?,
        // Nothing can reach a call that has let go of the GIL, so there is
        // nothing to cancel it with: the total time limit is what bounds it.
        cancel: None,
    };
    // The network waits happen with the GIL released.
    let f = detached(py, || opencrs_fetch::fetch_publisher(domain, now, &opts))?;
    if f.rounds_exhausted {
        return Err(fail(format!(
            "the publisher check still wanted URLs after its last round ({} rounds)",
            f.rounds
        )));
    }
    reply(&f.check)
}

#[pyfunction]
#[pyo3(signature = (domain, tokens=HashMap::new(), unsafe_allow_local_http=false, timeout_s=0.0, total_timeout_s=0.0, now=None))]
fn discover(
    py: Python<'_>,
    domain: &str,
    tokens: HashMap<String, String>,
    unsafe_allow_local_http: bool,
    timeout_s: f64,
    total_timeout_s: f64,
    now: Option<&str>,
) -> PyResult<String> {
    let now = parse_now(now.filter(|n| !n.is_empty())).map_err(fail)?;
    let opts = opencrs_fetch::DiscoverOptions {
        tokens,
        allow_local_http: unsafe_allow_local_http,
        timeout: seconds("timeout", timeout_s)?,
        total_timeout: seconds("total_timeout", total_timeout_s)?,
        cancel: None,
    };
    let d = detached(py, || opencrs_fetch::discover(domain, now, &opts))?.map_err(fail)?;
    reply(&d.result)
}
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("OpenCRSError", m.py().get_type::<OpenCRSError>())?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(lint, m)?)?;
    m.add_function(wrap_pyfunction!(resolve, m)?)?;
    m.add_function(wrap_pyfunction!(resolve_strictest, m)?)?;
    m.add_function(wrap_pyfunction!(probe, m)?)?;
    m.add_function(wrap_pyfunction!(probe_bytes_json, m)?)?;
    m.add_function(wrap_pyfunction!(check, m)?)?;
    m.add_function(wrap_pyfunction!(check_dir, m)?)?;
    m.add_function(wrap_pyfunction!(legibility, m)?)?;
    m.add_function(wrap_pyfunction!(dwell, m)?)?;
    m.add_function(wrap_pyfunction!(rule_packs, m)?)?;
    m.add_function(wrap_pyfunction!(publisher, m)?)?;
    m.add_function(wrap_pyfunction!(fetch_publisher, m)?)?;
    m.add_function(wrap_pyfunction!(discover, m)?)?;
    Ok(())
}
