//! The conformance suite's types and comparison. Loading cases from disk and
//! running validators live in the CLI crate; this module is pure.

use std::path::PathBuf;

use serde::Deserialize;

use crate::gofmt::quote;

/// Levels in order: each includes the ones before it.
pub const LEVELS: [&str; 3] = ["core", "layout", "full"];

/// One test case, read from `<dir>/case.json`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Case {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub level: String,
    /// The spec section the case tests.
    #[serde(default)]
    pub spec: String,
    /// What it checks, in plain words.
    #[serde(default)]
    pub description: String,
    /// Validation time, RFC 3339.
    #[serde(default)]
    pub now: String,
    /// screens.json files, relative to the case.
    #[serde(default)]
    pub specs: Vec<String>,
    /// creative.json, relative to the case.
    #[serde(default)]
    pub creative: String,
    #[serde(default)]
    pub expect: Vec<ExpectedReport>,
    /// The case's directory; set by the loader, not read from JSON.
    #[serde(skip)]
    pub dir: PathBuf,
}

/// The part of a validation report a validator must get right.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExpectedReport {
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub checks: Vec<ExpectedCheck>,
}

/// The part of a finding a validator must get right. `element` and `screens`
/// are compared only where the case gives them; wording, rectangles and
/// sources are left to each implementation.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExpectedCheck {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub element: String,
    #[serde(default)]
    pub screens: Vec<String>,
    /// How many screens a finding with a cut-short list applies to.
    #[serde(default)]
    pub screens_total: u64,
}

impl Case {
    /// Whether the case belongs to a conformance level: a level includes every
    /// level before it. An unknown level sorts before all known ones, as in Go.
    pub fn in_level(&self, level: &str) -> bool {
        rank(&self.level) <= rank(level)
    }
}

fn rank(level: &str) -> isize {
    LEVELS
        .iter()
        .position(|l| *l == level)
        .map_or(-1, |i| i as isize)
}

// Nulls are tolerated like Go's decoder: a null leaves the zero value.
#[derive(Deserialize)]
struct ActualReport {
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    checks: Option<Vec<Option<ActualCheck>>>,
}

#[derive(Deserialize, Default)]
struct ActualCheck {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    element: Option<String>,
    #[serde(default)]
    screens: Option<Vec<String>>,
    #[serde(default)]
    screens_total: Option<u64>,
    #[serde(default)]
    message: Option<String>,
}

struct Report {
    file: String,
    target: String,
    result: String,
    checks: Vec<Check>,
}

struct Check {
    code: String,
    severity: String,
    element: String,
    screens: Vec<String>,
    screens_total: u64,
    message: String,
}

impl From<ActualCheck> for Check {
    fn from(c: ActualCheck) -> Self {
        Check {
            code: c.code.unwrap_or_default(),
            severity: c.severity.unwrap_or_default(),
            element: c.element.unwrap_or_default(),
            screens: c.screens.unwrap_or_default(),
            screens_total: c.screens_total.unwrap_or_default(),
            message: c.message.unwrap_or_default(),
        }
    }
}

impl From<ActualReport> for Report {
    fn from(r: ActualReport) -> Self {
        Report {
            file: r.file.unwrap_or_default(),
            target: r.target.unwrap_or_default(),
            result: r.result.unwrap_or_default(),
            checks: r
                .checks
                .unwrap_or_default()
                .into_iter()
                .map(|c| c.unwrap_or_default().into())
                .collect(),
        }
    }
}

