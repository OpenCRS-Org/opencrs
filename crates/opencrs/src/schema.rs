//! JSON Schema validation of OpenCRS documents, ported from `crs/schema.go`.

use crate::error::Error;
use jsonschema::error::ValidationErrorKind;
use jsonschema::{ErrorIterator, Registry, Validator};
use serde::Serialize;
use serde_json::Value;
use serde_json::value::RawValue;
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

/// A screens.json's three big arrays, each validated an item at a time.
struct Items {
    venue: Validator,
    format: Validator,
    screen: Validator,
}

/// The schemas, parsed and registered once. A validator is built the first
/// time its kind is checked, so a process (or a fresh module instance) that
/// lints one kind of document doesn't pay for compiling them all.
struct Compiled {
    registry: Registry<'static>,
    docs: Vec<(&'static str, Value)>,
    kinds: Vec<(Kind, OnceLock<Result<Validator, String>>)>,
    items: OnceLock<Result<Items, String>>,
}

impl Compiled {
    fn validator(&self, kind: Kind) -> Result<&Validator, Error> {
        let (_, cell) = self
            .kinds
            .iter()
            .find(|(k, _)| *k == kind)
            .ok_or_else(|| Error::msg("unknown document kind"))?;
        cell.get_or_init(|| {
            let name = format!("{}.schema.json", kind.as_str());
            let (_, doc) = self
                .docs
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or("missing schema")?;
            jsonschema::options()
                .with_registry(&self.registry)
                .build(doc)
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| Error::Message(format!("loading schemas: {e}")))
    }

    fn items(&self) -> Result<&Items, Error> {
        self.items
            .get_or_init(|| {
                let item = |def: &str| {
                    jsonschema::options()
                        .with_registry(&self.registry)
                        .build(&serde_json::json!({
                            "$ref": format!("{BASE_URL}screens.schema.json#/$defs/{def}")
                        }))
                        .map_err(|e| e.to_string())
                };
                Ok(Items {
                    venue: item("venue")?,
                    format: item("format")?,
                    screen: item("screen")?,
                })
            })
            .as_ref()
            .map_err(|e| Error::Message(format!("loading schemas: {e}")))
    }
}

fn schemas() -> Result<&'static Compiled, Error> {
    static COMPILED: OnceLock<Result<Compiled, String>> = OnceLock::new();
    COMPILED
        .get_or_init(register)
        .as_ref()
        .map_err(|e| Error::Message(format!("loading schemas: {e}")))
}

fn register() -> Result<Compiled, String> {
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
    Ok(Compiled {
        registry,
        docs,
        kinds: Kind::ALL.iter().map(|&k| (k, OnceLock::new())).collect(),
        items: OnceLock::new(),
    })
}

/// Guesses a document's kind from its top-level keys. For a JSON array (such
/// as the CLI's list of reports) it uses the first element.
pub fn detect_kind(raw: &[u8]) -> Result<Kind, Error> {
    // The top-level keys are all that's needed, so read them without
    // building the document: its values stay unparsed slices of `raw`.
    let text = std::str::from_utf8(raw).ok();
    let keys = match text {
        Some(t) => serde_json::from_str::<RawMap>(t).ok(),
        None => serde_json::from_slice::<RawMap>(raw).ok(),
    }
    .map(|m| m.into_keys().collect::<Vec<_>>())
    .or_else(|| {
        let list = match text {
            Some(t) => serde_json::from_str::<Vec<&RawValue>>(t).ok()?,
            None => serde_json::from_slice::<Vec<&RawValue>>(raw).ok()?,
        };
        let m: RawMap = serde_json::from_str(list.first()?.get()).ok()?;
        Some(m.into_keys().collect())
    });
    if let Some(keys) = keys {
        return kind_of(|k| keys.iter().any(|x| x == k));
    }
    let mut v: Value = parse_value(raw)?;
    if let Some(first) = v.as_array().and_then(|a| a.first()) {
        v = first.clone();
    }
    let Some(m) = v.as_object() else {
        return Err(Error::msg("document is not a JSON object"));
    };
    kind_of(|k| m.contains_key(k))
}

fn kind_of(has: impl Fn(&str) -> bool) -> Result<Kind, Error> {
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
    validate_detecting(Some(kind), raw, opt).map(|(_, errs)| errs)
}

/// [`validate_schema`], telling the document's kind first when none is
/// given. A screens document (the big kind) is told apart and validated in
/// one reading of its text: its top-level keys come out of that reading,
/// so it isn't scanned once more to find them.
pub(crate) fn validate_detecting(
    kind: Option<Kind>,
    raw: &[u8],
    opt: &SchemaOptions,
) -> Result<(Kind, Vec<SchemaError>), Error> {
    let all = schemas()?;
    let by_item = if kind.is_none_or(|k| k == Kind::Screens) {
        validate_screens_by_item(all.items()?, all.validator(Kind::Screens)?, raw)
    } else {
        None
    };
    if let Some((errs, is_screens)) = by_item
        && (kind.is_some() || is_screens)
    {
        return Ok((Kind::Screens, errs));
    }
    // Stopped by the host part way: not a document the by-item reading
    // doesn't suit, so don't read it again.
    if crate::budget::interrupted() {
        return Err(Error::msg(crate::budget::INTERRUPTED_CODE));
    }
    let kind = match kind {
        Some(k) => k,
        None => detect_kind(raw)?,
    };
    let validator = all.validator(kind)?;
    let v: Value = parse_value(raw)?;
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
            Ok((kind, out))
        }
        v => Ok((kind, validate_value(validator, &prepare(v)))),
    }
}

