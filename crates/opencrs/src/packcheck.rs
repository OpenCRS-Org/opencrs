//! Checks for the rule packs a screen references: creative rules become
//! findings, playback rules become info. Port of `crs/packcheck.go`.
//!
//! The `Checker` itself lives in `checks.rs`; this file adds the pack checks
//! to it.

use crate::check::{CODE_RULE_PACK, Check, Severity};
use crate::checks::{Checker, map_first_char, title};
use crate::legibility::{DEFAULT_READING_RATE_WPS, TextClass, class_of, count_words, num};
use crate::model::{CreativeFile, Placement};
use crate::resolve::Requirements;
use crate::rulepack::{PACK_DRAFT, PACK_REVIEWED, RulePack, RulePacks, versions_of};
use std::collections::BTreeSet;

pub const CODE_MOTION_PROHIBITED: &str = "CRS-MOTION-PROHIBITED";
pub const CODE_TRAFFIC_DEVICE: &str = "CRS-TRAFFIC-DEVICE";
pub const CODE_GLANCE_TEXT: &str = "CRS-GLANCE-TEXT";
pub const CODE_SEQUENCING: &str = "CRS-SEQUENCING";
pub const CODE_PLAYBACK: &str = "CRS-PLAYBACK";

/// The longest a driver should look away from the road to read a sign: one
/// glance. A working assumption, like the legibility ratio and reading rate,
/// until calibrated.
pub const DEFAULT_GLANCE_S: f64 = 2.0;

/// The ratio CRS-GLANCE-TEXT uses when the screen's rules set none: text
/// readable at 150 times its cap height.
pub const DEFAULT_LEGIBILITY_RATIO: f64 = 150.0;

/// What a driver can read of a sign at a given speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glance {
    /// Reading time: one glance, or less if the sign is passed sooner.
    pub seconds: f64,
    /// Words readable in that time.
    pub words: i64,
    /// Distance at which reading must start to finish by the near end.
    pub read_from_m: f64,
}

/// Works out the glance limits for a sign seen from `distance` = [near, far]
/// metres by a driver at `speed_kmh`, reading `rate` words per second.
/// Reading time is one glance, cut short if the driver crosses the viewing
/// range sooner: t = min(DEFAULT_GLANCE_S, (far - near) / v). The word limit
/// is t x rate, rounded down. To finish by the near end, the driver must
/// start reading at near + v x t, so text must be legible from there.
pub fn glance_at(speed_kmh: f64, distance: [f64; 2], rate: f64) -> Glance {
    let v = speed_kmh / 3.6;
    let mut t = DEFAULT_GLANCE_S;
    if v > 0.0 {
        t = t.min((distance[1] - distance[0]) / v);
    }
    t = t.max(0.0);
    Glance {
        seconds: t,
        words: (t * rate + 1e-9).floor() as i64,
        read_from_m: distance[0] + v * t,
    }
}

/// Go's `lowerFirst`, mapping the first character like `title`.
fn lower_first(s: &str) -> String {
    map_first_char(s, |c, out| out.extend(c.to_lowercase()))
}

fn round2(f: f64) -> f64 {
    (f * 100.0).round() / 100.0
}

/// Joins the refs of packs for which `rule` holds.
fn refs_where(packs: &[&RulePack], rule: impl Fn(&RulePack) -> bool) -> String {
    packs
        .iter()
        .filter(|p| rule(p))
        .map(|p| p.reference())
        .collect::<Vec<_>>()
        .join(", ")
}

fn prohibits_motion(p: &RulePack) -> bool {
    p.creative
        .motion
        .as_ref()
        .is_some_and(|m| m.allowed == Some(false))
}

/// One source's value for a playback limit.
struct Limit {
    /// +Inf for "no limit".
    value: f64,
    /// e.g. "25 s at 90 km/h".
    text: String,
    /// A pack ref, or "screen rules".
    source: String,
}

const SCREEN_RULES: &str = "screen rules";

const LUMINANCE_PERIODS: [&str; 3] = ["day", "twilight", "night"];

fn luminance_label(period: &str) -> String {
    match period {
        "day" => "Daytime luminance".into(),
        "twilight" => "Twilight luminance".into(),
        "night" => "Night-time luminance".into(),
        _ => format!("{} luminance", title(period)),
    }
}

fn note_text(note: &str) -> String {
    if note.is_empty() {
        String::new()
    } else {
        format!(", {note}")
    }
}

type ByZone = std::collections::BTreeMap<String, Option<f64>>;

/// Finds a luminance limit for `zone`, falling back to "default". The outer
/// `None` is "no entry"; an inner `None` is an entry with no limit.
fn lookup_zone(by_zone: &ByZone, zone: &str) -> Option<(Option<f64>, String)> {
    if !zone.is_empty()
        && let Some(v) = by_zone.get(zone)
    {
        return Some((*v, format!(" in {zone}")));
    }
    by_zone.get("default").map(|v| (*v, String::new()))
}

fn zone_unknown(reference: &str, zone: &str, by_zone: &ByZone) -> String {
    let vals: Vec<String> = by_zone
        .iter() // already sorted by zone
        .map(|(z, v)| match v {
            Some(v) => format!("{z} {}", num(*v)),
            None => format!("{z} no limit"),
        })
        .collect();
    let vals = vals.join(", ");
    if zone.is_empty() {
        format!(
            "{reference} sets it by luminance zone, and the screen has no placement.luminance_zone: {vals}"
        )
    } else {
        format!("{reference} has no zone {zone} (zones: {vals})")
    }
}

