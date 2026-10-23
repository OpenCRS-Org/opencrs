//! JSON Schema validation of OpenCRS documents, ported from `crs/schema.go`.

use crate::error::Error;
use jsonschema::error::ValidationErrorKind;
use jsonschema::{ErrorIterator, Registry, Validator};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

const BASE_URL: &str = "https://opencrs.dev/schema/0.1/";

const SCHEMA_FILES: [(&str, &str); 6] = [
    (
        "common.schema.json",
        include_str!("../../../schema/0.1/common.schema.json"),
    ),
    (
        "creative.schema.json",
        include_str!("../../../schema/0.1/creative.schema.json"),
    ),
    (
        "index.schema.json",
        include_str!("../../../schema/0.1/index.schema.json"),
    ),
    (
        "report.schema.json",
        include_str!("../../../schema/0.1/report.schema.json"),
    ),
    (
        "rule-pack.schema.json",
        include_str!("../../../schema/0.1/rule-pack.schema.json"),
    ),
    (
        "screens.schema.json",
        include_str!("../../../schema/0.1/screens.schema.json"),
    ),
];

/// One of the OpenCRS document types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Index,
    Screens,
    Creative,
    Report,
    RulePack,
}

impl Kind {
    const ALL: [Kind; 5] = [
        Kind::Index,
        Kind::Screens,
        Kind::Creative,
        Kind::Report,
        Kind::RulePack,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Index => "index",
            Kind::Screens => "screens",
            Kind::Creative => "creative",
            Kind::Report => "report",
            Kind::RulePack => "rule-pack",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Kind {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        Kind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| {
                Error::Message(format!("unknown document kind {}", crate::gofmt::quote(s)))
            })
    }
}

type Compiled = Vec<(Kind, Validator)>;

fn schemas() -> Result<&'static Compiled, Error> {
    static COMPILED: OnceLock<Result<Compiled, String>> = OnceLock::new();
    COMPILED
        .get_or_init(compile)
        .as_ref()
        .map_err(|e| Error::Message(format!("loading schemas: {e}")))
}

fn compile() -> Result<Compiled, String> {
    let mut builder = Registry::new();
    let mut docs = Vec::new();
    for (name, text) in SCHEMA_FILES {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("{name}: {e}"))?;
        builder = builder
            .add(format!("{BASE_URL}{name}"), v.clone())
            .map_err(|e| e.to_string())?;
        docs.push((name, v));
    }
    let registry = builder.prepare().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let name = format!("{}.schema.json", kind.as_str());
        let (_, doc) = docs
            .iter()
            .find(|(n, _)| *n == name)
            .ok_or("missing schema")?;
        let v = jsonschema::options()
            .with_registry(&registry)
            .build(doc)
            .map_err(|e| e.to_string())?;
        out.push((kind, v));
    }
    Ok(out)
}

/// Guesses a document's kind from its top-level keys. For a JSON array (such
/// as the CLI's list of reports) it uses the first element.
pub fn detect_kind(raw: &[u8]) -> Result<Kind, Error> {
    let mut v: Value = serde_json::from_slice(raw)?;
    if let Some(first) = v.as_array().and_then(|a| a.first()) {
        v = first.clone();
    }
    let Some(m) = v.as_object() else {
        return Err(Error::msg("document is not a JSON object"));
    };
    let has = |k: &str| m.contains_key(k);
    if has("rule_pack") {
        Ok(Kind::RulePack)
    } else if has("networks") {
        Ok(Kind::Screens)
    } else if has("documents") {
        Ok(Kind::Index)
    } else if has("files") {
        Ok(Kind::Creative)
    } else if has("checks") {
        Ok(Kind::Report)
    } else {
        Err(Error::msg("can't tell which OpenCRS document this is"))
    }
}

/// One place a document breaks its schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchemaError {
    /// JSON pointer into the document.
    pub path: String,
    pub message: String,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = if self.path.is_empty() {
            "/"
        } else {
            &self.path
        };
        write!(f, "{p}: {}", self.message)
    }
}

/// Options that loosen schema validation for particular uses.
#[derive(Debug, Clone, Copy, Default)]
pub struct SchemaOptions {
    /// Accept a JSON array whose every element is a document of the kind,
    /// such as the reports `opencrs check -json` prints.
    pub list: bool,
    /// Accept http://localhost (and 127.0.0.1, [::1]) document URLs in an
    /// index, so publishers can test before deploying. Published indexes
    /// must use HTTPS.
    pub allow_local_http: bool,
}

/// Checks raw JSON against the schema for `kind`. Returns the violations, or
/// an error if the JSON can't be read at all.
pub fn validate_schema(
    kind: Kind,
    raw: &[u8],
    opt: &SchemaOptions,
) -> Result<Vec<SchemaError>, Error> {
    let all = schemas()?;
    let validator = &all
        .iter()
        .find(|(k, _)| *k == kind)
        .ok_or_else(|| Error::msg("unknown document kind"))?
        .1;
    let v: Value = serde_json::from_slice(raw)?;
    let prepare = |mut v: Value| {
        if kind == Kind::Index && opt.allow_local_http {
            secure_local_urls(&mut v);
        }
        v
    };
    match v {
        Value::Array(items) if opt.list => {
            let mut out = Vec::new();
            for (i, el) in items.into_iter().enumerate() {
                for mut e in validate_value(validator, &prepare(el)) {
                    e.path = format!("/{i}{}", e.path);
                    out.push(e);
                }
            }
            Ok(out)
        }
        v => Ok(validate_value(validator, &prepare(v))),
    }
}