/// Checks a validator's output against a case's expectations and returns
/// what's wrong, if anything:
///
/// - There must be exactly one report per expected (file, target) pair.
/// - Each report's result must match.
/// - Every expected check must be present; element and screens must match
///   where the case gives them, and a case that gives no screens expects the
///   finding to apply to every screen (no screens listed).
/// - Any other error, warning or not_checked finding is a failure. Extra info
///   findings are allowed.
pub fn compare(c: &Case, output: &[u8]) -> Vec<String> {
    let actual: Vec<Report> =
        match serde_json::from_slice::<Option<Vec<Option<ActualReport>>>>(output) {
            Ok(v) => v
                .unwrap_or_default()
                .into_iter()
                .map(|r| {
                    r.map_or_else(
                        || Report {
                            file: String::new(),
                            target: String::new(),
                            result: String::new(),
                            checks: vec![],
                        },
                        Into::into,
                    )
                })
                .collect(),
            Err(e) => return vec![format!("output isn't a JSON array of reports: {e}")],
        };
    let mut problems = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for want in &c.expect {
        let key = format!("{} → {}", want.file, want.target);
        seen.push(key.clone());
        let Some(got) = actual
            .iter()
            .find(|a| a.file == want.file && a.target == want.target)
        else {
            problems.push(format!("{key}: no report"));
            continue;
        };
        if got.result != want.result {
            problems.push(format!(
                "{key}: result {}, want {}",
                quote(&got.result),
                quote(&want.result)
            ));
        }
        let mut used = vec![false; got.checks.len()];
        for w in &want.checks {
            let found = got.checks.iter().enumerate().find(|(j, g)| {
                !used[*j]
                    && g.code == w.code
                    && g.severity == w.severity
                    && (w.element.is_empty() || g.element == w.element)
                    && same_set(&g.screens, &w.screens)
                    && g.screens_total == w.screens_total
            });
            match found {
                Some((j, _)) => used[j] = true,
                None => problems.push(format!(
                    "{key}: missing {}",
                    describe(&w.code, &w.severity, &w.element, &w.screens)
                )),
            }
        }
        for (j, g) in got.checks.iter().enumerate() {
            if !used[j] && g.severity != "info" {
                problems.push(format!(
                    "{key}: unexpected {} ({})",
                    describe(&g.code, &g.severity, &g.element, &g.screens),
                    quote(&g.message)
                ));
            }
        }
    }
    for a in &actual {
        let key = format!("{} → {}", a.file, a.target);
        if !seen.contains(&key) {
            problems.push(format!("{key}: unexpected report"));
        }
    }
    problems
}

fn describe(code: &str, severity: &str, element: &str, screens: &[String]) -> String {
    let mut s = format!("{code} {severity}");
    if !element.is_empty() {
        s.push_str(" on ");
        s.push_str(element);
    }
    if !screens.is_empty() {
        s.push_str(" [");
        s.push_str(&screens.join(", "));
        s.push(']');
    }
    s
}

fn same_set(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let (mut x, mut y) = (a.to_vec(), b.to_vec());
    x.sort();
    y.sort();
    x == y
}

#[cfg(test)]
mod tests {
    use super::*;

    const OTHER_PUBLISHER: &str =
        include_str!("../../../conformance/0.1/cases/core-other-publisher/case.json");

