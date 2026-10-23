//! Checks on a screens.json itself. Port of `crs/document.go`.

use crate::check::{CODE_REFERENCE, CODE_RULE_CONFLICT, Check, Severity};
use crate::model::Document;
use std::collections::{HashMap, HashSet};

/// Collects findings, merging those with the same code and message and
/// listing every screen they affect.
struct Findings {
    out: Vec<Check>,
    index: HashMap<(String, String), usize>,
}

impl Findings {
    fn add(&mut self, code: &str, msg: &str, screen: &str) {
        let key = (code.to_string(), msg.to_string());
        if let Some(&i) = self.index.get(&key) {
            let c = &mut self.out[i];
            if !screen.is_empty() && !c.screens.iter().any(|s| s == screen) {
                c.screens.push(screen.to_string());
            }
            return;
        }
        let mut c = Check::new(code, Severity::Error, msg);
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
/// CRS-RULE-CONFLICT for rules that tightening has made impossible to meet.
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
        }
    }
    f.out
}