/// Whether `u` is an http:// URL on a local host, optionally with a port.
fn is_local_http(u: &str) -> bool {
    let Some(rest) = u.strip_prefix("http://") else {
        return false;
    };
    let Some(rest) = ["localhost", "127.0.0.1", "[::1]"]
        .iter()
        .find_map(|h| rest.strip_prefix(h))
    else {
        return false;
    };
    let rest = match rest.strip_prefix(':') {
        Some(after) => {
            let digits = after.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                return false;
            }
            &after[digits..]
        }
        None => rest,
    };
    rest.is_empty() || rest.starts_with('/')
}

/// Rewrites local http:// URLs in an index to https:// so they pass the
/// HTTPS rule. Only the parsed copy is changed.
fn secure_local_urls(v: &mut Value) {
    fn fix(m: &mut serde_json::Map<String, Value>, key: &str) {
        if let Some(Value::String(u)) = m.get_mut(key)
            && is_local_http(u)
        {
            *u = format!("https://{}", &u["http://".len()..]);
        }
    }
    let Some(idx) = v.as_object_mut() else {
        return;
    };
    fix(idx, "changes_url");
    if let Some(Value::Array(docs)) = idx.get_mut("documents") {
        for d in docs {
            if let Some(m) = d.as_object_mut() {
                fix(m, "url");
            }
        }
    }
}

fn validate_value(validator: &Validator, v: &Value) -> Vec<SchemaError> {
    let errors: ErrorIterator<'_> = validator.iter_errors(v);
    let found: Vec<(String, String, bool)> = errors
        .map(|e| {
            let branching = matches!(
                e.kind(),
                ValidationErrorKind::OneOfNotValid { .. }
                    | ValidationErrorKind::OneOfMultipleValid { .. }
                    | ValidationErrorKind::AnyOf { .. }
            );
            (e.instance_path().to_string(), e.to_string(), branching)
        })
        .collect();
    drop_under_branches(found)
}

