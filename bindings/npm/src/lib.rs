//! Node.js bindings for the OpenCRS core. Every function is a thin layer
//! over `opencrs`: it takes and returns plain JS values (objects, not
//! JSON strings), and a core error becomes a thrown `Error` carrying the
//! core's message.

use napi::bindgen_prelude::Uint8Array;
use napi::{Error, Result};
use napi_derive::napi;
use opencrs::batch;
use opencrs::fs::{check_dir_text, probe_path, run_check};
use opencrs::load::decode_document;
use opencrs::probe::Probe;
use opencrs::probe::probe_bytes;
use opencrs::resolve::parse_ref;
use opencrs::schema::SchemaOptions;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

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
