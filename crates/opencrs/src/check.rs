//! Check findings: severities, standard codes and the `Check` record.
//! Port of the types and constants at the top of `crs/check.go`.

use crate::model::Rect;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Severity of a single check finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
    NotChecked,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::NotChecked => "not_checked",
        }
    }
}

// Standard check codes. CRS-RULE-CONFLICT and CRS-REFERENCE are also
// reported against a screens.json itself; see `check_document`.
pub const CODE_PIXELS: &str = "CRS-PIXELS";
pub const CODE_MEDIA_TYPE: &str = "CRS-MEDIA-TYPE";
pub const CODE_FILE_SIZE: &str = "CRS-FILE-SIZE";
pub const CODE_DURATION: &str = "CRS-DURATION";
pub const CODE_SAFE_AREA: &str = "CRS-SAFE-AREA";
pub const CODE_BLOCKED_REGION: &str = "CRS-BLOCKED-REGION";
pub const CODE_LEGIBILITY: &str = "CRS-LEGIBILITY";
pub const CODE_MAX_WORDS: &str = "CRS-MAX-WORDS";
pub const CODE_REQUIRED_ELEMENT: &str = "CRS-REQUIRED-ELEMENT";
pub const CODE_CATEGORY: &str = "CRS-CATEGORY";
pub const CODE_MOTION: &str = "CRS-MOTION";
pub const CODE_SPEC_STALE: &str = "CRS-SPEC-STALE";
pub const CODE_MANIFEST_MISMATCH: &str = "CRS-MANIFEST-MISMATCH";
pub const CODE_TARGET: &str = "CRS-TARGET";
pub const CODE_RULE_PACK: &str = "CRS-RULE-PACK";
pub const CODE_RULE_CONFLICT: &str = "CRS-RULE-CONFLICT";
pub const CODE_REFERENCE: &str = "CRS-REFERENCE";

/// One finding in a report. Checks that pass aren't reported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect_px: Option<Rect>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub element: String,
    /// For a finding about a screens.json that is caused at one level of it,
    /// where: the network's id, with a venue's or a format's when the finding
    /// is about that venue or format. Such a finding is reported once, not
    /// once for each screen that inherits it, and lists no screens.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub network: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub venue: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub format: String,
    /// The screens the finding applies to, in document order, when it
    /// doesn't apply to every screen of the target: at most
    /// [`MAX_SCREENS_LISTED`].
    #[serde(default, skip_serializing_if = "ScreenList::is_empty")]
    pub screens: ScreenList,
    /// How many screens the finding applies to, set only when `screens` is
    /// cut short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screens_total: Option<u64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Who attested to a check that needs a person's judgement, and when.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<Attested>,
}

/// The most screens a finding lists. A finding on more says how many in
/// `screens_total`.
pub const MAX_SCREENS_LISTED: usize = 1000;

/// The ids of the screens a finding applies to. A finding on every screen of
/// a large network would otherwise hold its own copy of each id, so the lists
/// a validator builds are positions in one shared table of ids; the strings
/// are only read out when the list is written or looked at.
#[derive(Clone, Default)]
pub struct ScreenList(Repr);

#[derive(Clone)]
enum Repr {
    Owned(Vec<String>),
    Shared {
        table: Arc<Vec<String>>,
        at: Vec<u32>,
    },
}

impl Default for Repr {
    fn default() -> Self {
        Repr::Owned(Vec::new())
    }
}

impl ScreenList {
    /// The screens at positions `at` of `table`.
    pub(crate) fn shared(table: Arc<Vec<String>>, at: Vec<u32>) -> Self {
        ScreenList(Repr::Shared { table, at })
    }

