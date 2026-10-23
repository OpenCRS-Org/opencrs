//! Python bindings for the OpenCRS core. Every function is a thin layer over
//! `opencrs`. Results cross the boundary as JSON text and
//! `python/opencrs/__init__.py` turns them into dicts and lists; a core error
//! becomes `opencrs.OpenCRSError` (a `ValueError`) carrying the core's message.

use opencrs::batch;
use opencrs::fs::{check_dir_text, probe_path, run_check};
use opencrs::load::decode_document;
use opencrs::probe::Probe;
use opencrs::probe::probe_bytes;
use opencrs::resolve::parse_ref;
use opencrs::schema::SchemaOptions;
use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde::Serialize;
use std::collections::HashMap;

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

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("OpenCRSError", m.py().get_type::<OpenCRSError>())?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(lint, m)?)?;
    m.add_function(wrap_pyfunction!(resolve, m)?)?;
    m.add_function(wrap_pyfunction!(probe, m)?)?;
    m.add_function(wrap_pyfunction!(probe_bytes_json, m)?)?;
    m.add_function(wrap_pyfunction!(check, m)?)?;
    m.add_function(wrap_pyfunction!(check_dir, m)?)?;
    Ok(())
}
