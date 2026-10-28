//! Checks on a screens.json itself. Port of `crs/document.go`.

use crate::check::{
    CODE_REFERENCE, CODE_RULE_CONFLICT, CODE_RULE_PACK, Check, ScreenTable, Severity,
};
use crate::model::{Document, Network, RuleSet, Screen};
use crate::resolve::{ConflictCause, Level, RuleCache, position_of};
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
    /// Unknown-pack findings made so far for each scope.
    packs: HashMap<[String; 3], usize>,
    /// Per scope, the finding that stands for the unknown packs past the
    /// limit.
    overflow: HashMap<[String; 3], Overflow>,
}

/// The most unknown-pack findings one scope gets. A scope naming more has
/// the rest counted in one more finding, so the output stays small however
/// many packs a document names.
const MAX_PACK_FINDINGS: usize = 100;

struct Overflow {
    at: usize,
    seen: HashSet<u64>,
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
            packs: HashMap::new(),
            overflow: HashMap::new(),
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
        let scope_key = [scope.network, scope.venue, scope.format].map(String::from);
        let key = (code.to_string(), msg.to_string(), scope_key.clone());
        let i = match self.index.get(&key) {
            Some(&i) => i,
            None => {
                if code == CODE_RULE_PACK
                    && self
                        .packs
                        .get(&scope_key)
                        .is_some_and(|&n| n >= MAX_PACK_FINDINGS)
                {
                    self.fold_pack(&scope_key, msg, severity, scope)
                } else {
                    if code == CODE_RULE_PACK {
                        *self.packs.entry(scope_key).or_default() += 1;
                    }
                    self.index.insert(key, self.out.len());
                    self.push(code, severity, msg, scope)
                }
            }
        };
        self.note_screen(i);
    }

    /// Lists the current screen under finding `i`. A screen's findings
    /// arrive together, so one remembered position is enough to list it once.
    fn note_screen(&mut self, i: usize) {
        if let Some(pos) = self.cur
            && self.at[i].last() != Some(&pos)
        {
            self.at[i].push(pos);
        }
    }

    /// Whether `scope` already has as many unknown-pack findings as it gets.
    fn packs_full(&self, scope: Scope) -> bool {
        let key = [scope.network, scope.venue, scope.format].map(String::from);
        self.packs
            .get(&key)
            .is_some_and(|&n| n >= MAX_PACK_FINDINGS)
    }

    /// Counts an unknown pack past the limit without making its message.
    fn fold_ref(&mut self, reference: &str, scope: Scope) {
        let key = [scope.network, scope.venue, scope.format].map(String::from);
        let i = self.fold_pack(&key, reference, Severity::Warning, scope);
        self.note_screen(i);
    }

    fn push(&mut self, code: &str, severity: Severity, msg: &str, scope: Scope) -> usize {
        crate::budget::charge(crate::budget::Work::Findings, 0, 256);
        let mut c = Check::new(code, severity, msg);
        c.network = scope.network.to_string();
        c.venue = scope.venue.to_string();
        c.format = scope.format.to_string();
        self.out.push(c);
        self.at.push(Vec::new());
        self.out.len() - 1
    }

    /// Counts an unknown pack past the scope's limit in that scope's one
    /// summary finding, and returns it.
    fn fold_pack(
        &mut self,
        key: &[String; 3],
        msg: &str,
        severity: Severity,
        scope: Scope,
    ) -> usize {
        use std::hash::{Hash, Hasher};
        let at = match self.overflow.get(key) {
            Some(o) => o.at,
            None => {
                let at = self.push(CODE_RULE_PACK, severity, "", scope);
                self.overflow.insert(
                    key.clone(),
                    Overflow {
                        at,
                        seen: HashSet::new(),
                    },
                );
                at
            }
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        msg.hash(&mut h);
        self.overflow
            .get_mut(key)
            .expect("just made")
            .seen
            .insert(h.finish());
        at
    }

    fn finish(self) -> Vec<Check> {
        let mut out = self.out;
        for o in self.overflow.values() {
            out[o.at].message = format!(
                "{} more rule packs named here aren't known to this validator, so creative won't be checked against them",
                o.seen.len()
            );
        }
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
        let mut cache = RuleCache::default();
        let mut scoped = check_network(d, n, &mut cache, &mut f);
        for s in &n.screens {
            if crate::budget::tripped() {
                break;
            }
            check_screen(d, n, s, &mut scoped, &mut cache, &mut f);
        }
    }
    crate::budget::explain(Some(d));
    f.finish()
}

