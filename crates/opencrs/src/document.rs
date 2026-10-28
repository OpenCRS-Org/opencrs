//! Checks on a screens.json itself. Port of `crs/document.go`.

use crate::check::{
    CODE_REFERENCE, CODE_RULE_CONFLICT, CODE_RULE_PACK, Check, ScreenTable, Severity,
};
use crate::model::{Document, Network, Screen};
use crate::rulepack::{builtin_rule_packs, versions_of};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_json::value::RawValue;
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Collects findings, merging those with the same code and message and
/// listing every screen they affect.
struct Findings {
    out: Vec<Check>,
    index: HashMap<(String, String, [String; 3]), usize>,
    table: ScreenTable,
    /// For each finding, the positions (in `table`) of its screens.
    at: Vec<Vec<u32>>,
    /// The screen being checked; None outside one or for a screen with no id.
    cur: Option<u32>,
}

/// Where in a document a finding is caused: a network, and a venue or a
/// format of it. Empty for a finding about screens.
#[derive(Default, Clone, Copy)]
struct Scope<'a> {
    network: &'a str,
    venue: &'a str,
    format: &'a str,
}

impl Findings {
    fn new() -> Self {
        Findings {
            out: Vec::new(),
            index: HashMap::new(),
            table: ScreenTable::default(),
            at: Vec::new(),
            cur: None,
        }
    }

    /// Starts a screen: findings added until `end_screen` are its.
    fn begin(&mut self, id: &str) {
        let pos = self.table.begin(id);
        self.cur = (!id.is_empty()).then_some(pos);
    }

    fn end_screen(&mut self) {
        self.cur = None;
    }

    fn add(&mut self, code: &str, msg: &str) {
        self.add_as(code, Severity::Error, msg, Scope::default());
    }

    fn add_as(&mut self, code: &str, severity: Severity, msg: &str, scope: Scope) {
        let key = (
            code.to_string(),
            msg.to_string(),
            [scope.network, scope.venue, scope.format].map(String::from),
        );
        let i = match self.index.get(&key) {
            Some(&i) => i,
            None => {
                self.index.insert(key, self.out.len());
                let mut c = Check::new(code, severity, msg);
                c.network = scope.network.to_string();
                c.venue = scope.venue.to_string();
                c.format = scope.format.to_string();
                self.out.push(c);
                self.at.push(Vec::new());
                self.out.len() - 1
            }
        };
        // A screen's findings arrive together, so one remembered position
        // is enough to list it once.
        if let Some(pos) = self.cur
            && self.at[i].last() != Some(&pos)
        {
            self.at[i].push(pos);
        }
    }

    fn finish(self) -> Vec<Check> {
        let mut out = self.out;
        for (c, (list, total)) in out.iter_mut().zip(self.table.lists(self.at)) {
            c.screens = list;
            c.screens_total = (total > c.screens.len()).then_some(total as u64);
        }
        out
    }
}

/// Checks a screens.json for problems in the document itself, so a
/// publisher finds them before any creative is validated: CRS-REFERENCE for
/// a screen naming a format or venue that doesn't exist, and
/// CRS-RULE-CONFLICT for rules that tightening has made impossible to meet,
/// both errors, and a CRS-RULE-PACK warning for a rule pack this validator
/// doesn't know, which creative on those screens won't be checked against.
///
/// A finding is reported where it is caused. A pack named in a network's,
/// venue's or format's rules, or a conflict between a network's rules and a
/// format's or a venue's, is reported once, scoped to that network, venue or
/// format, not once for each screen that inherits it. Findings that arise
/// from a screen itself, or from a venue and a format together, list the
/// screens they affect, up to `MAX_SCREENS_LISTED`.
pub fn check_document(d: &Document) -> Vec<Check> {
    let mut f = Findings::new();
    for n in &d.networks {
        let scoped = check_network(d, n, &mut f);
        for s in &n.screens {
            check_screen(d, n, s, &scoped, &mut f);
        }
    }
    f.finish()
}

