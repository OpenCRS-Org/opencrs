//! OpenCRS core: the document model, schema validation, resolution and checks.
//! No filesystem, network, clock or randomness; callers pass everything in.
//! The one exception is the opt-in `fs` feature (`fs` module) for hosts.

pub mod batch;
pub mod calc;
pub mod check;
pub mod checks;
pub mod conformance;
pub mod discover;
pub mod document;
pub mod error;
#[cfg(feature = "fs")]
pub mod fs;
pub mod gofmt;
pub mod json;
pub mod legibility;
pub mod load;
pub mod model;
pub mod packcheck;
pub mod probe;
pub mod report;
pub mod resolve;
pub mod rulepack;
pub mod schema;
#[cfg(test)]
mod testutil;

pub use error::Error;
pub use load::{decode_document, decode_manifest};
pub use model::*;
pub use schema::{Kind, SchemaError, SchemaOptions, detect_kind, validate_schema};

/// The spec version this crate implements.
pub const SPEC_VERSION: &str = "0.1";