/// What a network's formats already account for, so the screens that
/// inherit it don't repeat it. A conflict is told apart by its cause (the
/// element and the level that restated it), not by its wording, which also
/// depends on what the levels above it changed.
struct Scoped {
    /// By position of the first format with each id, the elements whose
    /// restating by the format conflicts with the network's.
    format_causes: HashMap<usize, HashSet<(String, String)>>,
    /// By (venue position or `usize::MAX`, format position), which of the
    /// conflicts of the levels above a screen are not accounted for by the
    /// venue's or the format's own: those a venue and a format cause
    /// together. Worked out once for the pair, not for each screen.
    pair_own: HashMap<(usize, usize), Vec<usize>>,
}

/// Reports what the network's own rules, its venues' and its formats' cause,
/// each once.
fn check_network(d: &Document, n: &Network, cache: &mut RuleCache, f: &mut Findings) -> Scoped {
    let at = |venue, format| Scope {
        network: &n.id,
        venue,
        format,
    };
    if let Some(r) = &n.rules {
        unknown_packs(f, r, at("", ""));
    }
    let mut scoped = Scoped {
        format_causes: HashMap::new(),
        pair_own: HashMap::new(),
    };
    for v in &n.venues {
        if let Some(r) = &v.rules {
            unknown_packs(f, r, at(&v.id, ""));
        }
        for msg in d.venue_rules(cache, n, v).conflicts.iter() {
            f.add_as(CODE_RULE_CONFLICT, Severity::Error, msg, at(&v.id, ""));
        }
    }
    for (i, fm) in n.formats.iter().enumerate() {
        if let Some(r) = &fm.fields.rules {
            unknown_packs(f, r, at("", &fm.id));
        }
        let rules = d.format_rules(cache, n, fm);
        for msg in rules.conflicts.iter() {
            f.add_as(CODE_RULE_CONFLICT, Severity::Error, msg, at("", &fm.id));
        }
        // Screens use the first format with an id; a repeat of it is
        // reported, but no screen inherits it.
        if !rules.causes.is_empty()
            && n.format(&fm.id).and_then(|x| position_of(&n.formats, x)) == Some(i)
        {
            scoped.format_causes.insert(
                i,
                rules
                    .causes
                    .iter()
                    .map(|c| (c.role.clone(), c.when.clone()))
                    .collect(),
            );
        }
    }
    scoped
}