/// What a network's venues and formats already account for, by id, so the
/// screens that inherit it don't repeat it.
struct Scoped {
    venue_conflicts: HashMap<String, HashSet<String>>,
    format_conflicts: HashMap<String, HashSet<String>>,
}

/// Reports what the network's own rules, its venues' and its formats' cause,
/// each once.
fn check_network(d: &Document, n: &Network, f: &mut Findings) -> Scoped {
    let at = |venue, format| Scope {
        network: &n.id,
        venue,
        format,
    };
    if let Some(r) = &n.rules {
        unknown_packs(f, &r.rule_packs, at("", ""));
    }
    let mut scoped = Scoped {
        venue_conflicts: HashMap::new(),
        format_conflicts: HashMap::new(),
    };
    for v in &n.venues {
        if let Some(r) = &v.rules {
            unknown_packs(f, &r.rule_packs, at(&v.id, ""));
        }
        let conflicts = d.venue_conflicts(n, v);
        for msg in &conflicts {
            f.add_as(CODE_RULE_CONFLICT, Severity::Error, msg, at(&v.id, ""));
        }
        if !conflicts.is_empty() {
            scoped
                .venue_conflicts
                .entry(v.id.clone())
                .or_default()
                .extend(conflicts);
        }
    }
    for fm in &n.formats {
        if let Some(r) = &fm.fields.rules {
            unknown_packs(f, &r.rule_packs, at("", &fm.id));
        }
        let conflicts = d.build(n, fm, None, None).rules.conflicts;
        for msg in &conflicts {
            f.add_as(CODE_RULE_CONFLICT, Severity::Error, msg, at("", &fm.id));
        }
        if !conflicts.is_empty() {
            scoped
                .format_conflicts
                .entry(fm.id.clone())
                .or_default()
                .extend(conflicts);
        }
    }
    scoped
}

fn check_screen(d: &Document, n: &Network, s: &Screen, scoped: &Scoped, f: &mut Findings) {
    f.begin(&s.id);
    match d.resolve_screen(n, s) {
        Err(e) => f.add(CODE_REFERENCE, &e.to_string()),
        Ok(r) => {
            let covered = |msg: &String| {
                scoped
                    .venue_conflicts
                    .get(&s.venue)
                    .is_some_and(|c| c.contains(msg))
                    || scoped
                        .format_conflicts
                        .get(&s.format)
                        .is_some_and(|c| c.contains(msg))
            };
            for msg in r.rules.conflicts.iter().filter(|m| !covered(m)) {
                f.add(CODE_RULE_CONFLICT, msg);
            }
            // The screen's own rules; the levels above it were reported.
            let own = [
                s.rules.as_deref(),
                s.overrides.as_ref().and_then(|o| o.fields.rules.as_ref()),
            ];
            for rs in own.into_iter().flatten() {
                unknown_packs(f, &rs.rule_packs, Scope::default());
            }
        }
    }
    f.end_screen();
}

/// An object's members, in order, with each value left as the text it was.
struct Members<'a>(Vec<(String, &'a RawValue)>);

impl<'de> Deserialize<'de> for Members<'de> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Members<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(kv) = m.next_entry::<String, &RawValue>()? {
                    out.push(kv);
                }
                Ok(Members(out))
            }
        }
        de.deserialize_map(V)
    }
}

impl<'a> Members<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    /// The text of the object with member `name` left out and, if `put` is
    /// given, `name: put` added; None if `name` is there twice, which the
    /// typed model refuses.
    fn without(&self, name: &str, put: &str) -> Option<(String, Option<&'a RawValue>)> {
        let mut text = String::from("{");
        let mut found = None;
        for (k, v) in &self.0 {
            if k == name {
                if found.is_some() {
                    return None;
                }
                found = Some(*v);
                continue;
            }
            text.push_str(&serde_json::to_string(k).ok()?);
            text.push(':');
            text.push_str(v.get());
            text.push(',');
        }
        text.push_str(&format!("\"{name}\":{put}}}"));
        Some((text, found))
    }
}

