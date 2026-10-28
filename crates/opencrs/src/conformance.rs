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
    /// For a case about strictest requirements instead of a creative: the
    /// references to resolve in `specs[0]`, and what each must give. Such a
    /// case has no `creative`, and the validator prints one result for each
    /// entry, in order, as a JSON array.
    #[serde(default)]
    pub strictest: Vec<ExpectedStrictest>,
    /// The optional feature the case tests, such as `strictest`; empty for
    /// a case every validator at its level must pass. A validator that
    /// doesn't offer the feature is not tested on the case.
    #[serde(default)]
    pub optional: String,
    /// The case's directory; set by the loader, not read from JSON.
    #[serde(skip)]
    pub dir: PathBuf,
}

/// What resolving a reference for its strictest requirements must give.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExpectedStrictest {
    #[serde(rename = "ref", default)]
    pub reference: String,
    /// Members the result must have, with these values: objects are matched
    /// by their members (others are not looked at), everything else exactly.
    /// Numbers are equal if their values are.
    #[serde(default)]
    pub expect: serde_json::Value,
    /// Members (dotted paths such as `media.motion`) the result must not have.
    #[serde(default)]
    pub absent: Vec<String>,
    /// How many `notes` the result must have; their wording is left to each
    /// validator. Absent means they aren't counted.
    #[serde(default)]
    pub notes: Option<usize>,
    /// Instead of a result, the error: the result must be `{"error": text}`
    /// with text containing this.
    #[serde(default)]
    pub error: String,
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
    /// Whether the case is for an optional feature, not a conformance level.
    pub fn is_optional(&self) -> bool {
        !self.optional.is_empty()
    }

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
    if !c.strictest.is_empty() {
        return compare_strictest(c, output);
    }
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

/// Checks a validator's strictest results against a case: one for each
/// entry, in order, each either containing what is expected or being the
/// expected error.
fn compare_strictest(c: &Case, output: &[u8]) -> Vec<String> {
    let actual: Vec<serde_json::Value> = match serde_json::from_slice(output) {
        Ok(v) => v,
        Err(e) => return vec![format!("output isn't a JSON array of results: {e}")],
    };
    if actual.len() != c.strictest.len() {
        return vec![format!(
            "{} results, want {}",
            actual.len(),
            c.strictest.len()
        )];
    }
    let mut problems = Vec::new();
    for (want, got) in c.strictest.iter().zip(&actual) {
        let r = &want.reference;
        if want.expect.is_null() && want.notes.is_none() && want.error.is_empty() {
            problems.push(format!("{r}: the case expects nothing of the result"));
            continue;
        }
        let err = got.get("error").and_then(|e| e.as_str());
        if !want.error.is_empty() {
            match err {
                Some(e) if e.contains(&want.error) => {}
                Some(e) => problems.push(format!(
                    "{r}: error {}, want one containing {}",
                    quote(e),
                    quote(&want.error)
                )),
                None => problems.push(format!(
                    "{r}: no error, want one containing {}",
                    quote(&want.error)
                )),
            }
            continue;
        }
        if let Some(e) = err {
            problems.push(format!("{r}: error {}", quote(e)));
            continue;
        }
        within(&want.expect, got, "", r, &mut problems);
        for path in &want.absent {
            let mut at = Some(got);
            for k in path.split('.') {
                at = at.and_then(|v| v.get(k));
            }
            if at.is_some() {
                problems.push(format!("{r}: {path} is there, want it left out"));
            }
        }
        if let Some(n) = want.notes {
            let have = got
                .get("notes")
                .and_then(|n| n.as_array())
                .map_or(0, Vec::len);
            if have != n {
                problems.push(format!("{r}: {have} notes, want {n}"));
            }
        }
    }
    problems
}

/// Whether `got` holds what `want` says, adding to `problems` where not.
fn within(
    want: &serde_json::Value,
    got: &serde_json::Value,
    at: &str,
    r: &str,
    problems: &mut Vec<String>,
) {
    use serde_json::Value::{Array, Null, Number, Object};
    let here = if at.is_empty() { "the result" } else { at };
    match (want, got) {
        (Null, _) => {}
        (Object(w), Object(g)) => {
            for (k, wv) in w {
                let path = if at.is_empty() {
                    k.clone()
                } else {
                    format!("{at}.{k}")
                };
                match g.get(k) {
                    Some(gv) => within(wv, gv, &path, r, problems),
                    None => problems.push(format!("{r}: {path} is missing, want {wv}")),
                }
            }
        }
        (Array(w), Array(g)) => {
            if w.len() != g.len() {
                problems.push(format!("{r}: {here} is {got}, want {want}"));
                return;
            }
            for (i, (wv, gv)) in w.iter().zip(g).enumerate() {
                within(wv, gv, &format!("{at}[{i}]"), r, problems);
            }
        }
        (Number(w), Number(g)) if w.as_f64() == g.as_f64() => {}
        (w, g) if w == g => {}
        (w, g) => problems.push(format!("{r}: {here} is {g}, want {w}")),
    }
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

    fn strictest_case() -> Case {
        serde_json::from_str(
            r#"{"id":"full-x","strictest":[
              {"ref":"p.example/n/f","notes":1,"expect":{"screens":2,"safe_area":{"top":5},"media":{"static":[]}}},
              {"ref":"p.example/n/g","error":"pixel"}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn strictest_results_are_matched_by_what_the_case_gives() {
        let good = r#"[{"screens":2.0,"extra":1,"notes":["any wording"],"safe_area":{"top":5,"left":1},"media":{"static":[]}},
            {"error":"different pixel sizes"}]"#;
        assert_eq!(
            compare(&strictest_case(), good.as_bytes()),
            Vec::<String>::new()
        );
        let empty: Case = serde_json::from_str(r#"{"strictest":[{"ref":"a/b/c"}]}"#).unwrap();
        assert!(!compare(&empty, br#"[{"screens":1}]"#).is_empty());
        let bad = [
            ("count", r#"[{}]"#),
            ("not json", "oops"),
            (
                "value",
                r#"[{"screens":3,"notes":["x"],"safe_area":{"top":5},"media":{"static":[]}},{"error":"pixel"}]"#,
            ),
            (
                "missing",
                r#"[{"screens":2,"notes":["x"],"media":{"static":[]}},{"error":"pixel"}]"#,
            ),
            (
                "list",
                r#"[{"screens":2,"notes":["x"],"safe_area":{"top":5},"media":{"static":["a"]}},{"error":"pixel"}]"#,
            ),
            (
                "notes",
                r#"[{"screens":2,"safe_area":{"top":5},"media":{"static":[]}},{"error":"pixel"}]"#,
            ),
            (
                "error wanted",
                r#"[{"screens":2,"notes":["x"],"safe_area":{"top":5},"media":{"static":[]}},{"screens":1}]"#,
            ),
            ("error unwanted", r#"[{"error":"no"},{"error":"pixel"}]"#),
        ];
        for (name, out) in bad {
            assert!(
                !compare(&strictest_case(), out.as_bytes()).is_empty(),
                "{name}: should fail"
            );
        }
    }

    #[test]
    fn null_output_is_an_empty_list() {
        let c = Case::default();
        assert!(compare(&c, b"null").is_empty());
    }
}
