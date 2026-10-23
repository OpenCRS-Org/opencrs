//! Check findings: severities, standard codes and the `Check` record.
//! Port of the types and constants at the top of `crs/check.go`.

use crate::model::Rect;
use serde::{Deserialize, Serialize};

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screens: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

impl Check {
    /// A check with only the required fields set.
    pub fn new(code: &str, severity: Severity, message: impl Into<String>) -> Self {
        Check {
            code: code.to_string(),
            severity,
            message: message.into(),
            rect_px: None,
            element: String::new(),
            screens: Vec::new(),
            source: String::new(),
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
        c.screens = vec!["s".into()];
        c.source = "file".into();
        assert_eq!(
            json::to_string(&c),
            r#"{"code":"CRS-REFERENCE","severity":"not_checked","message":"m","rect_px":[1,2,3,4],"element":"e","screens":["s"],"source":"file"}"#
        );
    }
}