fn check_screen(
    d: &Document,
    n: &Network,
    s: &Screen,
    scoped: &mut Scoped,
    cache: &mut RuleCache,
    f: &mut Findings,
) {
    f.begin(&s.id);
    crate::budget::charge_flat(8);
    match d.resolve_screen_with(cache, n, s) {
        Err(e) => f.add(CODE_REFERENCE, &e.to_string()),
        Ok(r) => {
            let format_at = n
                .format(&s.format)
                .and_then(|x| position_of(&n.formats, x))
                .unwrap_or(usize::MAX);
            let venue_at = r
                .venue
                .and_then(|v| position_of(&n.venues, v))
                .unwrap_or(usize::MAX);
            let (conflicts, causes) = (&r.rules.conflicts, &r.rules.causes);
            // The conflicts of the levels above the screen come first, and
            // are the same for every screen with this venue and format; the
            // screen's own follow.
            let own_from = causes
                .iter()
                .position(|c| matches!(c.level, Level::Screen | Level::ScreenOverrides))
                .unwrap_or(causes.len());
            let above = scoped
                .pair_own
                .entry((venue_at, format_at))
                .or_insert_with(|| {
                    let covered = |c: &ConflictCause| match c.level {
                        // The venue's conflicts with the network don't
                        // depend on the screen, and were reported for the
                        // venue.
                        Level::Venue => true,
                        Level::Format => scoped
                            .format_causes
                            .get(&format_at)
                            .is_some_and(|set| set.contains(&(c.role.clone(), c.when.clone()))),
                        _ => false,
                    };
                    (0..own_from).filter(|&i| !covered(&causes[i])).collect()
                });
            for &i in above.iter() {
                f.add(CODE_RULE_CONFLICT, &conflicts[i]);
            }
            for msg in &conflicts[own_from..] {
                f.add(CODE_RULE_CONFLICT, msg);
            }
            // The screen's own rules; the levels above it were reported.
            let own = [
                s.rules.as_deref(),
                s.overrides.as_ref().and_then(|o| o.fields.rules.as_ref()),
            ];
            for rs in own.into_iter().flatten() {
                unknown_packs(f, rs, Scope::default());
            }
        }
    }
    f.end_screen();
}

/// A screen read for the document checks, which look only at its ids, its
/// overrides and its own rules: the rest (placement, viewing, platform ids,
/// location) is skipped unread, and left at its default. Cheaper than the
/// whole screen, which dominates the time to lint a large document.
struct LintScreen(Screen);

impl<'de> Deserialize<'de> for LintScreen {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Slim {
            id: String,
            format: String,
            #[serde(default)]
            venue: String,
            #[serde(default)]
            overrides: Option<Box<crate::model::Overrides>>,
            #[serde(default)]
            rules: Option<Box<RuleSet>>,
        }
        let s = Slim::deserialize(de)?;
        Ok(LintScreen(Screen {
            id: s.id,
            format: s.format,
            venue: s.venue,
            overrides: s.overrides,
            rules: s.rules,
            ..Screen::default()
        }))
    }
}

/// An object's members, in order, with each value left as the text it was.
pub(crate) struct Members<'a>(Vec<(String, &'a RawValue)>);

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
    /// The text of the object with member `name` left out and, if `put` is
    /// given, `name: put` added; None if `name` is there twice, which the
    /// typed model refuses.
    pub(crate) fn without(&self, name: &str, put: &str) -> Option<(String, Option<&'a RawValue>)> {
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
    let root: Root = serde_json::from_str(raw).ok()?;
    let mut d: Document = serde_json::from_str(&root.head()?).ok()?;
    let mut shells = Vec::with_capacity(root.networks.len());
    let mut lists = Vec::with_capacity(root.networks.len());
    for net in root.networks {
        let (head, screens) = net.without("screens", "[]")?;
        shells.push(serde_json::from_str::<Network>(&head).ok()?);
        lists.push(screens?);
    }
    d.networks = shells;
    let mut f = Findings::new();
    for (n, list) in d.networks.iter().zip(lists) {
        let mut cache = RuleCache::default();
        let mut scoped = check_network(&d, n, &mut cache, &mut f);
        // The screens are decoded one at a time from the list's text, and
        // no further once the budget is spent.
        let mut de = serde_json::Deserializer::from_str(list.get());
        let mut stopped = false;
        let mut each = |s: Screen| {
            crate::budget::checkpoint();
            if crate::budget::tripped() {
                stopped = true;
                return false;
            }
            check_screen(&d, n, &s, &mut scoped, &mut cache, &mut f);
            true
        };
        let read = serde::de::DeserializeSeed::deserialize(
            ScreenStream::<LintScreen>::new(&mut each),
            &mut de,
        );
        if !stopped {
            read.ok()?;
            de.end().ok()?;
        }
    }
    crate::budget::explain(Some(&d));
    Some(f.finish())
}