/// [`check_document`] for a screens.json's text, without decoding all of its
/// screens at once: each network is decoded with its screens left out, and
/// then its screens one at a time, so memory stays near the size of the
/// text however many screens there are. The findings are those of decoding
/// the whole document and checking it. None when the text doesn't decode as
/// a document, as `serde_json::from_slice::<Document>` would fail.
pub fn check_document_text(raw: &str) -> Option<Vec<Check>> {
    let root = Members::parse(raw)?;
    let (head, networks) = root.without("networks", "[]")?;
    let mut d: Document = serde_json::from_str(&head).ok()?;
    let networks: Vec<&RawValue> = serde_json::from_str(networks?.get()).ok()?;
    let mut shells = Vec::with_capacity(networks.len());
    let mut lists = Vec::with_capacity(networks.len());
    for net in networks {
        let (head, screens) = Members::parse(net.get())?.without("screens", "[]")?;
        shells.push(serde_json::from_str::<Network>(&head).ok()?);
        lists.push(serde_json::from_str::<Vec<&RawValue>>(screens?.get()).ok()?);
    }
    d.networks = shells;
    let mut f = Findings::new();
    for (n, list) in d.networks.iter().zip(lists) {
        let scoped = check_network(&d, n, &mut f);
        for raw in list {
            let s: Screen = serde_json::from_str(raw.get()).ok()?;
            check_screen(&d, n, &s, &scoped, &mut f);
        }
    }
    Some(f.finish())
}

