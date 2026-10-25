//! Checks on a screens.json itself. Port of `crs/document.go`.

use crate::check::{CODE_REFERENCE, CODE_RULE_CONFLICT, CODE_RULE_PACK, Check, Severity};
use crate::model::Document;
use crate::rulepack::{builtin_rule_packs, versions_of};
use std::collections::{HashMap, HashSet};

/// Collects findings, merging those with the same code and message and
/// listing every screen they affect.
struct Findings {
    out: Vec<Check>,
    index: HashMap<(String, String), usize>,
}

impl Findings {
    fn add(&mut self, code: &str, msg: &str, screen: &str) {
        self.add_as(code, Severity::Error, msg, screen);
    }

    fn add_as(&mut self, code: &str, severity: Severity, msg: &str, screen: &str) {
        let key = (code.to_string(), msg.to_string());
        if let Some(&i) = self.index.get(&key) {
            let c = &mut self.out[i];
            if !screen.is_empty() && !c.screens.iter().any(|s| s == screen) {
                c.screens.push(screen.to_string());
            }
            return;
        }
        let mut c = Check::new(code, severity, msg);
        if !screen.is_empty() {
            c.screens = vec![screen.to_string()];
        }
        self.index.insert(key, self.out.len());
        self.out.push(c);
    }
}

/// Checks a screens.json for problems in the document itself, so a
/// publisher finds them before any creative is validated: CRS-REFERENCE for
/// a screen naming a format or venue that doesn't exist, and
/// CRS-RULE-CONFLICT for rules that tightening has made impossible to meet,
/// both errors, and a CRS-RULE-PACK warning for a rule pack this validator
/// doesn't know, which creative on those screens won't be checked against.
/// Each finding lists the screens it affects.
pub fn check_document(d: &Document) -> Vec<Check> {
    let mut f = Findings {
        out: Vec::new(),
        index: HashMap::new(),
    };
    for n in &d.networks {
        let mut used: HashSet<&str> = HashSet::new();
        for s in &n.screens {
            used.insert(&s.format);
            match d.resolve_screen(n, s) {
                Err(e) => f.add(CODE_REFERENCE, &e.to_string(), &s.id),
                Ok(r) => {
                    for msg in &r.rules.conflicts {
                        f.add(CODE_RULE_CONFLICT, msg, &s.id);
                    }
                    unknown_packs(&mut f, &r.rules.rule_packs, &s.id);
                }
            }
        }
        // Formats no screen uses still carry network and format rules.
        for fm in &n.formats {
            if used.contains(fm.id.as_str()) {
                continue;
            }
            let r = d.build(n, fm, None, None);
            for msg in &r.rules.conflicts {
                f.add(CODE_RULE_CONFLICT, msg, "");
            }
            unknown_packs(&mut f, &r.rules.rule_packs, "");
        }
    }
    f.out
}

/// Warns about each referenced rule pack that isn't built in, naming the
/// versions that are when only the version is wrong.
fn unknown_packs(f: &mut Findings, refs: &[String], screen: &str) {
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
        f.add_as(CODE_RULE_PACK, Severity::Warning, &msg, screen);
    }
}