/// A failing oneOf/anyOf is reported once, at its own node: drops anything
/// the library reports beneath it, then de-duplicates and sorts. Each
/// error's ancestors are looked up in a set, so this is linear in the total
/// length of the paths, not quadratic in the number of errors.
fn drop_under_branches(found: Vec<(String, String, bool)>) -> Vec<SchemaError> {
    let branch_paths: HashSet<&str> = found.iter().filter(|e| e.2).map(|e| e.0.as_str()).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (path, message, _) in &found {
        // Every proper ancestor of `path` ends just before one of its '/'s;
        // the first gives "", the root.
        let under_branch = !branch_paths.is_empty()
            && path
                .bytes()
                .enumerate()
                .any(|(i, b)| b == b'/' && branch_paths.contains(&path[..i]));
        if under_branch {
            continue;
        }
        let se = SchemaError {
            path: path.clone(),
            message: message.clone(),
        };
        if seen.insert(se.to_string()) {
            out.push(se);
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The filter as first written: each error against every branch.
    fn drop_under_branches_quadratic(found: Vec<(String, String, bool)>) -> Vec<SchemaError> {
        let branch_paths: Vec<&str> = found.iter().filter(|e| e.2).map(|e| e.0.as_str()).collect();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (path, message, _) in &found {
            if branch_paths.iter().any(|b| {
                path.len() > b.len() && path.starts_with(b) && path.as_bytes()[b.len()] == b'/'
            }) {
                continue;
            }
            let se = SchemaError {
                path: path.clone(),
                message: message.clone(),
            };
            if seen.insert(se.to_string()) {
                out.push(se);
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    #[test]
    fn branch_filter_matches_the_quadratic_one() {
        let paths = [
            "", "/a", "/a/b", "/a/b/c", "/ab", "/ab/c", "/a/bc", "/x/0", "/x/0/y", "/x/01",
        ];
        // Every subset of paths marked as branches, with errors at all paths.
        for mask in 0u32..(1 << paths.len()) {
            let found: Vec<(String, String, bool)> = paths
                .iter()
                .enumerate()
                .flat_map(|(i, p)| {
                    [
                        (p.to_string(), "m".to_string(), mask & (1 << i) != 0),
                        (p.to_string(), "m".to_string(), false),
                    ]
                })
                .collect();
            assert_eq!(
                drop_under_branches(found.clone()),
                drop_under_branches_quadratic(found),
                "mask {mask:b}"
            );
        }
    }

    /// A document with many failing anyOf nodes (the review's M5) took
    /// minutes when each error was compared with every branch.
    #[test]
    fn branch_filter_is_not_quadratic() {
        const N: usize = 200_000;
        let mut found = Vec::with_capacity(2 * N);
        for i in 0..N {
            found.push((format!("/formats/{i}/media"), "anyOf".to_string(), true));
            found.push((
                format!("/formats/{i}/media/kind"),
                "below".to_string(),
                false,
            ));
        }
        let out = drop_under_branches(found);
        assert_eq!(out.len(), N);
        assert!(out.iter().all(|e| e.path.ends_with("/media")));
    }
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn read(rel: &str) -> Vec<u8> {
        std::fs::read(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    fn check(kind: Kind, doc: &str) -> Vec<SchemaError> {
        validate_schema(kind, doc.as_bytes(), &SchemaOptions::default()).unwrap()
    }

    #[test]
    fn schema_rejects_bad_documents() {
        let cases = [
            (
                "missing networks",
                Kind::Screens,
                r#"{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-10-20T09:00:00+11:00"}"#,
            ),
            (
                "date without zone",
                Kind::Screens,
                r#"{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-10-20T09:00:00","networks":[]}"#,
            ),
            (
                "major version 1",
                Kind::Screens,
                r#"{"opencrs":"1.0","publisher":"a.example","updated_at":"2026-10-20T09:00:00Z","networks":[]}"#,
            ),
            (
                "format without media",
                Kind::Screens,
                r#"{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-10-20T09:00:00Z","networks":[{"id":"n","name":"N","formats":[{"id":"f","pixels":[1,1]}],"screens":[]}]}"#,
            ),
            (
                "bad target ref",
                Kind::Creative,
                r#"{"opencrs":"0.1","files":[{"file":"a.png","media_type":"image/png","targets":["portrait-totem-75"]}]}"#,
            ),
            (
                "http document url",
                Kind::Index,
                r#"{"opencrs":"0.1","publisher":{"id":"a.example","name":"A"},"documents":[{"url":"http://a.example/x.json"}]}"#,
            ),
        ];
        for (name, kind, doc) in cases {
            assert!(
                !check(kind, doc).is_empty(),
                "{name}: expected schema errors, got none"
            );
        }
    }

    #[test]
    fn local_http_only_with_flag() {
        let doc = br#"{"opencrs":"0.1","publisher":{"id":"a.example","name":"A"},"documents":[{"url":"http://localhost:8080/retail.screens.json"}]}"#;
        let off = SchemaOptions::default();
        let on = SchemaOptions {
            allow_local_http: true,
            ..Default::default()
        };
        assert!(
            !validate_schema(Kind::Index, doc, &off).unwrap().is_empty(),
            "http://localhost must fail without the flag"
        );
        let errs = validate_schema(Kind::Index, doc, &on).unwrap();
        assert!(
            errs.is_empty(),
            "http://localhost should pass with the flag: {errs:?}"
        );
        let remote = br#"{"opencrs":"0.1","publisher":{"id":"a.example","name":"A"},"documents":[{"url":"http://localhost.evil.example/x.json"}]}"#;
        assert!(
            !validate_schema(Kind::Index, remote, &on)
                .unwrap()
                .is_empty(),
            "the flag must only allow local hosts"
        );
    }

    // A 0.1 reader must accept later minor versions and fields it doesn't know.
    #[test]
    fn schema_is_forward_compatible() {
        let doc = r#"{"opencrs":"0.2","publisher":"a.example","updated_at":"2026-10-20T09:00:00Z","new_field":1,
          "networks":[{"id":"n","name":"N","future":true,"formats":[{"id":"f","pixels":[1,1],"media":{"static":["image/png"]},"hdr":"pq"}],"screens":[]}]}"#;
        let errs = check(Kind::Screens, doc);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn schema_paths_collapse_under_one_of() {
        // placement.facing is oneOf [compass enum, number]; true fits neither.
        let doc = r#"{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-10-20T09:00:00Z",
          "networks":[{"id":"n","name":"N","formats":[{"id":"f","pixels":[1,1],"media":{"static":["image/png"]}}],
          "screens":[{"id":"s","format":"f","placement":{"facing":true}}]}]}"#;
        let errs = check(Kind::Screens, doc);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(errs[0].path, "/networks/0/screens/0/placement/facing");
    }

    #[test]
    fn kind_names_round_trip() {
        for k in Kind::ALL {
            assert_eq!(k.as_str().parse::<Kind>().unwrap(), k);
        }
        assert_eq!(Kind::RulePack.as_str(), "rule-pack");
        assert!("nope".parse::<Kind>().is_err());
    }

    #[test]
    fn schema_error_displays_root_as_slash() {
        let e = SchemaError {
            path: String::new(),
            message: "bad".into(),
        };
        assert_eq!(e.to_string(), "/: bad");
    }

    #[test]
    fn decode_reports_schema_errors() {
        let err = crate::decode_document(br#"{"opencrs":"0.1"}"#)
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("not a valid OpenCRS screens document:\n  /: "),
            "{err}"
        );
        let d = crate::decode_document(&read("examples/harbourline/retail.screens.json")).unwrap();
        assert!(!d.source.is_empty() && !d.networks.is_empty());
        crate::decode_manifest(&read("examples/harbourline/creative.json")).unwrap();
    }
}