/// Parses `raw` as a `Value`. Text that is UTF-8 throughout (checked once,
/// quickly) is read as text, so each string in it isn't checked again.
fn parse_value(raw: &[u8]) -> Result<Value, serde_json::Error> {
    match std::str::from_utf8(raw) {
        Ok(text) => serde_json::from_str(text),
        Err(_) => serde_json::from_slice(raw),
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
    let mut found = Vec::new();
    collect_errors(validator, v, "", &mut found);
    drop_under_branches(found)
}

/// Adds every violation of `v` to `found` as (path, message, whether it is
/// a failed oneOf/anyOf), with `prefix` put before each path.
fn collect_errors(
    validator: &Validator,
    v: &Value,
    prefix: &str,
    found: &mut Vec<(String, String, bool)>,
) {
    let errors: ErrorIterator<'_> = validator.iter_errors(v);
    found.extend(errors.map(|e| {
        let branching = matches!(
            e.kind(),
            ValidationErrorKind::OneOfNotValid { .. }
                | ValidationErrorKind::OneOfMultipleValid { .. }
                | ValidationErrorKind::AnyOf { .. }
        );
        (
            format!("{prefix}{}", e.instance_path()),
            e.to_string(),
            branching,
        )
    }));
}

type RawMap<'a> = std::collections::BTreeMap<String, &'a RawValue>;

/// Validates a screens.json without ever holding the whole document as a
/// `Value`, which for a document of hundreds of thousands of screens is
/// several times the size of the text. The venues, formats and screens of
/// each network are arrays whose items the schema checks independently
/// (nothing in it looks at their count or compares them), so each is
/// validated alone, as it is read, and the rest of the document, with those
/// arrays emptied, is validated as a whole. The errors, paths and order are
/// those of validating the whole document, and the text is read once.
///
/// None means the document isn't shaped for this (or isn't valid JSON), and
/// the caller validates it the ordinary way, which also gives the usual
/// parse error. With the errors comes whether the document's top-level keys
/// say it is a screens document at all (see [`detect_kind`]).
fn validate_screens_by_item(
    items: &Items,
    validator: &Validator,
    raw: &[u8],
) -> Option<(Vec<SchemaError>, bool)> {
    let mut found = Vec::new();
    let skeleton = {
        let seed = by_item::Root {
            items,
            found: &mut found,
        };
        match std::str::from_utf8(raw) {
            Ok(text) => {
                let mut de = serde_json::Deserializer::from_str(text);
                let v = serde::de::DeserializeSeed::deserialize(seed, &mut de).ok()?;
                de.end().ok()?;
                v
            }
            Err(_) => {
                let mut de = serde_json::Deserializer::from_slice(raw);
                let v = serde::de::DeserializeSeed::deserialize(seed, &mut de).ok()?;
                de.end().ok()?;
                v
            }
        }
    };
    let is_screens = skeleton
        .as_object()
        .is_some_and(|m| matches!(kind_of(|k| m.contains_key(k)), Ok(Kind::Screens)));
    collect_errors(validator, &skeleton, "", &mut found);
    Some((drop_under_branches(found), is_screens))
}

/// The seeded visitors of [`validate_screens_by_item`]: the document is
/// read into a `Value` with each network's venue, format and screen lists
/// left empty, their items validated and dropped as they go by.
mod by_item {
    use super::{Items, collect_errors};
    use jsonschema::Validator;
    use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
    use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
    use serde_json::Value;
    use std::fmt;

    type Found = Vec<(String, String, bool)>;

    /// The document: an object, or it isn't shaped for this.
    pub(super) struct Root<'s> {
        pub(super) items: &'s Items,
        pub(super) found: &'s mut Found,
    }

    impl<'de> DeserializeSeed<'de> for Root<'_> {
        type Value = Value;
        fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
            de.deserialize_map(self)
        }
    }

    impl<'de> Visitor<'de> for Root<'_> {
        type Value = Value;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a document")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
            let mut obj = serde_json::Map::new();
            // A repeated key keeps its last value, as a `Value` does: the
            // errors found in an earlier value of `networks` are dropped.
            let mut networks_errors: Option<std::ops::Range<usize>> = None;
            while let Some(key) = map.next_key::<String>()? {
                let value = if key == "networks" {
                    if let Some(r) = networks_errors.take() {
                        self.found.drain(r);
                    }
                    let start = self.found.len();
                    let v = map.next_value_seed(Networks {
                        items: self.items,
                        found: self.found,
                    })?;
                    networks_errors = Some(start..self.found.len());
                    v
                } else {
                    map.next_value::<Value>()?
                };
                obj.insert(key, value);
            }
            Ok(Value::Object(obj))
        }
    }

    /// The `networks` value: a list of networks, or whatever it is.
    struct Networks<'s> {
        items: &'s Items,
        found: &'s mut Found,
    }

    impl<'de> DeserializeSeed<'de> for Networks<'_> {
        type Value = Value;
        fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
            de.deserialize_any(self)
        }
    }

    impl<'de> Visitor<'de> for Networks<'_> {
        type Value = Value;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("networks")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut shells = Vec::new();
            while let Some(shell) = seq.next_element_seed(Network {
                items: self.items,
                found: self.found,
                n: shells.len(),
            })? {
                shells.push(shell);
            }
            Ok(Value::Array(shells))
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Value, A::Error> {
            any_map(map)
        }
        scalar_visits!();
    }

    /// One network: an object whose three item lists are validated and
    /// emptied, or whatever it is.
    struct Network<'s> {
        items: &'s Items,
        found: &'s mut Found,
        n: usize,
    }

    impl<'de> DeserializeSeed<'de> for Network<'_> {
        type Value = Value;
        fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
            de.deserialize_any(self)
        }
    }

    impl<'de> Visitor<'de> for Network<'_> {
        type Value = Value;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a network")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
            let mut obj = serde_json::Map::new();
            // Per list, where its errors are, so a repeat of the key (whose
            // value wins) drops the earlier ones.
            let mut errors: [Option<std::ops::Range<usize>>; 3] = [None, None, None];
            while let Some(key) = map.next_key::<String>()? {
                let which = ["venues", "formats", "screens"]
                    .iter()
                    .position(|k| *k == key);
                let value = match which {
                    Some(w) => {
                        if let Some(r) = errors[w].take() {
                            self.found.drain(r);
                        }
                        let validator = match w {
                            0 => &self.items.venue,
                            1 => &self.items.format,
                            _ => &self.items.screen,
                        };
                        let start = self.found.len();
                        let v = map.next_value_seed(List {
                            validator,
                            found: self.found,
                            prefix: format!("/networks/{}/{key}", self.n),
                        })?;
                        errors[w] = Some(start..self.found.len());
                        v
                    }
                    None => map.next_value::<Value>()?,
                };
                obj.insert(key, value);
            }
            Ok(Value::Object(obj))
        }
        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Value, A::Error> {
            any_seq(seq)
        }
        scalar_visits!();
    }

    /// A list of items, each validated as it is read and then dropped, so
    /// the list comes out empty; or whatever the value is.
    struct List<'s> {
        validator: &'s Validator,
        found: &'s mut Found,
        prefix: String,
    }

    impl<'de> DeserializeSeed<'de> for List<'_> {
        type Value = Value;
        fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
            de.deserialize_any(self)
        }
    }

    impl<'de> Visitor<'de> for List<'_> {
        type Value = Value;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a list")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut i = 0usize;
            let mut path = self.prefix.clone();
            while let Some(v) = seq.next_element::<Value>()? {
                if crate::budget::checkpoint() {
                    return Err(serde::de::Error::custom(crate::budget::INTERRUPTED_CODE));
                }
                path.truncate(self.prefix.len());
                path.push('/');
                path.push_str(&i.to_string());
                collect_errors(self.validator, &v, &path, self.found);
                i += 1;
            }
            Ok(Value::Array(Vec::new()))
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Value, A::Error> {
            any_map(map)
        }
        scalar_visits!();
    }

    fn any_map<'de, A: MapAccess<'de>>(map: A) -> Result<Value, A::Error> {
        serde::Deserialize::deserialize(MapAccessDeserializer::new(map))
    }

    fn any_seq<'de, A: SeqAccess<'de>>(seq: A) -> Result<Value, A::Error> {
        serde::Deserialize::deserialize(SeqAccessDeserializer::new(seq))
    }

    /// The scalar visits, each giving the value as it is.
    macro_rules! scalar_visits {
        () => {
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
                Ok(Value::String(v.to_string()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Value, E> {
                Ok(Value::String(v))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
        };
    }
    use scalar_visits;
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

    /// Validating a screens.json item by item gives what validating it whole
    /// does, for documents that break the schema in many places.
    #[test]
    fn screens_by_item_match_the_whole_document() {
        let base: Value =
            serde_json::from_slice(&read("examples/harbourline/retail.screens.json")).unwrap();
        let all = schemas().unwrap();
        let sv = all.validator(Kind::Screens).unwrap();
        let whole = |v: &Value| validate_value(sv, v);
        type Edit = Box<dyn Fn(&mut Value)>;
        let edits: Vec<Edit> = vec![
            Box::new(|_| {}),
            Box::new(|v| v["networks"][0]["screens"][0]["format"] = 5.into()),
            Box::new(|v| {
                v["networks"][0]["screens"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("id");
            }),
            Box::new(|v| v["networks"][0]["screens"][1]["viewing"]["distance_m"] = "far".into()),
            Box::new(|v| {
                v["networks"][0]["screens"][1]["location"] = serde_json::json!({"lat": 91})
            }),
            Box::new(|v| v["networks"][0]["formats"][0]["pixels"] = "big".into()),
            Box::new(|v| v["networks"][0]["formats"][1]["media"] = serde_json::json!({})),
            Box::new(|v| v["networks"][0]["venues"][0]["address"] = 7.into()),
            Box::new(|v| v["networks"][0]["venues"] = serde_json::json!({})),
            Box::new(|v| v["networks"][0]["screens"] = "none".into()),
            Box::new(|v| v["networks"][0]["screens"] = serde_json::json!([1, null, []])),
            Box::new(|v| {
                v["networks"][0].as_object_mut().unwrap().remove("formats");
            }),
            Box::new(|v| v["networks"] = serde_json::json!([1, {"id": "x"}])),
            Box::new(|v| v["networks"] = serde_json::json!({})),
            Box::new(|v| v["publisher"] = 1.into()),
            Box::new(|v| {
                for s in v["networks"][0]["screens"].as_array_mut().unwrap() {
                    s["id"] = "bad id!".into();
                    s["rules"] = serde_json::json!({"max_words": "x", "rule_packs": [1]});
                }
            }),
        ];
        for (i, edit) in edits.iter().enumerate() {
            let mut v = base.clone();
            edit(&mut v);
            let raw = serde_json::to_vec(&v).unwrap();
            let want = whole(&v);
            assert_eq!(
                validate_screens_by_item(all.items().unwrap(), sv, &raw),
                Some((want.clone(), true)),
                "edit {i}"
            );
            assert_eq!(
                validate_schema(Kind::Screens, &raw, &SchemaOptions::default()).unwrap(),
                want,
                "edit {i}"
            );
        }
        // Text that isn't a document falls back to the usual path and error.
        for bad in [&b"{"[..], b"[]", b"7", b"{\"networks\":[1,"] {
            assert!(
                validate_screens_by_item(all.items().unwrap(), sv, bad).is_none()
                    || serde_json::from_slice::<Value>(bad).is_ok(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
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
        assert!(!d.networks.is_empty());
        crate::decode_manifest(&read("examples/harbourline/creative.json")).unwrap();
    }
}
