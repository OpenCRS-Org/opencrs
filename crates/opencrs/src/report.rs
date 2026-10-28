//! Reports: whether one creative file meets one target's requirements.
//! Port of `crs/report.go`.

use crate::SPEC_VERSION;
use crate::check::*;
use crate::checks::{Checker, Facts, gather_facts};
use crate::error::Error;
use crate::model::{Campaign, CreativeFile, Document};
use crate::probe::Probe;
use crate::resolve::{Requirements, ResolveError, parse_ref};
use crate::rulepack::{RulePacks, builtin_rule_packs};
use chrono::{DateTime, FixedOffset, SecondsFormat};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

/// A report's overall outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Pass,
    Warn,
    Fail,
    Incomplete,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::Warn => "warn",
            Outcome::Fail => "fail",
            Outcome::Incomplete => "incomplete",
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Names the validator that wrote a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorInfo {
    pub name: String,
    pub version: String,
}

impl Default for ValidatorInfo {
    /// This reference validator.
    fn default() -> Self {
        ValidatorInfo {
            name: "opencrs".into(),
            version: SPEC_VERSION.into(),
        }
    }
}

/// Says whether one creative file meets one target's requirements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub opencrs: String,
    pub validator: ValidatorInfo,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_sha256: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub spec_updated_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checked_at: String,
    pub result: Outcome,
    /// Always serialised, as `[]` when there are no findings.
    pub checks: Vec<Check>,
}

/// Text this close above its minimum (minimum x 1.25) passes with a warning,
/// as the spec states. Its example warns at 18 px against 16 px.
pub const DEFAULT_WARN_MARGIN: f64 = 1.25;

/// Controls validation.
#[derive(Debug, Clone)]
pub struct Options<'a> {
    /// The time of the check, for the stale-spec check and `checked_at`.
    /// The core never reads a clock, so the caller supplies it.
    pub now: DateTime<FixedOffset>,
    pub warn_margin: f64,
    pub validator: ValidatorInfo,
    /// The regulatory rule packs the validator knows.
    pub rule_packs: &'a RulePacks,
}

impl Options<'static> {
    /// Options with the defaults: the standard warn margin, this validator
    /// and the rule packs embedded in it.
    pub fn new(now: DateTime<FixedOffset>) -> Self {
        Options {
            now,
            warn_margin: DEFAULT_WARN_MARGIN,
            validator: ValidatorInfo::default(),
            rule_packs: builtin_rule_packs(),
        }
    }
}

/// One creative file to validate.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub file: &'a CreativeFile,
    pub campaign: Option<&'a Campaign>,
    /// None when only the manifest is available.
    pub probe: Option<&'a Probe>,
}

/// Why `validate` produced no report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidateError {
    /// The target belongs to a different publisher's document. It isn't a
    /// failure: the creative is simply also meant for screens this document
    /// doesn't describe. Go's `ErrOtherPublisher`.
    OtherPublisher(String),
    /// The target isn't a format or screen reference.
    BadTarget(String),
}

