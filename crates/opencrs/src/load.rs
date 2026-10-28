//! Decoding documents: schema first, then the typed model.

use crate::error::Error;
use crate::model::{Document, Manifest};
use crate::schema::{Kind, SchemaOptions, validate_schema};

/// Parses a screens.json, first checking it against the schema.
pub fn decode_document(raw: &[u8]) -> Result<Document, Error> {
    let mut d: Document = decode(Kind::Screens, raw)?;
    d.size = raw.len();
    Ok(d)
}

/// Parses a creative.json, first checking it against the schema.
pub fn decode_manifest(raw: &[u8]) -> Result<Manifest, Error> {
    let mut m: Manifest = decode(Kind::Creative, raw)?;
    m.size = raw.len();
    Ok(m)
}

/// Checks `raw` against the schema for `kind`, as decoding does first: the
/// violations, if any, are the error.
pub(crate) fn check_schema(kind: Kind, raw: &[u8]) -> Result<(), Error> {
    let errs = validate_schema(kind, raw, &SchemaOptions::default())?;
    if !errs.is_empty() {
        let lines: Vec<String> = errs.iter().map(|e| format!("  {e}")).collect();
        return Err(Error::Message(format!(
            "not a valid OpenCRS {kind} document:\n{}",
            lines.join("\n")
        )));
    }
    Ok(())
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(kind: Kind, raw: &[u8]) -> Result<T, Error> {
    check_schema(kind, raw)?;
    // Text that is UTF-8 throughout (checked once, quickly) is read as
    // text, so each string in it isn't checked again.
    Ok(match std::str::from_utf8(raw) {
        Ok(text) => serde_json::from_str(text)?,
        Err(_) => serde_json::from_slice(raw)?,
    })
}
