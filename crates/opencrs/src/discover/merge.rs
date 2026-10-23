//! Merging one publisher's documents, ported from `discover/merge.go`.

use chrono::DateTime;
use serde_json::{Map, Value};

use crate::error::Error;
use crate::json::to_string_pretty_escaped;
use crate::schema::{Kind, SchemaOptions, validate_schema};

type Obj = Map<String, Value>;

/// Combines one publisher's documents, typically a public document and the
/// partner document that adds locations and platform IDs to the same
/// screens. It works on the raw JSON so extensions and fields from later
/// 0.x versions survive.
///
/// Networks, venues, formats and screens are matched by id. Where only one
/// document sets a field, it's kept; where both set it to different values,
/// the first document's value is kept and the clash is reported in the
/// conflicts, rather than resolved silently. `updated_at` takes the latest
/// value and `expires_at` the earliest. The result is checked against the
/// screens schema.
pub fn merge(raws: &[&[u8]]) -> Result<(Vec<u8>, Vec<String>), Error> {
    if raws.is_empty() {
        return Err(Error::msg("nothing to merge"));
    }
    let mut conflicts = Vec::new();
    let mut out: Option<Obj> = None;
    for (i, raw) in raws.iter().enumerate() {
        let doc = match serde_json::from_slice::<Value>(raw) {
            Ok(Value::Object(m)) => m,
            Ok(other) => {
                return Err(Error::msg(format!(
                    "document {}: json: cannot unmarshal {} into Go value of type map[string]interface {{}}",
                    i + 1,
                    kind_name(&other)
                )));
            }
            Err(e) => return Err(Error::msg(format!("document {}: {e}", i + 1))),
        };
        let mut doc = doc;
        for v in doc.values_mut() {
            normalise_numbers(v);
        }
        let Some(out) = out.as_mut() else {
            out = Some(doc);
            continue;
        };
        let (op, dp) = (get(out, "publisher"), get(&doc, "publisher"));
        if op != dp {
            return Err(Error::msg(format!(
                "can't merge documents from {} and {}",
                go_display(&op),
                go_display(&dp)
            )));
        }
        let updated = pick_time(&get(out, "updated_at"), &get(&doc, "updated_at"), true);
        out.insert("updated_at".into(), updated);
        let expires = pick_time(&get(out, "expires_at"), &get(&doc, "expires_at"), false);
        out.insert("expires_at".into(), expires);
        merge_object(out, &doc, "", &mut conflicts);
    }
    let mut out = out.expect("at least one document");
    if get(&out, "expires_at") == Value::Null {
        out.remove("expires_at");
    }
    // Go encodes map keys sorted; sort explicitly rather than rely on
    // serde_json's default `Map` ordering (`preserve_order` would change it).
    let mut out = Value::Object(out);
    sort_keys(&mut out);
    let merged = to_string_pretty_escaped(&out).into_bytes();
    let errs = validate_schema(Kind::Screens, &merged, &SchemaOptions::default())?;
    if let Some(first) = errs.first() {
        return Err(Error::msg(format!(
            "the merged document isn't valid: {first}"
        )));
    }
    Ok((merged, conflicts))
}