/// A document's members except `networks`, each left as its text, and its
/// networks each as their members, so the text is read once.
pub(crate) struct Root<'a> {
    pub(crate) head: Vec<(String, &'a RawValue)>,
    pub(crate) networks: Vec<Members<'a>>,
    /// `networks` was there twice, which the typed model refuses.
    repeated: bool,
}

impl Root<'_> {
    /// The text of the document with `networks` set to `[]`.
    pub(crate) fn head(&self) -> Option<String> {
        if self.repeated {
            return None;
        }
        let mut text = String::from("{");
        for (k, v) in &self.head {
            text.push_str(&serde_json::to_string(k).ok()?);
            text.push(':');
            text.push_str(v.get());
            text.push(',');
        }
        text.push_str("\"networks\":[]}");
        Some(text)
    }
}

impl<'de> Deserialize<'de> for Root<'de> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Root<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a document")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Self::Value, A::Error> {
                let mut root = Root {
                    head: Vec::new(),
                    networks: Vec::new(),
                    repeated: false,
                };
                let mut seen = false;
                while let Some(k) = m.next_key::<String>()? {
                    if k == "networks" {
                        root.repeated |= seen;
                        seen = true;
                        root.networks = m.next_value::<Vec<Members>>()?;
                    } else {
                        root.head.push((k, m.next_value::<&RawValue>()?));
                    }
                }
                if !seen {
                    return Err(serde::de::Error::missing_field("networks"));
                }
                Ok(root)
            }
        }
        de.deserialize_map(V)
    }
}

impl From<LintScreen> for Screen {
    fn from(s: LintScreen) -> Screen {
        s.0
    }
}

/// Reads a list of screens one at a time, each decoded as a `T` (the whole
/// screen, or the part a lint reads), handing each to a function that says
/// whether to go on; stopping leaves the rest of the list unread, which the
/// deserializer reports as an error the caller disregards.
pub(crate) struct ScreenStream<'f, T>(
    pub(crate) &'f mut dyn FnMut(Screen) -> bool,
    pub(crate) std::marker::PhantomData<T>,
);

impl<'f, T> ScreenStream<'f, T> {
    pub(crate) fn new(each: &'f mut dyn FnMut(Screen) -> bool) -> Self {
        ScreenStream(each, std::marker::PhantomData)
    }
}

impl<'de, T: Deserialize<'de> + Into<Screen>> serde::de::DeserializeSeed<'de>
    for ScreenStream<'_, T>
{
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_seq(self)
    }
}

impl<'de, T: Deserialize<'de> + Into<Screen>> Visitor<'de> for ScreenStream<'_, T> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a list of screens")
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while let Some(s) = seq.next_element::<T>()? {
            if !(self.0)(s.into()) {
                return Err(serde::de::Error::custom("stopped"));
            }
        }
        Ok(())
    }
}