/// Warns about each referenced rule pack that isn't built in, naming the
/// versions that are when only the version is wrong.
fn unknown_packs(f: &mut Findings, refs: &[String], scope: Scope) {
    let known = builtin_rule_packs();
    for reference in refs {
        if known.contains_key(reference) {
            continue;
        }
        let mut msg = format!(
            "Rule pack {reference} isn't known to this validator, so creative won't be checked against it"
        );
        let name = reference.split('@').next().unwrap_or(reference);
        let versions = versions_of(known, name);
        if !versions.is_empty() {
            msg += &format!(" (known: {})", versions.join(", "));
        }
        f.add_as(CODE_RULE_PACK, Severity::Warning, &msg, scope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// The text-streaming check finds what decoding the whole document does.
    #[test]
    fn streaming_check_matches_the_whole_document() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let base: Value = serde_json::from_slice(
            &std::fs::read(root.join("examples/harbourline/retail.screens.json")).unwrap(),
        )
        .unwrap();
        let mut docs = vec![base.clone()];
        let mut with = |f: &dyn Fn(&mut Value)| {
            let mut v = base.clone();
            f(&mut v);
            docs.push(v);
        };
        with(&|v| v["networks"][0]["screens"][0]["format"] = "nope".into());
        with(&|v| v["networks"][0]["screens"][1]["venue"] = "nowhere".into());
        with(&|v| v["networks"][0]["rules"]["rule_packs"] = json!(["nope@1", "nope@1"]));
        with(&|v| {
            let s = v["networks"][0]["screens"].as_array().unwrap().clone();
            v["networks"][0]["screens"] =
                Value::Array(s.iter().cloned().chain(s.clone()).collect());
            v["networks"][0]["formats"][0]["rules"] = json!({"rule_packs": ["x@2"]});
        });
        with(&|v| v["networks"][0]["formats"] = json!([]));
        with(&|v| {
            let n = v["networks"][0].clone();
            v["networks"] = json!([n.clone(), n]);
        });
        with(&|v| v["networks"][0]["screens"][0]["pixels"] = "typed badly".into());
        with(&|v| v["networks"][0]["screens"][0]["viewing"]["dwell_s"] = "x".into());
        with(&|v| {
            v.as_object_mut().unwrap().remove("networks");
        });
        for (i, v) in docs.iter().enumerate() {
            let text = serde_json::to_string(v).unwrap();
            let want = serde_json::from_str::<Document>(&text)
                .ok()
                .map(|d| check_document(&d));
            assert_eq!(check_document_text(&text), want, "document {i}");
        }
        // Repeated members make the typed model fail, so there are no checks.
        let dup = r#"{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-01-01T00:00:00Z",
            "networks":[{"id":"n","name":"N","formats":[],"screens":[],"screens":[]}]}"#;
        assert!(serde_json::from_str::<Document>(dup).is_err());
        assert_eq!(check_document_text(dup), None);
        assert_eq!(check_document_text("[]"), None);
    }

    /// Findings list each screen once, in document order, even when ids
    /// repeat or the findings of many screens interleave.
    #[test]
    fn findings_list_screens_once_in_order() {
        let screens = ["a", "b", "a", "c", "b", "d"];
        let doc = json!({
            "opencrs": "0.1", "publisher": "p.example", "updated_at": "2026-01-01T00:00:00Z",
            "networks": [{
                "id": "n", "name": "N",
                "formats": [{"id": "f", "pixels": [1, 1], "media": {"static": ["image/png"]}}],
                "screens": screens.iter().map(|i| json!({"id": i, "format": "f", "rules": {"rule_packs": ["x@1", "y@1"]}})).collect::<Vec<_>>(),
            }],
        });
        let text = doc.to_string();
        let d: Document = serde_json::from_str(&text).unwrap();
        for checks in [check_document(&d), check_document_text(&text).unwrap()] {
            assert_eq!(checks.len(), 2);
            for c in &checks {
                assert_eq!(c.screens, ["a", "b", "c", "d"]);
            }
            let out = serde_json::to_string(&checks[0]).unwrap();
            assert!(out.contains(r#""screens":["a","b","c","d"]"#), "{out}");
        }
    }

    /// A pack named above the screens is reported once where it is named; a
    /// finding on more than 1,000 screens lists the first 1,000 and counts.
    #[test]
    fn scoped_findings_are_reported_once_and_long_lists_are_cut() {
        let screens: Vec<Value> = (0..2500)
            .map(|i| json!({"id": format!("s{i}"), "format": "f", "venue": "v"}))
            .collect();
        let mut doc = json!({
            "opencrs": "0.1", "publisher": "p.example", "updated_at": "2026-01-01T00:00:00Z",
            "networks": [{
                "id": "n", "name": "N",
                "rules": {"rule_packs": ["net@1"]},
                "venues": [{"id": "v", "name": "V", "rules": {"rule_packs": ["ven@1"]}}],
                "formats": [{"id": "f", "pixels": [1, 1], "media": {"static": ["image/png"]},
                    "rules": {"rule_packs": ["fmt@1"]}}],
                "screens": screens,
            }],
        });
        let text = doc.to_string();
        let d: Document = serde_json::from_str(&text).unwrap();
        for checks in [check_document(&d), check_document_text(&text).unwrap()] {
            let scopes: Vec<_> = checks
                .iter()
                .map(|c| (c.network.as_str(), c.venue.as_str(), c.format.as_str()))
                .collect();
            assert_eq!(scopes, [("n", "", ""), ("n", "v", ""), ("n", "", "f")]);
            assert!(
                checks
                    .iter()
                    .all(|c| c.screens.is_empty() && c.screens_total.is_none())
            );
        }
        // Unknown packs on 2,500 screens: the list stops at 1,000.
        for s in doc["networks"][0]["screens"].as_array_mut().unwrap() {
            s["rules"] = json!({"rule_packs": ["own@1"]});
        }
        let text = doc.to_string();
        let d: Document = serde_json::from_str(&text).unwrap();
        for checks in [check_document(&d), check_document_text(&text).unwrap()] {
            let own = checks.iter().find(|c| c.message.contains("own@1")).unwrap();
            assert_eq!(own.screens.len(), 1000);
            assert_eq!(own.screens_total, Some(2500));
            assert_eq!(own.screens.iter().last(), Some("s999"));
            let out = serde_json::to_string(own).unwrap();
            assert!(out.contains(r#""screens_total":2500"#), "{}", &out[..120]);
        }
    }
}