impl fmt::Display for ValidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidateError::OtherPublisher(t) => {
                write!(f, "target belongs to another publisher: {t}")
            }
            ValidateError::BadTarget(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ValidateError {}

impl From<ValidateError> for Error {
    fn from(e: ValidateError) -> Self {
        Error::Message(e.to_string())
    }
}

/// Checks one creative file against one target in `doc`.
pub fn validate(
    doc: &Document,
    input: &Input,
    target: &str,
    opt: &Options,
) -> Result<Report, ValidateError> {
    crate::budget::ensure(doc.size, || validate_within(doc, input, target, opt))
}

fn validate_within(
    doc: &Document,
    input: &Input,
    target: &str,
    opt: &Options,
) -> Result<Report, ValidateError> {
    let r = parse_ref(target).map_err(|e| ValidateError::BadTarget(e.to_string()))?;
    if r.publisher != doc.publisher {
        return Err(ValidateError::OtherPublisher(target.to_string()));
    }

    let (facts, mut checks) = gather_facts(input.file, input.probe);
    let sha256 = facts.sha256.clone();
    checks.extend(stale_check(doc, &opt.now));

    // Screens with the same requirements (apart from their id) are checked
    // once, and each screen's findings are merged as the screens come, so a
    // target of hundreds of thousands of screens costs by what is different
    // between them, not by how many there are.
    let mut merger = Merger::default();
    let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut key = Vec::new();
    let resolved = doc.resolve_each(&r, |req| {
        key.clear();
        let keyed = req.key(&mut key);
        let group = match seen.get(key.as_slice()) {
            Some(&g) if keyed => g,
            _ => {
                let g = merger.new_group(run_checks(&req, input, &facts, opt));
                if keyed {
                    seen.insert(key.clone(), g);
                }
                g
            }
        };
        merger.add(group, req.screen_id());
    });
    match resolved {
        Err(e) => {
            let (code, severity) = match e {
                ResolveError::Reference(_) => (CODE_REFERENCE, Severity::Error),
                ResolveError::Other(_) => (CODE_TARGET, Severity::Error),
                ResolveError::TooComplex(_) => (CODE_TOO_COMPLEX, Severity::NotChecked),
            };
            checks.push(Check::new(code, severity, e.to_string()));
        }
        Ok(()) => {
            checks.extend(merger.finish());
            // The work ran out while merging what was found.
            if let Some(why) = crate::budget::explain(Some(doc)) {
                checks.push(Check::too_complex(why));
            }
        }
    }

    let result = outcome(&checks);
    Ok(Report {
        opencrs: SPEC_VERSION.to_string(),
        validator: opt.validator.clone(),
        file: input.file.file.clone(),
        file_sha256: sha256,
        target: target.to_string(),
        spec_updated_at: doc.updated_at.clone(),
        checked_at: opt.now.to_rfc3339_opts(SecondsFormat::Secs, true),
        result,
        checks,
    })
}

fn run_checks(req: &Requirements, input: &Input, facts: &Facts, opt: &Options) -> Vec<Check> {
    // The inherited rule items each get looked at for this kind of screen.
    let origin = req
        .network
        .format(req.format_id)
        .map_or(req.network as *const _ as usize, |f| f as *const _ as usize);
    let items = req.rules.required_elements.len()
        + req.rules.rule_packs.len()
        + req.rules.restricted_categories.len();
    if !crate::budget::charge(
        crate::budget::Work::RuleCheck,
        origin,
        (items + 1) as u64 * 8,
    ) {
        return Vec::new();
    }
    Checker {
        req,
        file: input.file,
        campaign: input.campaign,
        facts: facts.clone(),
        margin: opt.warn_margin,
        rule_packs: opt.rule_packs,
        out: Vec::new(),
    }
    .run()
}

fn stale_check(doc: &Document, now: &DateTime<FixedOffset>) -> Vec<Check> {
    if doc.expires_at.is_empty() {
        return Vec::new();
    }
    let Ok(exp) = DateTime::parse_from_rfc3339(&doc.expires_at) else {
        return vec![Check::new(
            CODE_SPEC_STALE,
            Severity::NotChecked,
            format!("Can't read expires_at: {}", doc.expires_at),
        )];
    };
    // Compared as instants, so offsets don't matter.
    if *now > exp {
        return vec![Check::new(
            CODE_SPEC_STALE,
            Severity::Error,
            format!(
                "The spec expired at {}; fetch a current copy before validating",
                doc.expires_at
            ),
        )];
    }
    Vec::new()
}

/// Collapses identical findings from several screens into one, listing the
/// screens it applies to. A finding that applies to every screen of the
/// target lists none.
///
/// Screens come in groups with the same findings, which are worked out once
/// (`new_group`); a screen is then added to its group (`add`) at the cost of
/// remembering its position. Each finding keeps the groups it is in, and the
/// screens it lists are read off those groups at the end, so a finding on
/// every screen of a large target never holds a list of them.
#[derive(Default)]
struct Merger {
    index: HashMap<MergeKey, usize>,
    out: Vec<Check>,
    /// For each finding, the groups it is in.
    groups_of: Vec<Vec<u32>>,
    /// For each group, its checks, until a screen of it is first added.
    pending: Vec<Option<Vec<Check>>>,
    /// For each group, the positions of its screens, in order.
    members: Vec<Vec<u32>>,
    table: ScreenTable,
    screens: usize,
}

type MergeKey = (String, String, String, String, Severity, Option<[i64; 4]>);

impl Merger {
    /// Starts a group whose screens all have `checks`.
    fn new_group(&mut self, checks: Vec<Check>) -> usize {
        self.pending.push(Some(checks));
        self.members.push(Vec::new());
        self.pending.len() - 1
    }

    fn add(&mut self, group: usize, id: &str) {
        crate::budget::charge_flat(4);
        self.screens += 1;
        let pos = self.table.begin(id);
        if let Some(checks) = self.pending[group].take() {
            self.enter(group, checks);
        }
        // A screen with no id can't be listed, but still counts as a screen.
        if !id.is_empty() {
            self.members[group].push(pos);
        }
    }

    /// Merges a group's checks into the findings, the first time a screen of
    /// it arrives, which is when the findings are first seen.
    fn enter(&mut self, group: usize, checks: Vec<Check>) {
        // Each finding of the group is noted against it...
        crate::budget::charge(crate::budget::Work::Findings, 0, checks.len() as u64 * 8);
        if crate::budget::tripped() {
            return;
        }
        for c in checks {
            let key = (
                c.code.clone(),
                c.message.clone(),
                c.element.clone(),
                c.source.clone(),
                c.severity,
                c.rect_px.map(|r| r.0),
            );
            let i = match self.index.get(&key) {
                Some(&i) => i,
                None => {
                    // ...and a new one costs about what it takes to write.
                    crate::budget::charge(crate::budget::Work::Findings, 0, 256);
                    self.index.insert(key, self.out.len());
                    let mut cc = c;
                    cc.screens = ScreenList::default();
                    self.out.push(cc);
                    self.groups_of.push(Vec::new());
                    self.out.len() - 1
                }
            };
            if self.groups_of[i].last() != Some(&(group as u32)) {
                self.groups_of[i].push(group as u32);
            }
        }
    }

    fn finish(self) -> Vec<Check> {
        let Merger {
            mut out,
            groups_of,
            members,
            table,
            screens,
            ..
        } = self;
        let (ids, repeats) = table.into_ids();
        let ids = Arc::new(ids);
        // Findings in the same groups list the same screens.
        let mut lists: HashMap<Vec<u32>, (ScreenList, usize)> = HashMap::new();
        for (c, mut groups) in out.iter_mut().zip(groups_of) {
            groups.sort_unstable();
            let (list, total) = lists
                .entry(groups)
                .or_insert_with_key(|g| screens_in(g, &members, &ids, repeats))
                .clone();
            // A finding on every screen of the target lists none.
            if total == screens {
                continue;
            }
            c.screens_total = (total > list.len()).then_some(total as u64);
            c.screens = list;
        }
        out
    }
}

/// The screens in `groups`, in document order, cut to the limit, and how
/// many there are, counting each id once.
fn screens_in(
    groups: &[u32],
    members: &[Vec<u32>],
    ids: &Arc<Vec<String>>,
    repeats: bool,
) -> (ScreenList, usize) {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    // Merge the groups' positions, which are each in order.
    let mut heap: BinaryHeap<Reverse<(u32, u32, usize)>> = BinaryHeap::new();
    for &g in groups {
        if let Some(&p) = members[g as usize].first() {
            heap.push(Reverse((p, g, 0)));
        }
    }
    let mut at: Vec<u32> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut total = 0usize;
    // Without repeated ids, the total is a sum, and only the first positions
    // are read; with them, every position is.
    if !repeats {
        total = groups.iter().map(|&g| members[g as usize].len()).sum();
    }
    while let Some(Reverse((p, g, i))) = heap.pop() {
        let fresh = !repeats || seen.insert(ids[p as usize].as_str());
        if fresh {
            total += repeats as usize;
            if at.len() < MAX_SCREENS_LISTED {
                at.push(p);
            } else if !repeats {
                break;
            }
        }
        if let Some(&next) = members[g as usize].get(i + 1) {
            heap.push(Reverse((next, g, i + 1)));
        }
    }
    (ScreenList::shared(ids.clone(), at), total)
}

/// Applies the spec's precedence: any error fails; otherwise any unchecked
/// rule makes the report incomplete; otherwise any warning warns.
pub fn outcome(checks: &[Check]) -> Outcome {
    let mut has_not_checked = false;
    let mut has_warning = false;
    for c in checks {
        match c.severity {
            Severity::Error => return Outcome::Fail,
            Severity::NotChecked => has_not_checked = true,
            Severity::Warning => has_warning = true,
            Severity::Info => {}
        }
    }
    if has_not_checked {
        Outcome::Incomplete
    } else if has_warning {
        Outcome::Warn
    } else {
        Outcome::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checking screens with the same requirements once gives the report
    /// that checking every screen does, over screens that differ in what
    /// the checks read (viewing, overrides, venue, own rules), in what they
    /// don't (placement.zone), in ways the fields' JSON wouldn't show (an
    /// override of `[]`, a dwell of -0), with own rules that conflict (so
    /// their findings name them), and with repeated and empty ids.
    #[test]
    fn grouped_checks_match_checking_every_screen() {
        use crate::testutil::{lunch_file, offer_campaign};
        let mut d = harbourline();
        d.networks[0].rules.as_mut().unwrap().required_elements[0].within_rect_px =
            Some(Rect([0, 1600, 1080, 320]));
        let base = d.networks[0].screens[1].clone();
        for i in 0..120 {
            let mut s = base.clone();
            s.id = format!("X{}", i % 80); // some ids repeat
            s.platform_ids = Default::default();
            if i % 3 == 0 {
                s.viewing.as_mut().unwrap().distance_m = Some([1.0 + (i % 7) as f64, 12.0]);
            }
            if i % 5 == 0 {
                s.overrides = None;
            }
            if i % 7 == 0 {
                s.venue = String::new();
            }
            if i % 11 == 0 {
                s.rules = Some(Box::new(RuleSet {
                    max_words: Some(3),
                    ..Default::default()
                }));
            }
            if i % 13 == 0 {
                s.placement.get_or_insert_with(Default::default).zone = format!("zone {i}");
            }
            if i % 17 == 0 {
                s.overrides = Some(Box::new(
                    serde_json::from_str(r#"{"media":{"static":[]}}"#).unwrap(),
                ));
            }
            if i % 19 == 0 {
                s.rules = Some(Box::new(RuleSet {
                    required_elements: vec![RequiredElement {
                        role: "legal".into(),
                        when: "offer".into(),
                        within_rect_px: Some(Rect([0, 0, 1080, 300])),
                        ..Default::default()
                    }],
                    ..Default::default()
                }));
            }
            if i % 23 == 0 {
                s.viewing.as_mut().unwrap().dwell_s = Some(-0.0);
            }
            if i == 40 {
                s.id = String::new();
            }
            d.networks[0].screens.push(s);
        }
        let file = lunch_file();
        let campaign = offer_campaign();
        let input = Input {
            file: &file,
            campaign: Some(&campaign),
            probe: None,
        };
        let opt = Options::new(test_now());
        let fmt = &d.networks[0].screens[1].format;
        let target = format!("harbourline.example/retail/{fmt}");
        let grouped = validate(&d, &input, &target, &opt).unwrap();
        // The same, a group for every screen.
        let r = parse_ref(&target).unwrap();
        let (facts, _) = gather_facts(input.file, input.probe);
        let mut m = Merger::default();
        d.resolve_each(&r, |req| {
            let g = m.new_group(run_checks(&req, &input, &facts, &opt));
            m.add(g, req.screen_id());
        })
        .unwrap();
        let each = m.finish();
        assert!(!each.is_empty());
        let got: Vec<_> = grouped
            .checks
            .iter()
            .filter(|c| !c.screens.is_empty() || c.code != "CRS-FILE-SIZE")
            .collect();
        let want = |c: &Check| each.iter().any(|e| e == c);
        assert!(got.iter().all(|c| want(c)), "{got:?}\nvs\n{each:?}");
        for e in &each {
            assert!(grouped.checks.contains(e), "missing {e:?}");
        }
    }

    /// An override that clears a list (`"static": []`) and one that leaves
    /// it alone (`{}`) resolve differently, so their screens are not one
    /// group, though both serialise to `{}`.
    #[test]
    fn overrides_that_clear_a_list_are_not_grouped_with_those_that_do_not() {
        use crate::testutil::{check_doc, find, lunch_file, offer_campaign};
        let mut d = harbourline();
        let n = &mut d.networks[0];
        let mut a = n.screens[0].clone();
        a.id = "A".into();
        a.overrides = Some(Box::new(
            serde_json::from_str(r#"{"media":{"static":[]},"segments":[]}"#).unwrap(),
        ));
        let mut b = a.clone();
        b.id = "B".into();
        b.overrides = Some(Box::new(serde_json::from_str(r#"{"media":{}}"#).unwrap()));
        let target = format!("harbourline.example/retail/{}", a.format);
        n.screens = vec![a, b];
        let rep = check_doc(&d, &lunch_file(), Some(&offer_campaign()), &target);
        let got = find(&rep, CODE_MEDIA_TYPE, Severity::Error);
        assert_eq!(got.len(), 1, "{:?}", rep.checks);
        assert_eq!(got[0].screens, ["A"], "only A clears the static types");
    }

    /// A conflict a screen's own rules cause names the screen, so screens
    /// that are otherwise alike get their own findings, as checking each
    /// screen gives.
    #[test]
    fn screens_with_their_own_conflicting_rules_are_reported_apart() {
        use crate::testutil::{check_doc, find, lunch_file, offer_campaign};
        let mut d = harbourline();
        d.networks[0].rules.as_mut().unwrap().required_elements[0].within_rect_px =
            Some(Rect([0, 1600, 1080, 320]));
        let own = RuleSet {
            required_elements: vec![RequiredElement {
                role: "legal".into(),
                when: "offer".into(),
                within_rect_px: Some(Rect([0, 0, 1080, 300])),
                ..Default::default()
            }],
            ..Default::default()
        };
        let n = &mut d.networks[0];
        let mut a = n.screens[0].clone();
        a.id = "A".into();
        a.rules = Some(Box::new(own));
        let mut b = a.clone();
        b.id = "B".into();
        let target = format!("harbourline.example/retail/{}", a.format);
        n.screens = vec![a, b];
        let rep = check_doc(&d, &lunch_file(), Some(&offer_campaign()), &target);
        let got = find(&rep, CODE_RULE_CONFLICT, Severity::Error);
        assert_eq!(got.len(), 2, "{:?}", rep.checks);
        for id in ["A", "B"] {
            assert!(
                got.iter()
                    .any(|c| c.message.starts_with(&format!("screen {id} restates"))
                        && c.screens == [id]),
                "{id}: {got:?}"
            );
        }
    }

    #[test]
    fn merging_lists_each_screen_once_and_drops_a_full_list() {
        let finding = |m: &str| vec![Check::new("CRS-X", Severity::Error, m)];
        let mut m = Merger::default();
        let g1 = m.new_group([finding("all"), finding("some")].concat());
        let g2 = m.new_group([finding("all"), finding("all")].concat());
        m.add(g1, "a");
        m.add(g2, "b");
        let out = m.finish();
        assert_eq!(out.len(), 2);
        // "all" is on both screens, so it lists none; "some" lists its one.
        assert!(out[0].screens.is_empty(), "{:?}", out[0].screens);
        assert_eq!(out[1].screens, ["a"]);
    }
    use crate::json;
    use crate::model::*;
    use crate::schema::{Kind, validate_schema};
    use crate::testutil::*;

    const HC1: &str = "harbourline.example/retail/screen/HC-L1-001";
    const TOTEM: &str = "harbourline.example/retail/portrait-totem-75";

    fn first<'a>(cs: &[&'a Check]) -> &'a Check {
        assert_eq!(cs.len(), 1, "{cs:?}");
        cs[0]
    }

    // The same 18 px legal line warns on HC-L1-001 (16 px minimum at 2 m) and
    // fails on HC-L1-002 (23 px at 3 m). The headline warns on HC-L1-002 only.
    #[test]
    fn same_creative_differs_by_screen() {
        let c = offer_campaign();
        let rep = check(&lunch_file(), Some(&c), TOTEM);
        assert_eq!(rep.result, Outcome::Fail);
        let errs = find(&rep, CODE_LEGIBILITY, Severity::Error);
        let e = first(&errs);
        assert_eq!(e.element, "legal");
        assert_eq!(e.screens, ["HC-L1-002"]);
        let warn = find(&rep, CODE_LEGIBILITY, Severity::Warning);
        assert!(
            warn.iter()
                .any(|w| w.element == "legal" && w.screens == ["HC-L1-001"])
        );
        assert!(
            warn.iter()
                .any(|w| w.element == "headline" && w.screens == ["HC-L1-002"])
        );
        assert_eq!(
            e.message,
            "Legal text is 18 px; 23 px minimum at 3 m, ratio 150"
        );
    }

    #[test]
    fn clean_creative_on_one_screen_warns() {
        let c = offer_campaign();
        let rep = check(&lunch_file(), Some(&c), HC1);
        assert_eq!(rep.result, Outcome::Warn, "{:?}", rep.checks);
        assert_eq!(rep.checks.len(), 1);
        assert!(
            rep.checks[0].screens.is_empty(),
            "a single-screen target shouldn't list screens"
        );
    }

    // The legal minimum on HC-L1-001 is 15.3 px. Rounding to the nearest
    // pixel would let 15 px text pass; minimums round up, so it fails.
    #[test]
    fn just_below_minimum_fails() {
        let mut f = lunch_file();
        f.elements[1].cap_height_px = Some(15.0);
        let rep = check(&f, Some(&offer_campaign()), HC1);
        let got = find(&rep, CODE_LEGIBILITY, Severity::Error);
        assert_eq!(
            first(&got).message,
            "Legal text is 15 px; 16 px minimum at 2 m, ratio 150"
        );
    }

    // 20 px is 1.25 x the 16 px minimum: the first size without a warning.
    #[test]
    fn bigger_legal_passes() {
        let mut f = lunch_file();
        f.elements[1].cap_height_px = Some(20.0);
        let rep = check(&f, Some(&offer_campaign()), HC1);
        assert_eq!(rep.result, Outcome::Pass, "{:?}", rep.checks);
    }

    #[test]
    fn layout_errors() {
        let mut f = lunch_file();
        f.elements[2].rect_px = Some(Rect([820, 100, 200, 60])); // logo top-right
        let rep = check(&f, Some(&offer_campaign()), HC1);
        let br = find(&rep, CODE_BLOCKED_REGION, Severity::Error);
        let b = first(&br);
        assert_eq!(b.message, "Logo overlaps the network-logo area");
        assert_eq!(b.rect_px, Some(Rect([880, 40, 160, 80])));
        let sa = find(&rep, CODE_SAFE_AREA, Severity::Error);
        assert_eq!(
            first(&sa).message,
            "Logo extends into the safe-area margin (top 120 px)"
        );
    }

    // HC-L1-002's column override blocks its bottom-left corner, where the
    // legal line sits.
    #[test]
    fn column_override_blocks_legal() {
        let mut f = lunch_file();
        f.elements[1].rect_px = Some(Rect([90, 1700, 520, 40]));
        let rep = check(
            &f,
            Some(&offer_campaign()),
            "harbourline.example/retail/screen/HC-L1-002",
        );
        let br = find(&rep, CODE_BLOCKED_REGION, Severity::Error);
        assert_eq!(first(&br).element, "legal");
    }

    #[test]
    fn core_checks() {
        let mut f = lunch_file();
        f.pixels = Some([1080, 1080]);
        f.media_type = "image/webp".into();
        f.bytes = Some(60 * 1024 * 1024);
        let rep = check(&f, Some(&offer_campaign()), HC1);
        for code in [CODE_PIXELS, CODE_MEDIA_TYPE, CODE_FILE_SIZE] {
            assert_eq!(
                find(&rep, code, Severity::Error).len(),
                1,
                "want one {code} error in {:?}",
                rep.checks
            );
        }
        assert_eq!(
            find(&rep, CODE_PIXELS, Severity::Error)[0].message,
            "Canvas is 1080 \u{d7} 1080 px; the format needs 1080 \u{d7} 1920 px"
        );
    }

    #[test]
    fn max_words() {
        let mut f = lunch_file();
        f.elements[0].text = "Our best lunch specials are back for one week only".into();
        let rep = check(&f, Some(&offer_campaign()), HC1);
        let got = find(&rep, CODE_MAX_WORDS, Severity::Error);
        assert_eq!(
            first(&got).message,
            "Main message is 10 words; the limit is 8 (4 s dwell at 2 words/s)"
        );
    }

    #[test]
    fn required_element() {
        let mut f = lunch_file();
        f.elements.remove(1); // drop the legal line
        let offer = offer_campaign();

        let rep = check(&f, Some(&offer), HC1);
        assert_eq!(
            find(&rep, CODE_REQUIRED_ELEMENT, Severity::Error).len(),
            1,
            "offer without legal: {:?}",
            rep.checks
        );
        let no_tags = Campaign {
            category: "IAB8".into(),
            tags: Some(vec![]),
            ..Default::default()
        };
        let rep = check(&f, Some(&no_tags), HC1);
        assert!(
            rep.checks.is_empty(),
            "non-offer shouldn't need legal: {:?}",
            rep.checks
        );
        let unknown = Campaign {
            category: "IAB8".into(),
            ..Default::default()
        };
        let rep = check(&f, Some(&unknown), HC1);
        assert_eq!(
            find(&rep, CODE_REQUIRED_ELEMENT, Severity::NotChecked).len(),
            1
        );
        assert_eq!(rep.result, Outcome::Incomplete);

        // Legal must be 12 mm = 14 px on this screen.
        let mut f = lunch_file();
        f.elements[1].cap_height_px = Some(13.0);
        let rep = check(&f, Some(&offer), HC1);
        let got = find(&rep, CODE_REQUIRED_ELEMENT, Severity::Error);
        assert_eq!(
            first(&got).message,
            "Legal (on offer creative) is 13 px; it must be at least 14 px (12 mm)"
        );
    }

    #[test]
    fn category() {
        let restricted = Campaign {
            category: "IAB7-5".into(),
            tags: Some(vec!["offer".into()]),
            ..Default::default()
        };
        let rep = check(&lunch_file(), Some(&restricted), HC1);
        assert_eq!(find(&rep, CODE_CATEGORY, Severity::Error).len(), 1);
        let missing = Campaign {
            tags: Some(vec!["offer".into()]),
            ..Default::default()
        };
        let rep = check(&lunch_file(), Some(&missing), HC1);
        assert_eq!(find(&rep, CODE_CATEGORY, Severity::NotChecked).len(), 1);
    }

    fn motion_file(target: &str) -> CreativeFile {
        let mut f = lunch_file();
        f.file = "lunch.mp4".into();
        f.media_type = "video/mp4".into();
        f.targets = vec![target.into()];
        f.duration_s = Some(10.0);
        f
    }

    #[test]
    fn motion() {
        let c = offer_campaign();
        let mut f = motion_file(HC1);
        let rep = check(&f, Some(&c), HC1);
        assert_eq!(
            find(&rep, CODE_MOTION, Severity::NotChecked).len(),
            1,
            "flash rate without measurement should be not_checked: {:?}",
            rep.checks
        );
        f.max_flash_hz = Some(5.0);
        let rep = check(&f, Some(&c), HC1);
        assert_eq!(
            find(&rep, CODE_MOTION, Severity::Error).len(),
            1,
            "5 Hz over a 3 Hz limit"
        );
        f.max_flash_hz = Some(1.0);
        f.duration_s = Some(10.02); // within one frame at 30 fps
        let rep = check(&f, Some(&c), HC1);
        assert_eq!(
            find(&rep, CODE_MOTION, Severity::Error).len()
                + find(&rep, CODE_DURATION, Severity::Error).len(),
            0,
            "{:?}",
            rep.checks
        );
        f.duration_s = Some(15.0);
        let rep = check(&f, Some(&c), HC1);
        assert_eq!(
            find(&rep, CODE_DURATION, Severity::Error).len(),
            1,
            "15 s in a 10 s slot"
        );
        f.duration_s = None;
        let rep = check(&f, Some(&c), HC1);
        assert_eq!(find(&rep, CODE_DURATION, Severity::NotChecked).len(), 1);
    }

    #[test]
    fn motion_prohibited_by_placement() {
        let mut d = harbourline();
        d.networks[0].screens[0]
            .placement
            .as_mut()
            .unwrap()
            .motion_allowed = Some(false);
        let rep = check_doc(&d, &motion_file(HC1), Some(&offer_campaign()), HC1);
        let got = find(&rep, CODE_MOTION, Severity::Error);
        assert_eq!(
            first(&got).message,
            "This screen doesn't allow motion; supply a static file"
        );
    }

    #[test]
    fn stale_spec() {
        let input = Input {
            file: &lunch_file(),
            campaign: Some(&offer_campaign()),
            probe: None,
        };
        let now = DateTime::parse_from_rfc3339("2027-02-01T00:00:00Z").unwrap();
        let rep = validate(&harbourline(), &input, HC1, &Options::new(now)).unwrap();
        assert_eq!(find(&rep, CODE_SPEC_STALE, Severity::Error).len(), 1);
        assert_eq!(rep.result, Outcome::Fail);
    }

    // Core-level publishers (no physical size, viewing or rules) must not
    // make every report incomplete: rules that don't exist aren't "not checked".
    #[test]
    fn core_only_format_passes() {
        let d = Document {
            publisher: "p.example".into(),
            updated_at: "2026-10-20T09:00:00Z".into(),
            networks: vec![Network {
                id: "n".into(),
                formats: vec![Format {
                    id: "f".into(),
                    fields: FormatFields {
                        pixels: Some([1920, 1080]),
                        media: Some(Media {
                            static_types: Some(vec!["image/jpeg".into()]),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let f = CreativeFile {
            file: "a.jpg".into(),
            media_type: "image/jpeg".into(),
            pixels: Some([1920, 1080]),
            targets: vec!["p.example/n/f".into()],
            ..Default::default()
        };
        let rep = check_doc(&d, &f, None, "p.example/n/f");
        assert_eq!(rep.result, Outcome::Pass);
        assert!(rep.checks.is_empty(), "{:?}", rep.checks);
    }

    #[test]
    fn targets() {
        let d = harbourline();
        let f = lunch_file();
        let input = Input {
            file: &f,
            campaign: None,
            probe: None,
        };
        let opt = Options::new(test_now());
        let err = validate(&d, &input, "other.example/retail/portrait-totem-75", &opt).unwrap_err();
        assert!(
            matches!(err, ValidateError::OtherPublisher(_)),
            "another publisher's target should be skipped with OtherPublisher"
        );
        let rep = validate(&d, &input, "harbourline.example/retail/landscape-99", &opt).unwrap();
        assert_eq!(find(&rep, CODE_TARGET, Severity::Error).len(), 1);
    }

    #[test]
    fn outcome_precedence() {
        let c = |s| Check::new("X", s, "");
        for (checks, want) in [
            (vec![], Outcome::Pass),
            (vec![c(Severity::Info)], Outcome::Pass),
            (vec![c(Severity::Warning)], Outcome::Warn),
            (
                vec![c(Severity::Warning), c(Severity::NotChecked)],
                Outcome::Incomplete,
            ),
            (
                vec![c(Severity::NotChecked), c(Severity::Error)],
                Outcome::Fail,
            ),
        ] {
            assert_eq!(outcome(&checks), want, "{checks:?}");
        }
    }

    #[test]
    fn report_matches_schema() {
        let rep = check(&lunch_file(), Some(&offer_campaign()), TOTEM);
        let raw = json::to_string(&rep);
        let errs = validate_schema(Kind::Report, raw.as_bytes(), &Default::default()).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn example_manifest_end_to_end() {
        let raw = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/harbourline/creative.json"),
        )
        .unwrap();
        let m = crate::decode_manifest(&raw).unwrap();
        let d = harbourline();
        for f in &m.files {
            let input = Input {
                file: f,
                campaign: m.campaign.as_ref(),
                probe: None,
            };
            let rep = validate(&d, &input, &f.targets[0], &Options::new(test_now())).unwrap();
            assert_eq!(rep.result, Outcome::Fail, "{}: {:?}", f.file, rep.checks);
        }
    }

    // The probed file's facts override the manifest's, and disagreements are
    // reported against the file.
    #[test]
    fn file_overrides_manifest() {
        // A 1080 x 1080 PNG header; the prober reads only the header.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&1080u32.to_be_bytes());
        png.extend_from_slice(&1080u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        let p = crate::probe::probe_bytes(&png);
        let mut f = lunch_file();
        f.sha256 = "9f2c".into();
        f.bytes = None;
        let c = offer_campaign();
        let input = Input {
            file: &f,
            campaign: Some(&c),
            probe: Some(&p),
        };
        let rep = validate(&harbourline(), &input, HC1, &Options::new(test_now())).unwrap();
        let px = find(&rep, CODE_PIXELS, Severity::Error);
        assert_eq!(first(&px).source, "file", "{:?}", rep.checks);
        assert_eq!(find(&rep, CODE_MANIFEST_MISMATCH, Severity::Error).len(), 1);
        assert_eq!(
            find(&rep, CODE_MANIFEST_MISMATCH, Severity::Warning).len(),
            1
        );
        assert_eq!(
            rep.file_sha256, p.sha256,
            "report should cite the real file hash"
        );
    }

    // The non-overlap conflict is repeated in creative reports, and the
    // required element's position isn't also checked against it.
    #[test]
    fn non_overlapping_limits_reported_in_creative_report() {
        let mut d = harbourline();
        d.networks[0].rules.as_mut().unwrap().required_elements[0].within_rect_px =
            Some(Rect([0, 1600, 1080, 320]));
        d.networks[0].venues[0].rules = Some(RuleSet {
            required_elements: vec![RequiredElement {
                role: "legal".into(),
                when: "offer".into(),
                within_rect_px: Some(Rect([0, 0, 1080, 300])),
                ..Default::default()
            }],
            ..Default::default()
        });
        let rep = check_doc(&d, &lunch_file(), Some(&offer_campaign()), HC1);
        assert_eq!(
            find(&rep, CODE_RULE_CONFLICT, Severity::Error).len(),
            1,
            "{:?}",
            rep.checks
        );
        assert_eq!(find(&rep, CODE_REQUIRED_ELEMENT, Severity::Error).len(), 0);
    }

    #[test]
    fn checked_at_keeps_offset() {
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00+11:00").unwrap();
        let f = lunch_file();
        let input = Input {
            file: &f,
            campaign: None,
            probe: None,
        };
        let rep = validate(&harbourline(), &input, HC1, &Options::new(now)).unwrap();
        assert_eq!(rep.checked_at, "2026-10-22T12:00:00+11:00");
        let rep = check(&f, None, HC1);
        assert_eq!(rep.checked_at, "2026-10-22T12:00:00Z");
    }

    // 2026-10-22T10:30:00+11:00 is 2026-10-21T23:30:00Z, before the expiry.
    #[test]
    fn stale_across_offsets() {
        let mut d = harbourline();
        d.expires_at = "2026-10-22T00:00:00Z".into();
        let f = lunch_file();
        let input = Input {
            file: &f,
            campaign: None,
            probe: None,
        };
        let now = DateTime::parse_from_rfc3339("2026-10-22T10:30:00+11:00").unwrap();
        let rep = validate(&d, &input, HC1, &Options::new(now)).unwrap();
        assert!(find(&rep, CODE_SPEC_STALE, Severity::Error).is_empty());
        let later = DateTime::parse_from_rfc3339("2026-10-22T11:30:00+11:00").unwrap();
        let rep = validate(&d, &input, HC1, &Options::new(later)).unwrap();
        assert_eq!(find(&rep, CODE_SPEC_STALE, Severity::Error).len(), 1);
    }

    #[test]
    fn empty_checks_serialise_as_array() {
        let mut f = lunch_file();
        f.elements[1].cap_height_px = Some(20.0);
        let rep = check(&f, Some(&offer_campaign()), HC1);
        assert_eq!(rep.result, Outcome::Pass);
        assert!(json::to_string(&rep).contains("\"checks\":[]"));
    }
}