/// Warns about each referenced rule pack that isn't built in, naming the
/// versions that are when only the version is wrong.
fn unknown_packs(f: &mut Findings, rules: &RuleSet, scope: Scope) {
    let known = builtin_rule_packs();
    let refs = &rules.rule_packs;
    // Each reference is looked up, and may become a finding.
    if !crate::budget::charge(
        crate::budget::Work::Packs,
        rules as *const RuleSet as usize,
        refs.len() as u64 * 32,
    ) {
        return;
    }
    for reference in refs {
        if known.contains_key(reference) {
            continue;
        }
        if f.packs_full(scope) {
            f.fold_ref(reference, scope);
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
        with(&|v| v["networks"][0]["screens"][0]["overrides"] = json!({"pixels": "x"}));
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

    fn network_doc(rules: Value, screens: Vec<Value>) -> Document {
        serde_json::from_value(json!({
            "opencrs": "0.1", "publisher": "p.example", "updated_at": "2026-01-01T00:00:00Z",
            "networks": [{
                "id": "n", "name": "N",
                "rules": rules,
                "venues": [{"id": "v", "name": "V"}],
                "formats": [{"id": "f", "pixels": [1, 1], "media": {"static": ["image/png"]}}],
                "screens": screens,
            }],
        }))
        .unwrap()
    }

    fn timed(f: impl FnOnce() -> Vec<Check>) -> Vec<Check> {
        let start = std::time::Instant::now();
        let out = f();
        assert!(start.elapsed().as_secs() < 30, "took {:?}", start.elapsed());
        out
    }

    /// Merging a long list of rules is not a cost for every screen: these
    /// inputs took many seconds, then minutes, when each screen redid the
    /// merge (the second two are over the schema's limits on purpose, as a
    /// typed document can be).
    #[test]
    fn many_rules_and_screens_stay_fast() {
        let screens = |n: usize| -> Vec<Value> {
            (0..n)
                .map(|i| json!({"id": format!("s{i}"), "format": "f", "venue": "v"}))
                .collect()
        };
        let elements: Vec<Value> = (0..2000)
            .map(|i| json!({"role": format!("r{i}"), "min_cap_height_mm": 1}))
            .collect();
        let d = network_doc(json!({"required_elements": elements}), screens(2000));
        assert!(timed(|| check_document(&d)).is_empty());

        let packs: Vec<String> = (0..50_000).map(|i| format!("p{i}@1")).collect();
        let d = network_doc(json!({"rule_packs": packs}), screens(5000));
        let checks = timed(|| check_document(&d));
        // 100 findings, and one that counts the 49,900 others.
        assert_eq!(checks.len(), 101);
        assert!(
            checks[100].message.starts_with("49900 more rule packs"),
            "{}",
            checks[100].message
        );
        assert!(
            checks
                .iter()
                .all(|c| c.screens.is_empty() && c.network == "n")
        );
    }

    /// A conflict the format causes against the network is reported once,
    /// for the format, even where a venue's rules change what it conflicts
    /// with and so how it reads on that venue's screens.
    #[test]
    fn a_format_conflict_is_reported_once() {
        let legal = |rect: [i64; 4]| json!({"required_elements": [{"role": "legal", "within_rect_px": rect}]});
        let mut d = network_doc(
            legal([0, 1600, 1080, 320]),
            vec![
                json!({"id": "a", "format": "f", "venue": "v"}),
                json!({"id": "b", "format": "f"}),
            ],
        );
        d.networks[0].venues[0].rules =
            serde_json::from_value(legal([0, 1700, 1080, 100])).unwrap();
        d.networks[0].formats[0].fields.rules =
            serde_json::from_value(legal([0, 0, 1080, 300])).unwrap();
        let checks = check_document(&d);
        let conflicts: Vec<_> = checks
            .iter()
            .filter(|c| c.code == "CRS-RULE-CONFLICT")
            .collect();
        assert_eq!(conflicts.len(), 1, "{conflicts:?}");
        assert_eq!(conflicts[0].format, "f");
        assert!(conflicts[0].screens.is_empty());
    }

    /// A repeated format is reported on its own, and its conflicts are not
    /// taken for the first one's: screens use the first.
    #[test]
    fn a_repeated_format_does_not_cover_the_first() {
        let legal = |rect: [i64; 4]| -> crate::model::RuleSet {
            serde_json::from_value(
                json!({"required_elements": [{"role": "legal", "within_rect_px": rect}]}),
            )
            .unwrap()
        };
        let mut d = network_doc(
            json!({"required_elements": [{"role": "legal", "within_rect_px": [0, 1600, 1080, 320]}]}),
            vec![json!({"id": "a", "format": "f"})],
        );
        let mut again = d.networks[0].formats[0].clone();
        again.fields.rules = Some(legal([0, 0, 1080, 300]));
        d.networks[0].formats.push(again);
        let checks = check_document(&d);
        assert_eq!(checks.len(), 1, "{checks:?}");
        assert!(checks[0].screens.is_empty());
        // Now the first one conflicts, and the repeat doesn't.
        let mut d = d;
        d.networks[0].formats.reverse();
        let checks = check_document(&d);
        assert_eq!(checks.len(), 1, "{checks:?}");
    }
}