impl<'a> Checker<'a> {
    /// The known packs that apply to the screen, reporting unknown packs as
    /// not_checked and packs short of draft status as info.
    fn packs(&mut self) -> Vec<&'a RulePack> {
        let known: &'a RulePacks = self.rule_packs;
        let req: &'a Requirements<'a> = self.req;
        let mut out = Vec::new();
        for reference in &req.rules.rule_packs {
            let Some(p) = known.get(reference) else {
                let mut msg = format!(
                    "Rule pack {reference} isn't known to this validator, so its rules weren't checked"
                );
                let name = reference.split('@').next().unwrap_or(reference);
                let versions = versions_of(known, name);
                if !versions.is_empty() {
                    msg += &format!(" (known: {})", versions.join(", "));
                }
                self.report(CODE_RULE_PACK, Severity::NotChecked, msg);
                continue;
            };
            if p.status != PACK_DRAFT && p.status != PACK_REVIEWED {
                let mut msg = format!("Rule pack {reference} is {}", p.status);
                if !p.status_note.is_empty() {
                    msg += &format!("; {}", lower_first(&p.status_note));
                }
                self.report(CODE_RULE_PACK, Severity::Info, msg);
            }
            out.push(p);
        }
        out
    }

    /// Evaluates the screen's rule packs: creative rules as checks, playback
    /// rules (with the screen's own playback limits) as info.
    pub(crate) fn rule_packs_check(&mut self) {
        let packs = self.packs();
        self.motion_prohibited(&packs);
        self.traffic_device(&packs);
        self.glance_text(&packs);
        self.sequencing(&packs);
        self.playback(&packs);
    }

    /// The screen's known packs, without reporting the unknown ones.
    fn known_packs(&self) -> impl Iterator<Item = &'a RulePack> + use<'a> {
        let known: &'a RulePacks = self.rule_packs;
        let req: &'a Requirements<'a> = self.req;
        req.rules
            .rule_packs
            .iter()
            .filter_map(move |r| known.get(r))
    }

    /// Whether any of the screen's known packs disallows motion. It doesn't
    /// report unknown packs.
    pub(crate) fn pack_prohibits_motion(&self) -> bool {
        self.known_packs().any(prohibits_motion)
    }

    /// Whether any known pack forbids flashing.
    pub(crate) fn pack_no_flashing(&self) -> bool {
        self.known_packs().any(|p| {
            p.creative
                .motion
                .as_ref()
                .is_some_and(|m| m.no_flashing == Some(true))
        })
    }

    fn motion_prohibited(&mut self, packs: &[&RulePack]) {
        if !self.is_motion() {
            return;
        }
        let refs = refs_where(packs, prohibits_motion);
        if !refs.is_empty() {
            let mut c = Check::new(
                CODE_MOTION_PROHIBITED,
                Severity::Error,
                format!(
                    "Rule pack {refs} doesn't allow motion on this screen; supply a static file"
                ),
            );
            c.source = self.facts.media_src.clone();
            self.add(c);
        }
    }

    fn traffic_device(&mut self, packs: &[&RulePack]) {
        let refs = refs_where(packs, |p| p.creative.no_traffic_device_imagery);
        if !refs.is_empty() {
            self.report(
                CODE_TRAFFIC_DEVICE,
                Severity::NotChecked,
                format!(
                    "Nothing may look like a traffic sign or signal, or read as a driving instruction ({refs}); this needs visual review"
                ),
            );
        }
    }

    fn sequencing(&mut self, packs: &[&RulePack]) {
        let refs = refs_where(packs, |p| p.creative.no_sequencing);
        if !refs.is_empty() {
            self.report(
                CODE_SEQUENCING,
                Severity::Info,
                format!(
                    "Don't schedule this creative in a sequence that leads drivers to anticipate the next message, on this sign or across signs ({refs})"
                ),
            );
        }
    }

    fn glance_text(&mut self, packs: &[&RulePack]) {
        let refs = refs_where(packs, |p| p.creative.max_words_at_glance);
        if refs.is_empty() {
            return;
        }
        let req: &'a Requirements<'a> = self.req;
        let speed = req
            .screen
            .and_then(|s| s.placement.as_deref())
            .and_then(|p| p.speed_kmh);
        let dist = req
            .screen
            .and_then(|s| s.viewing.as_ref())
            .and_then(|v| v.distance_m);
        let mut missing = Vec::new();
        if speed.is_none() {
            missing.push("placement.speed_kmh");
        }
        if dist.is_none() {
            missing.push("viewing.distance_m");
        }
        let (Some(speed), Some(dist)) = (speed, dist) else {
            self.report(
                CODE_GLANCE_TEXT,
                Severity::NotChecked,
                format!(
                    "Glance limits need the screen's {} ({refs})",
                    missing.join(" and ")
                ),
            );
            return;
        };
        if self.file.elements.is_empty() {
            self.report(
                CODE_GLANCE_TEXT,
                Severity::NotChecked,
                format!(
                    "No elements in the manifest, so the glance limits can't be checked ({refs})"
                ),
            );
            return;
        }
        let rate = req
            .rules
            .reading_rate_wps
            .unwrap_or(DEFAULT_READING_RATE_WPS);
        let g = glance_at(speed, dist, rate);
        self.glance_words(&g, speed, &refs);
        self.glance_size(&g, speed, &refs);
    }

    /// Counts the words of every key-message element.
    fn glance_words(&mut self, g: &Glance, speed: f64, refs: &str) {
        let file: &'a CreativeFile = self.file;
        let mut parts: Vec<&str> = Vec::new();
        for e in &file.elements {
            if class_of(&e.role) != Some(TextClass::Key) {
                continue;
            }
            if e.text.is_empty() {
                let mut c = Check::new(
                    CODE_GLANCE_TEXT,
                    Severity::NotChecked,
                    format!(
                        "{} has no text in the manifest, so the glance word limit can't be checked ({refs})",
                        title(&e.role)
                    ),
                );
                c.element = e.role.clone();
                self.add(c);
                return;
            }
            parts.push(&e.text);
        }
        let n = count_words(&parts.join(" "));
        if n as i64 > g.words {
            self.report(
                CODE_GLANCE_TEXT,
                Severity::Error,
                format!(
                    "Text is {n} words; a driver at {} km/h can read {} in a {} s glance ({refs})",
                    num(speed),
                    g.words,
                    num(round2(g.seconds)),
                ),
            );
        }
    }

    /// Checks that key text is legible from where reading must start, at the
    /// screen's legibility ratio or the default.
    fn glance_size(&mut self, g: &Glance, speed: f64, refs: &str) {
        let req: &'a Requirements<'a> = self.req;
        let file: &'a CreativeFile = self.file;
        let ratio = req
            .rules
            .legibility
            .as_ref()
            .and_then(|l| l.ratio)
            .unwrap_or(DEFAULT_LEGIBILITY_RATIO);
        let min_px = req.mm_to_px(g.read_from_m * 1000.0 / ratio);
        let basis = format!(
            "to read from {} m, the start of a {} s glance at {} km/h (ratio {})",
            num(g.read_from_m.round()),
            num(round2(g.seconds)),
            num(speed),
            num(ratio),
        );
        for e in &file.elements {
            if class_of(&e.role) != Some(TextClass::Key) {
                continue;
            }
            let Some(min_px) = min_px else {
                self.report(
                    CODE_GLANCE_TEXT,
                    Severity::NotChecked,
                    format!(
                        "Glance text size can't be checked: no physical_mm to convert millimetres to pixels ({refs})"
                    ),
                );
                return;
            };
            let Some(got) = e.cap_height_px else {
                let mut c = Check::new(
                    CODE_GLANCE_TEXT,
                    Severity::NotChecked,
                    format!(
                        "{} has no cap_height_px in the manifest, so its glance size can't be checked ({refs})",
                        title(&e.role)
                    ),
                );
                c.element = e.role.clone();
                self.add(c);
                continue;
            };
            let msg = format!(
                "{} text is {} px; {min_px} px minimum {basis}",
                title(&e.role),
                num(got)
            );
            let min = min_px as f64;
            let (sev, msg) = if got < min {
                (Severity::Error, msg)
            } else if got < min * self.margin {
                (Severity::Warning, msg + ". Passes with little margin")
            } else {
                continue;
            };
            let mut c = Check::new(CODE_GLANCE_TEXT, sev, msg);
            c.element = e.role.clone();
            c.rect_px = e.rect_px;
            self.add(c);
        }
    }

    /// Combines limits from packs and the screen's rules into one info line.
    /// The strictest known value wins (the largest when `at_least`,
    /// otherwise the smallest); the others, and any that need context the
    /// screen doesn't give, are listed after it.
    fn playback_limit(
        &mut self,
        what: &str,
        at_least: bool,
        unit: &str,
        parts: &[Limit],
        unknown: &[String],
    ) {
        if parts.is_empty() && unknown.is_empty() {
            return;
        }
        let mut msg;
        if parts.is_empty() {
            msg = format!("{what} unknown");
        } else {
            let mut best = 0;
            for (i, p) in parts.iter().enumerate() {
                if (at_least && p.value > parts[best].value)
                    || (!at_least && p.value < parts[best].value)
                {
                    best = i;
                }
            }
            let b = &parts[best];
            msg = if b.value == f64::INFINITY {
                format!("{what} has no limit{} ({})", b.text, b.source)
            } else if at_least {
                format!(
                    "{what} at least {} {unit}{} ({})",
                    num(b.value),
                    b.text,
                    b.source
                )
            } else {
                format!(
                    "{what} at most {} {unit}{} ({})",
                    num(b.value),
                    b.text,
                    b.source
                )
            };
            for (i, p) in parts.iter().enumerate() {
                if i == best {
                    continue;
                }
                if p.value == f64::INFINITY {
                    msg += &format!("; {} sets no limit{}", p.source, p.text);
                } else {
                    msg += &format!("; {} sets {} {unit}{}", p.source, num(p.value), p.text);
                }
            }
        }
        for u in unknown {
            msg += "; ";
            msg += u;
        }
        self.report(CODE_PLAYBACK, Severity::Info, msg);
    }

    fn placement(&self) -> &'a Placement {
        static EMPTY: Placement = Placement {
            environment: String::new(),
            level: String::new(),
            zone: String::new(),
            mounted_height_mm: None,
            motion_allowed: None,
            speed_kmh: None,
            faces_road: None,
            school_zone: None,
            luminance_zone: String::new(),
        };
        let req: &'a Requirements<'a> = self.req;
        req.screen
            .and_then(|s| s.placement.as_deref())
            .unwrap_or(&EMPTY)
    }

    fn playback(&mut self, packs: &[&RulePack]) {
        self.dwell(packs);
        self.transition(packs);
        self.failure_state(packs);
        self.luminance(packs);
        self.school_zone(packs);
        let refs = refs_where(packs, |p| p.creative.no_device_interaction);
        if !refs.is_empty() {
            self.report(
                CODE_PLAYBACK,
                Severity::Info,
                format!("The sign mustn't interact with phones or in-vehicle devices ({refs})"),
            );
        }
    }

    fn dwell(&mut self, packs: &[&RulePack]) {
        let req: &'a Requirements<'a> = self.req;
        let mut parts = Vec::new();
        let mut unknown = Vec::new();
        if let Some(v) = req.rules.playback.min_dwell_s {
            parts.push(Limit {
                value: v,
                text: String::new(),
                source: SCREEN_RULES.into(),
            });
        }
        let speed = self.placement().speed_kmh;
        for p in packs {
            let bands = &p.playback.min_dwell_s;
            if bands.is_empty() {
                continue;
            }
            if !bands.by_speed() {
                parts.push(Limit {
                    value: bands.0[0].value,
                    text: note_text(&bands.0[0].note),
                    source: p.reference(),
                });
                continue;
            }
            let Some(speed) = speed else {
                let all: Vec<String> = bands
                    .0
                    .iter()
                    .map(|b| format!("{} s {}", num(b.value), b.describe()))
                    .collect();
                unknown.push(format!(
                    "{} sets dwell by speed, and the screen has no placement.speed_kmh: {}",
                    p.reference(),
                    all.join(", ")
                ));
                continue;
            };
            let Some(b) = bands.at(speed) else {
                unknown.push(format!(
                    "{} sets no dwell for {} km/h",
                    p.reference(),
                    num(speed)
                ));
                continue;
            };
            parts.push(Limit {
                value: b.value,
                text: format!(" at {} km/h{}", num(speed), note_text(&b.note)),
                source: p.reference(),
            });
        }
        self.playback_limit("Dwell", true, "s", &parts, &unknown);
    }

    fn transition(&mut self, packs: &[&RulePack]) {
        let req: &'a Requirements<'a> = self.req;
        let mut parts = Vec::new();
        if let Some(v) = req.rules.playback.max_transition_s {
            parts.push(Limit {
                value: v,
                text: String::new(),
                source: SCREEN_RULES.into(),
            });
        }
        for p in packs {
            if let Some(v) = p.playback.max_transition_s {
                parts.push(Limit {
                    value: v,
                    text: String::new(),
                    source: p.reference(),
                });
            }
        }
        self.playback_limit("Transition", false, "s", &parts, &[]);
    }

    fn failure_state(&mut self, packs: &[&RulePack]) {
        let mut states = Vec::new();
        for p in packs {
            let s = p.playback.failure_state.as_str();
            if s.is_empty() {
                continue;
            }
            let s = match s {
                "black" => "a black screen",
                "blank" => "a blank screen",
                other => other,
            };
            states.push(format!("{s} ({})", p.reference()));
        }
        if !states.is_empty() {
            self.report(
                CODE_PLAYBACK,
                Severity::Info,
                format!("If the image fails, show {}", states.join(" and ")),
            );
        }
    }

    fn luminance(&mut self, packs: &[&RulePack]) {
        let req: &'a Requirements<'a> = self.req;
        // Periods beyond the usual three, in order.
        let extra: BTreeSet<&str> = packs
            .iter()
            .flat_map(|p| p.playback.max_luminance_cd_m2.keys())
            .map(String::as_str)
            .filter(|p| !LUMINANCE_PERIODS.contains(p))
            .collect();
        let zone = self.placement().luminance_zone.as_str();
        let b = &req.rules.brightness;
        for period in LUMINANCE_PERIODS.iter().copied().chain(extra) {
            let mut parts = Vec::new();
            let mut unknown = Vec::new();
            let screen_limit = match period {
                "day" => b.max_day_cd_m2,
                "night" => b.max_night_cd_m2,
                _ => None,
            };
            if let Some(v) = screen_limit {
                parts.push(Limit {
                    value: v,
                    text: String::new(),
                    source: SCREEN_RULES.into(),
                });
            }
            for p in packs {
                let Some(by_zone) = p.playback.max_luminance_cd_m2.get(period) else {
                    continue;
                };
                let Some((v, where_)) = lookup_zone(by_zone, zone) else {
                    unknown.push(zone_unknown(&p.reference(), zone, by_zone));
                    continue;
                };
                parts.push(Limit {
                    value: v.unwrap_or(f64::INFINITY),
                    text: where_,
                    source: p.reference(),
                });
            }
            self.playback_limit(
                &luminance_label(period),
                false,
                "cd/m\u{b2}",
                &parts,
                &unknown,
            );
        }
    }

    fn school_zone(&mut self, packs: &[&RulePack]) {
        let sz = self.placement().school_zone;
        if sz == Some(false) {
            return;
        }
        for p in packs {
            let rule = p.playback.school_zone.as_str();
            if rule.is_empty() {
                continue;
            }
            let what = if rule == "fixed_display_during_hours" {
                "switch to a fixed display during school-zone hours".to_string()
            } else {
                format!("follow the school-zone rule {rule}")
            };
            if sz.is_none() {
                self.report(
                    CODE_PLAYBACK,
                    Severity::Info,
                    format!(
                        "If the sign is visible from a school zone, {what}; the screen has no placement.school_zone ({})",
                        p.reference()
                    ),
                );
                continue;
            }
            self.report(
                CODE_PLAYBACK,
                Severity::Info,
                format!("{} ({})", title(&what), p.reference()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::Facts;
    use crate::load::decode_document;
    use crate::model::{Document, Element, MotionRules, Rect};
    use crate::resolve::parse_ref;
    use crate::rulepack::{PACK_UNVERIFIED, PackCreative, PackPlayback, builtin_rule_packs};

    fn roadside() -> Document {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/roadside.screens.json");
        decode_document(&std::fs::read(p).unwrap()).unwrap()
    }

    /// A static 12 m x 3 m billboard creative: a 2-word headline and a 2-word
    /// CTA, both at 70 px.
    fn billboard_file(target: &str) -> CreativeFile {
        let el = |role: &str, text: &str, rect: [i64; 4]| Element {
            role: role.into(),
            text: text.into(),
            rect_px: Some(Rect(rect)),
            cap_height_px: Some(70.0),
        };
        CreativeFile {
            file: "drive.png".into(),
            media_type: "image/png".into(),
            pixels: Some([1248, 336]),
            targets: vec![target.into()],
            elements: vec![
                el("headline", "Lunch ready", [100, 60, 700, 80]),
                el("cta", "Exit 4", [100, 200, 300, 60]),
            ],
            ..Default::default()
        }
    }

    fn motion_file(target: &str) -> CreativeFile {
        let mut f = billboard_file(target);
        f.file = "drive.mp4".into();
        f.media_type = "video/mp4".into();
        f.duration_s = Some(10.0);
        f
    }

    fn screen_target(id: &str) -> String {
        let network = if id.starts_with("HP-") {
            "roadside-other"
        } else {
            "roadside-nsw"
        };
        format!("harbourline.example/{network}/screen/{id}")
    }

    /// Runs the pack checks the way `validate` will: resolve the screen,
    /// build the checker from the manifest alone, and collect its findings.
    fn run_with(d: &Document, f: &CreativeFile, screen: &str, packs: &RulePacks) -> Vec<Check> {
        let reqs = d
            .resolve(&parse_ref(&screen_target(screen)).unwrap())
            .unwrap();
        let facts = Facts {
            media_type: f.media_type.clone(),
            media_src: "manifest".into(),
            ..Default::default()
        };
        let mut c = Checker {
            req: &reqs[0],
            file: f,
            campaign: None,
            facts,
            margin: 1.25,
            rule_packs: packs,
            out: Vec::new(),
        };
        c.rule_packs_check();
        c.out
    }

    fn run(f: &CreativeFile, screen: &str) -> Vec<Check> {
        run_with(&roadside(), f, screen, builtin_rule_packs())
    }

    fn find(cs: &[Check], code: &str, sev: Severity) -> Vec<String> {
        cs.iter()
            .filter(|c| c.code == code && c.severity == sev)
            .map(|c| c.message.clone())
            .collect()
    }

    fn with_packs(d: &mut Document, network: usize, screen: usize, packs: &[&str]) {
        let s = &mut d.networks[network].screens[screen];
        s.rules.get_or_insert_default().rule_packs = packs.iter().map(|s| s.to_string()).collect();
    }

    // The worked examples in the spec's CRS-GLANCE-TEXT section.
    #[test]
    fn glance_at_worked_examples() {
        let cases: [(f64, [f64; 2], f64, i64, f64); 8] = [
            (80.0, [20.0, 150.0], 2.0, 4, 64.44),      // a full glance
            (100.0, [10.0, 50.0], 1.44, 2, 50.0),      // passed in under 2 s
            (90.0, [30.0, 200.0], 2.0, 4, 80.0),       // HR-M4-001
            (0.0, [5.0, 10.0], 2.0, 4, 5.0),           // stationary
            (110.0, [20.0, 20.0], 0.0, 0, 20.0),       // no time to read
            (60.0, [15.0, 120.0], 2.0, 4, 48.33),      // HR-PAR-002
            (50.0, [10.0, 20.0], 0.72, 1, 20.0),       // 10 m at 13.9 m/s
            (100.0, [30.0, 300.0], 2.0, 4, 85.555556), // long range
        ];
        for (speed, dist, seconds, words, read_from) in cases {
            let g = glance_at(speed, dist, DEFAULT_READING_RATE_WPS);
            assert!(
                (g.seconds - seconds).abs() <= 0.01
                    && g.words == words
                    && (g.read_from_m - read_from).abs() <= 0.01,
                "glance_at({speed}, {dist:?}) = {g:?}; want {seconds} s, {words} words, from {read_from} m"
            );
        }
    }

    #[test]
    fn glance_text() {
        type Edit = fn(&mut CreativeFile);
        type Case = (
            &'static str,
            &'static str,
            Option<Edit>,
            Option<Severity>,
            Vec<&'static str>,
        );
        let cases: [Case; 7] = [
            ("passes", "HR-M4-001", None, None, vec![]),
            (
                "too many words",
                "HR-M4-001",
                Some(|f| f.elements[0].text = "Our lunch specials are ready now".into()),
                Some(Severity::Error),
                vec![
                    "Text is 8 words; a driver at 90 km/h can read 4 in a 2 s glance (au-nsw-transport-corridor@2017-11)",
                ],
            ),
            (
                "too small",
                "HR-M4-001",
                Some(|f| f.elements[0].cap_height_px = Some(50.0)),
                Some(Severity::Error),
                vec![
                    "Headline text is 50 px; 54 px minimum to read from 80 m, the start of a 2 s glance at 90 km/h (ratio 150)",
                ],
            ),
            (
                "little margin",
                "HR-M4-001",
                Some(|f| f.elements[0].cap_height_px = Some(55.0)),
                Some(Severity::Warning),
                vec![
                    "Headline text is 55 px; 54 px minimum to read from 80 m, the start of a 2 s glance at 90 km/h (ratio 150). Passes with little margin",
                ],
            ),
            (
                "no speed or distance",
                "HR-CBD-003",
                None,
                Some(Severity::NotChecked),
                vec![
                    "Glance limits need the screen's placement.speed_kmh and viewing.distance_m (au-nsw-transport-corridor@2017-11)",
                ],
            ),
            (
                "no text",
                "HR-M4-001",
                Some(|f| f.elements[1].text = String::new()),
                Some(Severity::NotChecked),
                vec![
                    "Cta has no text in the manifest, so the glance word limit can't be checked (au-nsw-transport-corridor@2017-11)",
                ],
            ),
            (
                "no elements",
                "HR-M4-001",
                Some(|f| f.elements.clear()),
                Some(Severity::NotChecked),
                vec![
                    "No elements in the manifest, so the glance limits can't be checked (au-nsw-transport-corridor@2017-11)",
                ],
            ),
        ];
        for (name, screen, edit, sev, want) in cases {
            let mut f = billboard_file(&screen_target(screen));
            if let Some(edit) = edit {
                edit(&mut f);
            }
            let cs = run(&f, screen);
            let glance: Vec<&Check> = cs.iter().filter(|c| c.code == CODE_GLANCE_TEXT).collect();
            let got: Vec<&str> = glance.iter().map(|c| c.message.as_str()).collect();
            assert_eq!(got, want, "{name}");
            for c in glance {
                assert_eq!(Some(c.severity), sev, "{name}");
            }
        }
    }

    #[test]
    fn glance_text_findings_carry_element_and_rect() {
        let mut f = billboard_file(&screen_target("HR-M4-001"));
        f.elements[0].cap_height_px = Some(50.0);
        let cs = run(&f, "HR-M4-001");
        let c = cs.iter().find(|c| c.code == CODE_GLANCE_TEXT).unwrap();
        assert_eq!(c.element, "headline");
        assert_eq!(c.rect_px, Some(Rect([100, 60, 700, 80])));
    }

    #[test]
    fn pack_creative_codes() {
        let target = screen_target("HR-M4-001");
        let cs = run(&billboard_file(&target), "HR-M4-001");
        assert_eq!(
            find(&cs, CODE_TRAFFIC_DEVICE, Severity::NotChecked),
            [
                "Nothing may look like a traffic sign or signal, or read as a driving instruction (au-nsw-transport-corridor@2017-11); this needs visual review"
            ]
        );
        assert_eq!(find(&cs, CODE_SEQUENCING, Severity::Info).len(), 1);
        assert!(
            find(&cs, CODE_MOTION_PROHIBITED, Severity::Error).is_empty(),
            "a static file isn't motion"
        );
        assert!(
            find(&cs, CODE_RULE_PACK, Severity::Info).is_empty(),
            "a draft pack needs no status note"
        );

        let cs = run(&motion_file(&target), "HR-M4-001");
        assert_eq!(
            find(&cs, CODE_MOTION_PROHIBITED, Severity::Error),
            [
                "Rule pack au-nsw-transport-corridor@2017-11 doesn't allow motion on this screen; supply a static file"
            ]
        );
        let c = cs
            .iter()
            .find(|c| c.code == CODE_MOTION_PROHIBITED)
            .unwrap();
        assert_eq!(c.source, "manifest");
    }

    // A pack with no creative rules (the Victorian luminance pack) raises no
    // creative checks, even for motion.
    #[test]
    fn pack_without_creative_rules() {
        let target = screen_target("HP-VIC-003");
        let cs = run(&motion_file(&target), "HP-VIC-003");
        for code in [
            CODE_MOTION_PROHIBITED,
            CODE_TRAFFIC_DEVICE,
            CODE_GLANCE_TEXT,
            CODE_SEQUENCING,
        ] {
            assert!(
                !cs.iter().any(|c| c.code == code),
                "{code} doesn't apply under the Victorian pack: {cs:?}"
            );
        }
    }

    #[test]
    fn playback_info() {
        let cases: [(&str, Vec<&str>); 7] = [
            (
                "HR-M4-001",
                vec![
                    "Dwell at least 25 s at 90 km/h (au-nsw-transport-corridor@2017-11)",
                    "Transition at most 0.1 s (au-nsw-transport-corridor@2017-11)",
                    "If the image fails, show a black screen (au-nsw-transport-corridor@2017-11)",
                    "Daytime luminance at most 6000 cd/m\u{b2} in zone_4 (au-nsw-transport-corridor@2017-11)",
                    "Twilight luminance at most 500 cd/m\u{b2} in zone_4 (au-nsw-transport-corridor@2017-11)",
                    "Night-time luminance at most 200 cd/m\u{b2} in zone_4 (au-nsw-transport-corridor@2017-11)",
                    "The sign mustn't interact with phones or in-vehicle devices (au-nsw-transport-corridor@2017-11)",
                ],
            ),
            // The site consent is stricter on dwell and night luminance, and
            // can't loosen the transition.
            (
                "HR-PAR-002",
                vec![
                    "Dwell at least 40 s (screen rules); au-nsw-transport-corridor@2017-11 sets 10 s at 60 km/h",
                    "Transition at most 0.1 s (au-nsw-transport-corridor@2017-11); screen rules sets 0.5 s",
                    "If the image fails, show a black screen (au-nsw-transport-corridor@2017-11)",
                    "Daytime luminance at most 6000 cd/m\u{b2} in zone_2 (au-nsw-transport-corridor@2017-11)",
                    "Twilight luminance at most 700 cd/m\u{b2} in zone_2 (au-nsw-transport-corridor@2017-11)",
                    "Night-time luminance at most 150 cd/m\u{b2} (screen rules); au-nsw-transport-corridor@2017-11 sets 350 cd/m\u{b2} in zone_2",
                    "Switch to a fixed display during school-zone hours (au-nsw-transport-corridor@2017-11)",
                    "The sign mustn't interact with phones or in-vehicle devices (au-nsw-transport-corridor@2017-11)",
                ],
            ),
            // Missing context is stated, not guessed.
            (
                "HR-CBD-003",
                vec![
                    "Dwell unknown; au-nsw-transport-corridor@2017-11 sets dwell by speed, and the screen has no placement.speed_kmh: 10 s below 80 km/h, 25 s at 80 km/h and over",
                    "Transition at most 0.1 s (au-nsw-transport-corridor@2017-11)",
                    "If the image fails, show a black screen (au-nsw-transport-corridor@2017-11)",
                    "Daytime luminance has no limit in zone_1 (au-nsw-transport-corridor@2017-11)",
                    "Twilight luminance at most 700 cd/m\u{b2} in zone_1 (au-nsw-transport-corridor@2017-11)",
                    "Night-time luminance at most 350 cd/m\u{b2} in zone_1 (au-nsw-transport-corridor@2017-11)",
                    "If the sign is visible from a school zone, switch to a fixed display during school-zone hours; the screen has no placement.school_zone (au-nsw-transport-corridor@2017-11)",
                    "The sign mustn't interact with phones or in-vehicle devices (au-nsw-transport-corridor@2017-11)",
                ],
            ),
            (
                "HP-WA-001",
                vec![
                    "Dwell at least 10 s at 60 km/h, the floor of Table 6; up to 35 s by crash risk and offset (au-wa-mrwa-digital-signs@2025-05)",
                    "Transition at most 0.1 s (au-wa-mrwa-digital-signs@2025-05)",
                    "Daytime luminance at most 6000 cd/m\u{b2} (au-wa-mrwa-digital-signs@2025-05)",
                    "Twilight luminance at most 600 cd/m\u{b2} (au-wa-mrwa-digital-signs@2025-05)",
                    "Night-time luminance at most 250 cd/m\u{b2} in A3 (au-wa-mrwa-digital-signs@2025-05)",
                ],
            ),
            (
                "HP-QLD-002",
                vec![
                    "Dwell at least 15 s at 100 km/h, 25 s if visible from a motorway or motorway-standard road (au-qld-tmr-roadside-advertising@2022-12)",
                    "Transition at most 0.5 s (au-qld-tmr-roadside-advertising@2022-12)",
                    "If the image fails, show a blank screen (au-qld-tmr-roadside-advertising@2022-12)",
                ],
            ),
            (
                "HP-VIC-003",
                vec![
                    "Dwell at least 45 s (screen rules)",
                    "Daytime luminance at most 6000 cd/m\u{b2} in cat_a_medium (au-vic-dtp-illuminated-signage@2023-10)",
                    "Twilight luminance at most 700 cd/m\u{b2} in cat_a_medium (au-vic-dtp-illuminated-signage@2023-10)",
                    "Night-time luminance at most 250 cd/m\u{b2} in cat_a_medium (au-vic-dtp-illuminated-signage@2023-10)",
                ],
            ),
            (
                "HP-SA-004",
                vec![
                    "Dwell at least 45 s, longer at complex or high-speed sites (au-sa-dit-advertising-signs@2014-08)",
                    "Transition at most 0.1 s (au-sa-dit-advertising-signs@2014-08)",
                    "If the image fails, show a black screen (au-sa-dit-advertising-signs@2014-08)",
                    "Night-time luminance at most 200 cd/m\u{b2} (au-sa-dit-advertising-signs@2014-08)",
                ],
            ),
        ];
        for (screen, want) in cases {
            let cs = run(&billboard_file(&screen_target(screen)), screen);
            assert_eq!(find(&cs, CODE_PLAYBACK, Severity::Info), want, "{screen}");
        }
        let cs = run(&billboard_file(&screen_target("HP-TAS-005")), "HP-TAS-005");
        assert!(find(&cs, CODE_PLAYBACK, Severity::Info).is_empty());
    }

    #[test]
    fn unverified_pack_says_so() {
        let mut packs = RulePacks::new();
        packs.insert(
            "x-summary@2024-02".into(),
            RulePack {
                name: "x-summary".into(),
                version: "2024-02".into(),
                status: PACK_UNVERIFIED.into(),
                status_note: "Matches a summary, but not checked against the primary source".into(),
                playback: PackPlayback {
                    min_dwell_s: crate::rulepack::DwellBands(vec![crate::rulepack::SpeedBand::of(
                        45.0,
                    )]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let mut d = roadside();
        with_packs(&mut d, 1, 3, &["x-summary@2024-02"]);
        let f = billboard_file(&screen_target("HP-SA-004"));
        let cs = run_with(&d, &f, "HP-SA-004", &packs);
        assert_eq!(
            find(&cs, CODE_RULE_PACK, Severity::Info),
            [
                "Rule pack x-summary@2024-02 is unverified; matches a summary, but not checked against the primary source"
            ]
        );
        assert_eq!(find(&cs, CODE_PLAYBACK, Severity::Info).len(), 1);
    }

    // A pack without a status note says just the status.
    #[test]
    fn pack_status_without_note() {
        let raw =
            br#"{"rule_pack":"x-test","version":"2026-01","source_url":"https://a.example/x.pdf"}"#;
        let packs = crate::rulepack::load_rule_packs(&[("x-test@2026-01.json", raw)]).unwrap();
        let mut d = roadside();
        with_packs(&mut d, 1, 3, &["x-test@2026-01"]);
        let f = billboard_file(&screen_target("HP-SA-004"));
        let cs = run_with(&d, &f, "HP-SA-004", &packs);
        assert_eq!(
            find(&cs, CODE_RULE_PACK, Severity::Info),
            ["Rule pack x-test@2026-01 is unverified"]
        );
    }

    // Every published pack's creative rules, for a motion file and a static
    // file on its screen. Motion is prohibited everywhere but Victoria;
    // traffic-device imagery always needs review where a pack forbids it.
    #[test]
    fn state_pack_creative_rules() {
        // screen, motion, traffic, sequencing, glance (not_checked: no
        // viewing distance), status (the pack is short of draft)
        let cases = [
            ("HP-WA-001", true, true, true, false, false),
            ("HP-QLD-002", true, true, true, false, false),
            ("HP-VIC-003", false, false, false, false, false),
            ("HP-SA-004", true, true, true, true, false),
            ("HP-TAS-005", true, false, false, false, true),
        ];
        for (screen, motion, traffic, sequencing, glance, status) in cases {
            let target = screen_target(screen);
            let cs = run(&motion_file(&target), screen);
            assert_eq!(
                find(&cs, CODE_MOTION_PROHIBITED, Severity::Error).len() == 1,
                motion,
                "{screen} motion prohibited"
            );
            let cs = run(&billboard_file(&target), screen);
            let one = |code, sev| find(&cs, code, sev).len() == 1;
            assert_eq!(
                one(CODE_TRAFFIC_DEVICE, Severity::NotChecked),
                traffic,
                "{screen} traffic"
            );
            assert_eq!(
                one(CODE_SEQUENCING, Severity::Info),
                sequencing,
                "{screen} sequencing"
            );
            assert_eq!(
                one(CODE_GLANCE_TEXT, Severity::NotChecked),
                glance,
                "{screen} glance"
            );
            assert_eq!(
                one(CODE_RULE_PACK, Severity::Info),
                status,
                "{screen} status"
            );
        }
    }

    // Unknown packs (or unknown versions) are not_checked; known packs on the
    // same screen are still evaluated.
    #[test]
    fn unknown_packs() {
        let f = billboard_file(&screen_target("HR-UNK-004"));
        let cs = run(&f, "HR-UNK-004");
        assert_eq!(
            find(&cs, CODE_RULE_PACK, Severity::NotChecked),
            [
                "Rule pack au-nt-roadside@2020-01 isn't known to this validator, so its rules weren't checked",
                "Rule pack au-nsw-transport-corridor@2009-01 isn't known to this validator, so its rules weren't checked (known: au-nsw-transport-corridor@2017-11)",
            ]
        );
        let playback = find(&cs, CODE_PLAYBACK, Severity::Info);
        assert_eq!(
            playback.first().map(String::as_str),
            Some("Dwell at least 10 s at 70 km/h (au-nsw-transport-corridor@2017-11)"),
            "the known pack should still apply"
        );
        // A validator given no packs at all reports every reference.
        let f = billboard_file(&screen_target("HR-M4-001"));
        let cs = run_with(&roadside(), &f, "HR-M4-001", &RulePacks::new());
        assert_eq!(find(&cs, CODE_RULE_PACK, Severity::NotChecked).len(), 1);
        assert!(find(&cs, CODE_PLAYBACK, Severity::Info).is_empty());
    }

    // Packs accumulate down the chain, and a screen that re-allows motion
    // can't undo a pack's prohibition.
    #[test]
    fn packs_accumulate_and_motion_stays_prohibited() {
        let d = roadside();
        let reqs = d
            .resolve(&parse_ref(&screen_target("HR-UNK-004")).unwrap())
            .unwrap();
        assert_eq!(
            reqs[0].rules.rule_packs,
            [
                "au-nsw-transport-corridor@2017-11",
                "au-nt-roadside@2020-01",
                "au-nsw-transport-corridor@2009-01"
            ]
        );
        // HR-PAR-002 sets motion_allowed and rules.motion.allowed to true.
        let f = motion_file(&screen_target("HR-PAR-002"));
        let cs = run(&f, "HR-PAR-002");
        assert_eq!(find(&cs, CODE_MOTION_PROHIBITED, Severity::Error).len(), 1);
    }

    #[test]
    fn known_pack_lookups_ignore_unknown_refs() {
        let f = billboard_file(&screen_target("HR-UNK-004"));
        let d = roadside();
        let reqs = d
            .resolve(&parse_ref(&screen_target("HR-UNK-004")).unwrap())
            .unwrap();
        let mut packs = builtin_rule_packs().clone();
        let c = Checker {
            req: &reqs[0],
            file: &f,
            campaign: None,
            facts: Facts::default(),
            margin: 1.25,
            rule_packs: &packs,
            out: Vec::new(),
        };
        assert!(c.pack_prohibits_motion());
        assert!(c.pack_no_flashing(), "the NSW pack forbids flashing");
        // A pack's no_flashing makes the flash limit 0 Hz for motion it allows.
        let flash = RulePack {
            name: "x-flash".into(),
            version: "1".into(),
            status: PACK_DRAFT.into(),
            creative: PackCreative {
                motion: Some(MotionRules {
                    no_flashing: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        packs.insert("x-flash@1".into(), flash);
        let mut d = roadside();
        let target = screen_target("HP-VIC-003");
        let reqs = d.resolve(&parse_ref(&target).unwrap()).unwrap();
        let vic = Checker {
            req: &reqs[0],
            file: &f,
            campaign: None,
            facts: Facts::default(),
            margin: 1.25,
            rule_packs: &packs,
            out: Vec::new(),
        };
        assert!(
            !vic.pack_no_flashing(),
            "the Victorian pack says nothing about flashing"
        );
        with_packs(
            &mut d,
            1,
            2,
            &["au-vic-dtp-illuminated-signage@2023-10", "x-flash@1"],
        );
        let reqs = d.resolve(&parse_ref(&target).unwrap()).unwrap();
        let c = Checker {
            req: &reqs[0],
            file: &f,
            campaign: None,
            facts: Facts::default(),
            margin: 1.25,
            rule_packs: &packs,
            out: Vec::new(),
        };
        assert!(c.pack_no_flashing());
        assert!(!c.pack_prohibits_motion());
    }

    #[test]
    fn title_and_lower_first_match_go() {
        assert_eq!(title("fine_print"), "Fine print");
        assert_eq!(title(""), "");
        assert_eq!(lower_first("Matches a summary"), "matches a summary");
        assert_eq!(lower_first(""), "");
        // Unlike Go (byte-wise), a multi-byte first character maps cleanly.
        assert_eq!(lower_first("\u{c9}t\u{e9}"), "\u{e9}t\u{e9}");
        assert_eq!(title("\u{e9}cran_principal"), "\u{c9}cran principal");
    }

    fn check_roadside(f: &CreativeFile, target: &str) -> crate::report::Report {
        crate::testutil::check_doc(&roadside(), f, None, target)
    }

    // The pack checks run inside `validate`, and the result follows the
    // findings.
    #[test]
    fn pack_creative_codes_report() {
        use crate::report::Outcome;
        let target = screen_target("HR-M4-001");
        let rep = check_roadside(&billboard_file(&target), &target);
        assert_eq!(
            rep.result,
            Outcome::Incomplete,
            "a clean static creative is incomplete (traffic devices need review): {:?}",
            rep.checks
        );
        let find_in = |rep: &crate::report::Report, code: &str, sev: Severity| {
            crate::testutil::find(rep, code, sev)
                .iter()
                .map(|c| c.message.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            find_in(&rep, CODE_TRAFFIC_DEVICE, Severity::NotChecked),
            [
                "Nothing may look like a traffic sign or signal, or read as a driving instruction (au-nsw-transport-corridor@2017-11); this needs visual review"
            ]
        );
        assert_eq!(
            find_in(&rep, CODE_SEQUENCING, Severity::Info).len(),
            1,
            "{:?}",
            rep.checks
        );
        assert!(find_in(&rep, CODE_MOTION_PROHIBITED, Severity::Error).is_empty());
        assert!(
            find_in(&rep, CODE_RULE_PACK, Severity::Info).is_empty(),
            "a draft pack needs no status note"
        );

        // Motion: CRS-MOTION-PROHIBITED, and not also CRS-MOTION.
        let rep = check_roadside(&motion_file(&target), &target);
        assert_eq!(
            find_in(&rep, CODE_MOTION_PROHIBITED, Severity::Error),
            [
                "Rule pack au-nsw-transport-corridor@2017-11 doesn't allow motion on this screen; supply a static file"
            ],
            "{:?}",
            rep.checks
        );
        assert!(
            rep.checks
                .iter()
                .all(|c| c.code != crate::check::CODE_MOTION),
            "motion is reported once, as CRS-MOTION-PROHIBITED: {:?}",
            rep.checks
        );
    }

    // A pack's no_flashing makes the flash limit 0 Hz for motion it allows.
    #[test]
    fn pack_no_flashing_limits_flash_rate() {
        let mut packs = RulePacks::new();
        packs.insert(
            "x-flash@1".into(),
            RulePack {
                name: "x-flash".into(),
                version: "1".into(),
                status: PACK_DRAFT.into(),
                creative: PackCreative {
                    motion: Some(MotionRules {
                        no_flashing: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let mut d = roadside();
        with_packs(&mut d, 1, 0, &["x-flash@1"]);
        let target = screen_target("HP-WA-001");
        let mut f = motion_file(&target);
        f.max_flash_hz = Some(1.0);
        let input = crate::report::Input {
            file: &f,
            campaign: None,
            probe: None,
        };
        let opt = crate::report::Options {
            rule_packs: &packs,
            ..crate::report::Options::new(crate::testutil::test_now())
        };
        let rep = crate::report::validate(&d, &input, &target, &opt).unwrap();
        assert_eq!(
            find(&rep.checks, crate::check::CODE_MOTION, Severity::Error),
            ["Flashes at 1 Hz; the limit is 0 Hz"],
            "{:?}",
            rep.checks
        );
    }

    // An unverified pack never changes the result.
    #[test]
    fn info_never_changes_the_result() {
        let mut packs = RulePacks::new();
        packs.insert(
            "x-summary@2024-02".into(),
            RulePack {
                name: "x-summary".into(),
                version: "2024-02".into(),
                status: PACK_UNVERIFIED.into(),
                status_note: "Matches a summary, but not checked against the primary source".into(),
                playback: PackPlayback {
                    min_dwell_s: crate::rulepack::DwellBands(vec![crate::rulepack::SpeedBand::of(
                        45.0,
                    )]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let mut d = roadside();
        with_packs(&mut d, 1, 3, &["x-summary@2024-02"]);
        let target = screen_target("HP-SA-004");
        let f = billboard_file(&target);
        let input = crate::report::Input {
            file: &f,
            campaign: None,
            probe: None,
        };
        let opt = crate::report::Options {
            rule_packs: &packs,
            ..crate::report::Options::new(crate::testutil::test_now())
        };
        let rep = crate::report::validate(&d, &input, &target, &opt).unwrap();
        assert_eq!(
            find(&rep.checks, CODE_RULE_PACK, Severity::Info),
            [
                "Rule pack x-summary@2024-02 is unverified; matches a summary, but not checked against the primary source"
            ]
        );
        assert_eq!(
            rep.result,
            crate::report::Outcome::Pass,
            "info never changes the result: {:?}",
            rep.checks
        );
    }

    // A screen can't loosen what a pack sets.
    #[test]
    fn pack_motion_prohibition_fails_the_report() {
        let target = screen_target("HR-PAR-002");
        let rep = check_roadside(&motion_file(&target), &target);
        assert_eq!(rep.result, crate::report::Outcome::Fail, "{:?}", rep.checks);
    }
}