    pub fn len(&self) -> usize {
        match &self.0 {
            Repr::Owned(v) => v.len(),
            Repr::Shared { at, .. } => at.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        let (owned, shared) = match &self.0 {
            Repr::Owned(v) => (Some(v), None),
            Repr::Shared { table, at } => (None, Some((table, at))),
        };
        owned.into_iter().flatten().map(String::as_str).chain(
            shared
                .into_iter()
                .flat_map(|(t, at)| at.iter().map(|&i| t[i as usize].as_str())),
        )
    }

    pub fn join(&self, sep: &str) -> String {
        self.iter().collect::<Vec<_>>().join(sep)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.iter().any(|s| s == id)
    }

    pub fn push(&mut self, id: String) {
        self.owned().push(id);
    }

    pub fn clear(&mut self) {
        self.0 = Repr::default();
    }

    pub fn to_vec(&self) -> Vec<String> {
        self.iter().map(String::from).collect()
    }

    fn owned(&mut self) -> &mut Vec<String> {
        if let Repr::Shared { .. } = self.0 {
            self.0 = Repr::Owned(self.to_vec());
        }
        match &mut self.0 {
            Repr::Owned(v) => v,
            Repr::Shared { .. } => unreachable!(),
        }
    }
}

impl From<Vec<String>> for ScreenList {
    fn from(v: Vec<String>) -> Self {
        ScreenList(Repr::Owned(v))
    }
}

impl PartialEq for ScreenList {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl PartialEq<[&str]> for ScreenList {
    fn eq(&self, other: &[&str]) -> bool {
        self.iter().eq(other.iter().copied())
    }
}

impl<const N: usize> PartialEq<[&str; N]> for ScreenList {
    fn eq(&self, other: &[&str; N]) -> bool {
        self.iter().eq(other.iter().copied())
    }
}

impl fmt::Debug for ScreenList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Serialize for ScreenList {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for ScreenList {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Vec::<String>::deserialize(d).map(ScreenList::from)
    }
}

/// Where a finding's screens are collected as positions in a table of ids,
/// one entry for each screen the validator went through. Screens of one
/// finding arrive in order, so a finding only has to remember the last
/// position it took to avoid listing a screen twice; ids that repeat in the
/// document are collapsed when the lists are made.
#[derive(Default)]
pub(crate) struct ScreenTable {
    ids: Vec<String>,
}

impl ScreenTable {
    /// Starts a screen and returns its position.
    pub(crate) fn begin(&mut self, id: &str) -> u32 {
        self.ids.push(id.to_string());
        (self.ids.len() - 1) as u32
    }

    /// Makes the lists for `found`, which are positions as `begin` gave them,
    /// each in order, with how many screens each has. A screen whose id came
    /// earlier in a list is dropped from it, and a list is cut to
    /// [`MAX_SCREENS_LISTED`].
    pub(crate) fn lists(self, found: Vec<Vec<u32>>) -> Vec<(ScreenList, usize)> {
        let mut seen = std::collections::HashSet::with_capacity(self.ids.len());
        let repeats = !self.ids.iter().all(|i| seen.insert(i.as_str()));
        drop(seen);
        let table = Arc::new(self.ids);
        found
            .into_iter()
            .map(|mut at| {
                if repeats {
                    let mut seen = std::collections::HashSet::new();
                    at.retain(|&i| seen.insert(table[i as usize].as_str()));
                }
                let total = at.len();
                at.truncate(MAX_SCREENS_LISTED);
                at.shrink_to_fit();
                (ScreenList::shared(table.clone(), at), total)
            })
            .collect()
    }
}

/// The attestation behind a finding, as the report shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attested {
    pub by: String,
    pub date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl Check {
    /// Where in a screens.json a finding is caused, as "network n, venue v",
    /// or "" for a finding about screens.
    pub fn scope_label(&self) -> String {
        let mut parts = Vec::new();
        for (name, id) in [
            ("network", &self.network),
            ("venue", &self.venue),
            ("format", &self.format),
        ] {
            if !id.is_empty() {
                parts.push(format!("{name} {id}"));
            }
        }
        parts.join(", ")
    }

    /// The screens a finding lists, comma separated, and how many more it
    /// applies to when the list is cut short; "" if it lists none.
    pub fn screens_label(&self) -> String {
        let mut out = self.screens.join(", ");
        if let Some(total) = self.screens_total {
            out += &format!(" and {} more", total - self.screens.len() as u64);
        }
        out
    }

    /// A check with only the required fields set.
    pub fn new(code: &str, severity: Severity, message: impl Into<String>) -> Self {
        Check {
            code: code.to_string(),
            severity,
            message: message.into(),
            rect_px: None,
            element: String::new(),
            network: String::new(),
            venue: String::new(),
            format: String::new(),
            screens: ScreenList::default(),
            screens_total: None,
            source: String::new(),
            attestation: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn check_serialises_like_go() {
        let mut c = Check::new(CODE_REFERENCE, Severity::NotChecked, "m");
        assert_eq!(
            json::to_string(&c),
            r#"{"code":"CRS-REFERENCE","severity":"not_checked","message":"m"}"#
        );
        c.rect_px = Some(Rect([1, 2, 3, 4]));
        c.element = "e".into();
        c.screens = vec!["s".to_string()].into();
        c.source = "file".into();
        assert_eq!(
            json::to_string(&c),
            r#"{"code":"CRS-REFERENCE","severity":"not_checked","message":"m","rect_px":[1,2,3,4],"element":"e","screens":["s"],"source":"file"}"#
        );
    }
}