    fn case() -> Case {
        Case {
            expect: vec![ExpectedReport {
                file: "a.png".into(),
                target: "p.example/n/f".into(),
                result: "fail".into(),
                checks: vec![ExpectedCheck {
                    code: "CRS-LEGIBILITY".into(),
                    severity: "error".into(),
                    element: "legal".into(),
                    screens: vec!["S2".into()],
                    screens_total: 0,
                }],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn levels() {
        let c = Case {
            level: "layout".into(),
            ..Default::default()
        };
        assert!(!c.in_level("core") && c.in_level("layout") && c.in_level("full"));
    }

    #[test]
    fn case_json_parses() {
        let c: Case = serde_json::from_str(OTHER_PUBLISHER).unwrap();
        assert_eq!(c.id, "core-other-publisher");
        assert_eq!(c.level, "core");
        assert_eq!(c.specs, ["screens.json"]);
        assert_eq!(c.creative, "creative.json");
        assert!(c.expect.is_empty());
        assert!(LEVELS.contains(&c.level.as_str()));
    }

    #[test]
    fn compare_accepts_good_output() {
        let good = r#"[{"file":"a.png","target":"p.example/n/f","result":"fail","checks":[
          {"code":"CRS-LEGIBILITY","severity":"error","element":"legal","screens":["S2"],"message":"any wording"},
          {"code":"CRS-PLAYBACK","severity":"info","message":"extra info is fine"}]}]"#;
        assert_eq!(compare(&case(), good.as_bytes()), Vec::<String>::new());
    }

    #[test]
    fn compare_catches_every_way_to_be_wrong() {
        let bad = [
            (
                "wrong result",
                r#"[{"file":"a.png","target":"p.example/n/f","result":"warn","checks":[{"code":"CRS-LEGIBILITY","severity":"error","element":"legal","screens":["S2"]}]}]"#,
            ),
            (
                "wrong screens",
                r#"[{"file":"a.png","target":"p.example/n/f","result":"fail","checks":[{"code":"CRS-LEGIBILITY","severity":"error","element":"legal"}]}]"#,
            ),
            (
                "wrong severity",
                r#"[{"file":"a.png","target":"p.example/n/f","result":"fail","checks":[{"code":"CRS-LEGIBILITY","severity":"warning","element":"legal","screens":["S2"]}]}]"#,
            ),
            (
                "extra error",
                r#"[{"file":"a.png","target":"p.example/n/f","result":"fail","checks":[{"code":"CRS-LEGIBILITY","severity":"error","element":"legal","screens":["S2"]},{"code":"CRS-PIXELS","severity":"error"}]}]"#,
            ),
            ("missing report", "[]"),
            (
                "unexpected report",
                r#"[{"file":"a.png","target":"p.example/n/f","result":"fail","checks":[{"code":"CRS-LEGIBILITY","severity":"error","element":"legal","screens":["S2"]}]},{"file":"b.png","target":"p.example/n/f","result":"pass","checks":[]}]"#,
            ),
            ("not json", "oops"),
        ];
        for (name, out) in bad {
            assert!(
                !compare(&case(), out.as_bytes()).is_empty(),
                "{name}: should fail"
            );
        }
    }

    #[test]
    fn compare_messages_match_go() {
        let out = r#"[{"file":"a.png","target":"p.example/n/f","result":"warn","checks":[{"code":"CRS-LEGIBILITY","severity":"error","element":"legal"},{"code":"CRS-PIXELS","severity":"error","screens":["S1","S2"],"message":"too \"big\""}]}]"#;
        assert_eq!(
            compare(&case(), out.as_bytes()),
            [
                "a.png → p.example/n/f: result \"warn\", want \"fail\"",
                "a.png → p.example/n/f: missing CRS-LEGIBILITY error on legal [S2]",
                "a.png → p.example/n/f: unexpected CRS-LEGIBILITY error on legal (\"\")",
                "a.png → p.example/n/f: unexpected CRS-PIXELS error [S1, S2] (\"too \\\"big\\\"\")",
            ]
        );
        assert_eq!(
            compare(&case(), b"[]"),
            ["a.png → p.example/n/f: no report"]
        );
        let p = compare(&case(), b"oops");
        assert!(p[0].starts_with("output isn't a JSON array of reports: "));
    }

    #[test]
    fn compare_screens_are_a_set_and_one_check_matches_once() {
        let mut c = case();
        c.expect[0].checks[0].screens = vec!["S1".into(), "S2".into()];
        c.expect[0].checks[0].element = String::new();
        let out = r#"[{"file":"a.png","target":"p.example/n/f","result":"fail","checks":[{"code":"CRS-LEGIBILITY","severity":"error","element":"x","screens":["S2","S1"]}]}]"#;
        assert!(compare(&c, out.as_bytes()).is_empty());
        let dup = r#"[{"file":"a.png","target":"p.example/n/f","result":"fail","checks":[{"code":"CRS-LEGIBILITY","severity":"error","screens":["S2","S1"]},{"code":"CRS-LEGIBILITY","severity":"error","screens":["S2","S1"]}]}]"#;
        assert_eq!(compare(&c, dup.as_bytes()).len(), 1);
    }

    #[test]
    fn null_output_is_an_empty_list() {
        let c = Case::default();
        assert!(compare(&c, b"null").is_empty());
    }
}