/// Rebuilds every object with its keys in sorted (byte) order.
fn sort_keys(v: &mut Value) {
    match v {
        Value::Object(m) => {
            let mut entries: Vec<(String, Value)> = std::mem::take(m).into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, mut val) in entries {
                sort_keys(&mut val);
                m.insert(k, val);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(sort_keys),
        _ => {}
    }
}

fn get(m: &Obj, k: &str) -> Value {
    m.get(k).cloned().unwrap_or(Value::Null)
}

/// Go decodes every JSON number into a float64, so 1 and 1.0 are equal and
/// print alike.
fn normalise_numbers(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64().and_then(serde_json::Number::from_f64) {
                *n = f;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(normalise_numbers),
        Value::Object(m) => m.values_mut().for_each(normalise_numbers),
        _ => {}
    }
}

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Go's `%v` for a decoded JSON value, as far as messages need it.
fn go_display(v: &Value) -> String {
    match v {
        Value::Null => "<nil>".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The arrays whose items are matched by id when merging.
fn by_id(k: &str) -> bool {
    matches!(k, "networks" | "venues" | "formats" | "screens")
}

fn merge_object(dst: &mut Obj, src: &Obj, path: &str, conflicts: &mut Vec<String>) {
    for (k, sv) in src {
        if path.is_empty() && (k == "updated_at" || k == "expires_at") {
            continue; // handled by merge
        }
        let p = format!("{path}/{k}");
        let Some(dv) = dst.get_mut(k) else {
            dst.insert(k.clone(), sv.clone());
            continue;
        };
        match (&mut *dv, sv) {
            (Value::Object(dm), Value::Object(sm)) => merge_object(dm, sm, &p, conflicts),
            (Value::Array(da), Value::Array(sa)) if by_id(k) => {
                merge_by_id(da, sa, &p, conflicts);
            }
            (dv, sv) if dv != sv => conflicts.push(format!(
                "{p} differs between documents; kept the first document's value"
            )),
            _ => {}
        }
    }
}

fn id_of(v: &Value) -> Option<&str> {
    v.as_object()?.get("id")?.as_str()
}

fn merge_by_id(dst: &mut Vec<Value>, src: &[Value], path: &str, conflicts: &mut Vec<String>) {
    let mut index = std::collections::HashMap::<String, usize>::new();
    for (i, item) in dst.iter().enumerate() {
        if let Some(id) = id_of(item) {
            index.insert(id.to_string(), i);
        }
    }
    for item in src {
        let Some(id) = id_of(item) else {
            dst.push(item.clone());
            continue;
        };
        if let Some(&i) = index.get(id) {
            if let (Some(existing), Some(m)) = (dst[i].as_object_mut(), item.as_object()) {
                merge_object(existing, m, &format!("{path}/{id}"), conflicts);
            }
            continue;
        }
        index.insert(id.to_string(), dst.len());
        dst.push(item.clone());
    }
}

/// Returns the later (or earlier) of two RFC 3339 timestamps, ignoring
/// missing ones.
fn pick_time(a: &Value, b: &Value, later: bool) -> Value {
    let parse = |v: &Value| {
        v.as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
    };
    match (parse(a), parse(b)) {
        (None, None) => a.clone(),
        (None, Some(_)) => b.clone(),
        (Some(_), None) => a.clone(),
        (Some(ta), Some(tb)) => {
            if later == (tb > ta) {
                b.clone()
            } else {
                a.clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screens(publisher: &str, network: &str, ids: &[&str]) -> String {
        let ss: Vec<String> = ids
            .iter()
            .map(|id| format!(r#"{{"id":"{id}","format":"f"}}"#))
            .collect();
        format!(
            r#"{{"opencrs":"0.1","publisher":"{publisher}","updated_at":"2026-10-20T09:00:00+11:00","networks":[{{"id":"{network}","name":"N",
	  "formats":[{{"id":"f","pixels":[1080,1920],"media":{{"static":["image/png"]}}}}],"screens":[{}]}}]}}"#,
            ss.join(",")
        )
    }

    #[test]
    fn merge_public_and_partner() {
        let public = r#"{"opencrs":"0.1","publisher":"p.example","updated_at":"2026-10-20T09:00:00+11:00","expires_at":"2027-01-20T00:00:00+11:00",
	  "networks":[{"id":"retail","name":"Retail","ext":{"cms-a.example":{"tier":"gold"}},
	    "formats":[{"id":"f","pixels":[1080,1920],"media":{"static":["image/png"]},"future_field":true}],
	    "screens":[{"id":"S1","format":"f","placement":{"environment":"indoor"}},{"id":"S2","format":"f"}]}]}"#;
        let partner = r#"{"opencrs":"0.1","publisher":"p.example","updated_at":"2026-10-21T09:00:00+11:00","expires_at":"2026-12-01T00:00:00+11:00",
	  "networks":[{"id":"retail","name":"Retail","formats":[],
	    "screens":[{"id":"S1","format":"f","location":{"lat":-33.86,"lon":151.2},"placement":{"environment":"outdoor","level":"L2"}},
	               {"id":"S3","format":"f","platform_ids":{"cms-a":"frame-1"}}]}]}"#;
        let (merged, conflicts) = merge(&[public.as_bytes(), partner.as_bytes()]).unwrap();
        let doc: Value = serde_json::from_slice(&merged).unwrap();
        let n = &doc["networks"][0];
        let screens = n["screens"].as_array().unwrap();
        assert_eq!(screens.len(), 3, "{screens:?}");
        assert!(!screens[0]["location"].is_null());
        assert_eq!(screens[0]["placement"]["environment"], "indoor");
        assert_eq!(screens[0]["placement"]["level"], "L2");
        assert!(!n["ext"].is_null());
        assert_eq!(n["formats"][0]["future_field"], true);
        assert_eq!(doc["updated_at"], "2026-10-21T09:00:00+11:00");
        assert_eq!(doc["expires_at"], "2026-12-01T00:00:00+11:00");
        assert_eq!(conflicts.len(), 1, "{conflicts:?}");
        assert!(conflicts[0].contains("/networks/retail/screens/S1/placement/environment"));
    }

    #[test]
    fn merge_refuses_mixed_publishers() {
        let a = screens("a.example", "n", &["S"]);
        let b = screens("b.example", "n", &["S"]);
        assert!(merge(&[a.as_bytes(), b.as_bytes()]).is_err());
    }

    #[test]
    fn merge_nothing_and_bad_json() {
        assert_eq!(merge(&[]).unwrap_err().to_string(), "nothing to merge");
        let e = merge(&[b"[]"]).unwrap_err().to_string();
        assert!(e.starts_with("document 1: "), "{e}");
    }

    #[test]
    fn merge_output_keys_are_sorted() {
        let a = screens("a.example", "n", &["S"]);
        let (merged, _) = merge(&[a.as_bytes()]).unwrap();
        let s = String::from_utf8(merged).unwrap();
        let pos = |k: &str| s.find(k).unwrap_or_else(|| panic!("{k} missing: {s}"));
        assert!(pos("\"formats\"") < pos("\"id\""));
        assert!(pos("\"id\"") < pos("\"name\""));
        assert!(pos("\"name\"") < pos("\"screens\""));
        assert!(pos("\"networks\"") < pos("\"opencrs\""));
        assert!(pos("\"opencrs\"") < pos("\"publisher\""));
        assert!(pos("\"publisher\"") < pos("\"updated_at\""));
        // The helper itself orders keys whatever the map's own ordering is.
        let mut v: Value =
            serde_json::from_str(r#"{"b":{"z":1,"a":2},"a":[{"y":1,"x":2}]}"#).unwrap();
        sort_keys(&mut v);
        assert_eq!(v.to_string(), r#"{"a":[{"x":2,"y":1}],"b":{"a":2,"z":1}}"#);
    }

    #[test]
    fn merge_output_is_go_style() {
        let a = screens("a.example", "n", &["S"]).replace("\"N\"", "\"A<b>&c\"");
        let (merged, _) = merge(&[a.as_bytes()]).unwrap();
        let s = String::from_utf8(merged).unwrap();
        assert!(s.contains("\"name\": \"A\\u003cb\\u003e\\u0026c\""), "{s}");
        assert!(s.contains("\"pixels\": [\n"), "{s}");
        assert!(!s.ends_with('\n'));
        // Keys come out sorted, as Go's map encoding does.
        assert!(s.find("\"networks\"").unwrap() < s.find("\"opencrs\"").unwrap());
    }
}
