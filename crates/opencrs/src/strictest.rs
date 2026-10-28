//! Strictest requirements: one set of requirements for everything a
//! reference covers, such that a file that meets it passes on every screen.
//!
//! Resolving a format gives a result for each of its screens. A team making
//! one file for the whole format wants one. Each screen's requirements are
//! combined here, a limit at a time: the larger safe-area inset on each
//! side, the smaller maximum, the larger minimum, the intersection of what
//! may be delivered, the union of what must be avoided or included. Minimum
//! text sizes are worked out in pixels at each screen's own viewing
//! distance and then compared. The answer names the screen that set each
//! value (`sources`), and says in `notes` where no single file can pass
//! everywhere.
//!
//! Like [`crate::stream::Resolved`] this reads the screens of the document's
//! text one at a time. Screens of the same kind (see
//! [`Requirements::key`]) add nothing to each other, so only the first of
//! each kind is resolved and combined; the rest are counted. What is kept is
//! the combined limits, the set of kinds seen and the screens' count, and
//! each kind combined is charged to the work budget.

use crate::budget::{self, Work};
use crate::error::Error;
use crate::json;
use crate::legibility::{KEY_ROLES, LEGAL_ROLES, TextClass};
use crate::model::*;
use crate::resolve::{Key, Requirements, intersect, parse_ref, screen_key};
use crate::stream::{Sink, walk};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The strictest requirements of everything a reference covers.
pub struct Strictest {
    reference: String,
    screens: usize,
    seen: HashSet<Vec<u8>>,
    key: Vec<u8>,
    error: Option<String>,

    format: String,
    pixels: Option<([i64; 2], String)>,
    physical: Option<Option<[f64; 2]>>,
    physical_same: bool,

    accepted: Option<Vec<String>>,
    statics: Option<Vec<String>>,
    motions: Option<Vec<String>>,
    codecs: Option<Vec<String>>,
    slot_s: Pick<f64>,
    slot_longest: Pick<f64>,
    fps: Pick<f64>,
    max_bytes: Pick<i64>,
    colour: Option<(String, String)>,
    colour_differs: bool,
    audio_off: Flag,
    audio_on: bool,

    safe: bool,
    insets: [Pick<i64>; 4],
    regions: Vec<Region>,
    region_keys: HashSet<Vec<u8>>,
    segments: Option<Vec<Segment>>,
    segments_same: bool,

    near_m: Pick<f64>,
    far_m: Pick<f64>,
    dwell_s: Pick<f64>,

    caps: BTreeMap<String, (i64, String)>,
    elements: Vec<Combined>,
    element_at: HashMap<String, usize>,
    conditional: Vec<(String, String)>,
    conditional_seen: HashSet<(String, String)>,

    max_words: Pick<i64>,
    categories: Uniq,
    packs: Uniq,
    rule_notes: Uniq,
    conflicts: Uniq,
    motion_off: Flag,
    motion_on: bool,
    no_flashing: Flag,
    flash_hz: Pick<f64>,
    frame_hold_s: Pick<f64>,
    night: Pick<f64>,
    day: Pick<f64>,
    min_dwell: Pick<f64>,
    max_transition: Pick<f64>,

    notes: Vec<String>,
    noted: HashSet<String>,
}

/// A required element as combined: where it may sit, and what was said of it.
struct Combined {
    role: String,
    within: Option<Rect>,
    notes: Vec<String>,
}

/// The extreme value seen of a limit, and the first screen that had it.
struct Pick<T>(Option<(T, String)>);

impl<T> Default for Pick<T> {
    fn default() -> Self {
        Pick(None)
    }
}

impl<T: PartialOrd + Copy> Pick<T> {
    fn smaller(&mut self, v: Option<T>, screen: &str) {
        if let Some(v) = v
            && self.0.as_ref().is_none_or(|(x, _)| v < *x)
        {
            self.0 = Some((v, screen.to_string()));
        }
    }

    fn larger(&mut self, v: Option<T>, screen: &str) {
        if let Some(v) = v
            && self.0.as_ref().is_none_or(|(x, _)| v > *x)
        {
            self.0 = Some((v, screen.to_string()));
        }
    }

    fn value(&self) -> Option<T> {
        self.0.as_ref().map(|(v, _)| *v)
    }
}

/// Whether some screen set something, and the first that did.
#[derive(Default)]
struct Flag(Option<String>);

impl Flag {
    fn raise(&mut self, screen: &str) {
        self.0.get_or_insert_with(|| screen.to_string());
    }
}

/// A list of strings kept in the order first seen, with no repeats.
#[derive(Default)]
struct Uniq {
    list: Vec<String>,
    set: HashSet<String>,
}

impl Uniq {
    fn add(&mut self, s: &str) {
        if !self.set.contains(s) {
            self.set.insert(s.to_string());
            self.list.push(s.to_string());
        }
    }
}

/// Records which screen set a value, where one is known.
fn from<'a, T>(into: &mut BTreeMap<String, &'a str>, key: &str, pick: &'a Option<(T, String)>) {
    if let Some((_, s)) = pick
        && !s.is_empty()
    {
        into.insert(key.to_string(), s.as_str());
    }
}

fn flag<'a>(into: &mut BTreeMap<String, &'a str>, key: &str, f: &'a Flag) {
    if let Some(s) = f.0.as_deref().filter(|s| !s.is_empty()) {
        into.insert(key.to_string(), s);
    }
}

fn has(list: &[String], s: &str) -> bool {
    list.iter().any(|t| t == s)
}

/// Narrows `slot` to what is also in `list`, keeping the order of the first.
fn meet<'a>(slot: &mut Option<Vec<String>>, list: impl Iterator<Item = &'a String> + Clone) {
    match slot {
        None => {
            let mut first: Vec<String> = Vec::new();
            for t in list {
                if !has(&first, t) {
                    first.push(t.clone());
                }
            }
            *slot = Some(first);
        }
        Some(cur) => {
            if cur.is_empty() {
                return;
            }
            if cur.len() > 8 {
                let set: HashSet<&str> = list.map(String::as_str).collect();
                cur.retain(|t| set.contains(t.as_str()));
            } else {
                let list: Vec<&String> = list.collect();
                cur.retain(|t| list.contains(&t));
            }
        }
    }
}

fn label(id: &str) -> String {
    if id.is_empty() {
        "the format".to_string()
    } else {
        format!("screen {}", crate::gofmt::quote(id))
    }
}

impl Strictest {
    fn new(reference: String) -> Strictest {
        Strictest {
            reference,
            screens: 0,
            seen: HashSet::new(),
            key: Vec::new(),
            error: None,
            format: String::new(),
            pixels: None,
            physical: None,
            physical_same: true,
            accepted: None,
            statics: None,
            motions: None,
            codecs: None,
            slot_s: Pick::default(),
            slot_longest: Pick::default(),
            fps: Pick::default(),
            max_bytes: Pick::default(),
            colour: None,
            colour_differs: false,
            audio_off: Flag::default(),
            audio_on: false,
            safe: false,
            insets: Default::default(),
            regions: Vec::new(),
            region_keys: HashSet::new(),
            segments: None,
            segments_same: true,
            near_m: Pick::default(),
            far_m: Pick::default(),
            dwell_s: Pick::default(),
            caps: BTreeMap::new(),
            elements: Vec::new(),
            element_at: HashMap::new(),
            conditional: Vec::new(),
            conditional_seen: HashSet::new(),
            max_words: Pick::default(),
            categories: Uniq::default(),
            packs: Uniq::default(),
            rule_notes: Uniq::default(),
            conflicts: Uniq::default(),
            motion_off: Flag::default(),
            motion_on: false,
            no_flashing: Flag::default(),
            flash_hz: Pick::default(),
            frame_hold_s: Pick::default(),
            night: Pick::default(),
            day: Pick::default(),
            min_dwell: Pick::default(),
            max_transition: Pick::default(),
            notes: Vec::new(),
            noted: HashSet::new(),
        }
    }

    /// Combines the requirements for `reference` in the document `raw`. The
    /// errors are those of [`crate::stream::Resolved::from_text`], and a
    /// reference that covers screens of different pixel sizes, which no
    /// file can fit.
    pub fn from_text(raw: &[u8], reference: &str) -> Result<Strictest, Error> {
        let shown = parse_ref(reference).map_or_else(|_| reference.to_string(), |r| r.to_string());
        let mut s = Strictest::new(shown);
        walk(raw, reference, &mut s)?;
        match s.error.take() {
            Some(e) => Err(Error::Message(e)),
            None => Ok(s),
        }
    }

    /// Says something once, however many screens it holds for: `key` names
    /// what it is about.
    fn note_once(&mut self, key: &str, text: impl FnOnce() -> String) {
        if !self.noted.contains(key) {
            self.noted.insert(key.to_string());
            self.notes.push(text());
        }
    }

    fn cap(&mut self, role: &str, px: i64, screen: &str) {
        match self.caps.get_mut(role) {
            Some(e) if e.0 >= px => {}
            Some(e) => *e = (px, screen.to_string()),
            None => {
                self.caps.insert(role.to_string(), (px, screen.to_string()));
            }
        }
    }

    /// Combines one screen's (or a format's own) requirements in.
    fn fold(&mut self, q: &Requirements<'_>) {
        let id = q.screen_id();
        if self.pixels.is_none() {
            self.format = q.format_id.to_string();
            self.pixels = Some((q.pixels, id.to_string()));
        } else if let Some((p, first)) = &self.pixels
            && *p != q.pixels
        {
            self.error = Some(format!(
                "{} covers screens of different pixel sizes, {}x{} on {} and {}x{} on {}; no single file fits both",
                self.reference,
                p[0],
                p[1],
                label(first),
                q.pixels[0],
                q.pixels[1],
                label(id)
            ));
            return;
        }
        match &self.physical {
            None => self.physical = Some(q.physical_mm),
            Some(p) if *p != q.physical_mm => self.physical_same = false,
            _ => {}
        }
        self.media(q, id);
        self.layout(q, id);
        self.text(q, id);
        self.rules(q, id);
    }

    fn media(&mut self, q: &Requirements<'_>, id: &str) {
        let m = &*q.media;
        let (st, mo) = (
            m.static_types.as_deref().unwrap_or_default(),
            m.motion.as_deref().unwrap_or_default(),
        );
        // A format that lists no static types takes none, and one with no
        // motion types takes no motion: a file must be listed by each screen.
        meet(&mut self.accepted, st.iter().chain(mo));
        meet(&mut self.statics, st.iter());
        meet(&mut self.motions, mo.iter());
        if let Some(c) = m.codecs.as_deref().filter(|c| !c.is_empty()) {
            meet(&mut self.codecs, c.iter());
        }
        self.slot_s.smaller(m.slot_s, id);
        self.slot_longest.larger(m.slot_s, id);
        self.fps.smaller(m.fps, id);
        self.max_bytes.smaller(m.max_bytes, id);
        if !m.colour_space.is_empty() {
            match &self.colour {
                None => self.colour = Some((m.colour_space.clone(), id.to_string())),
                Some((c, _)) if *c != m.colour_space => self.colour_differs = true,
                _ => {}
            }
        }
        match m.audio {
            Some(false) => self.audio_off.raise(id),
            Some(true) => self.audio_on = true,
            None => {}
        }
    }

    fn layout(&mut self, q: &Requirements<'_>, id: &str) {
        if let Some(sa) = q.safe_area {
            self.safe = true;
            for (pick, v) in self
                .insets
                .iter_mut()
                .zip([sa.top, sa.right, sa.bottom, sa.left])
            {
                pick.larger(Some(v), id);
            }
        }
        let mut key = Vec::new();
        for r in q.blocked_regions.iter() {
            key.clear();
            r.key(&mut key);
            if !self.region_keys.contains(&key) {
                self.region_keys.insert(key.clone());
                self.regions.push(r.clone());
            }
        }
        match &self.segments {
            None => self.segments = Some(q.segments.to_vec()),
            Some(s) if s[..] != q.segments[..] => self.segments_same = false,
            _ => {}
        }
        if let Some(v) = q.screen.and_then(|s| s.viewing.as_ref()) {
            self.near_m.smaller(v.distance_m.map(|d| d[0]), id);
            self.far_m.larger(v.distance_m.map(|d| d[1]), id);
            self.dwell_s.smaller(v.dwell_s, id);
        }
    }

    /// Minimum text sizes, in pixels on this screen, for each role.
    fn text(&mut self, q: &Requirements<'_>, id: &str) {
        for (class, roles) in [
            (TextClass::Key, &KEY_ROLES[..]),
            (TextClass::Legal, &LEGAL_ROLES[..]),
        ] {
            let (m, applies, missing) = q.min_cap_height(class);
            if !applies {
                continue;
            }
            if !missing.is_empty() {
                self.note_once(missing, || {
                    format!(
                        "The minimum text size can't be worked out on {}: {missing}. It is missing from min_cap_height_px, or only a floor there.",
                        label(id)
                    )
                });
            }
            if m.px > 0 {
                for role in roles {
                    self.cap(role, m.px, id);
                }
            }
        }
        for e in q.rules.required_elements.iter() {
            if !e.when.is_empty() {
                let k = (e.role.clone(), e.when.clone());
                if !self.conditional_seen.contains(&k) {
                    self.conditional_seen.insert(k.clone());
                    self.conditional.push(k);
                }
                continue;
            }
            let mut px = e.min_cap_height_px;
            if let Some(mm) = e.min_cap_height_mm {
                match q.mm_to_px(mm) {
                    Some(p) => px = Some(px.map_or(p, |x| x.max(p))),
                    None => self.note_once("element mm", || {
                        format!(
                            "A required element's minimum size in millimetres can't be turned into pixels on {}: no physical_mm. It is missing from min_cap_height_px.",
                            label(id)
                        )
                    }),
                }
            }
            if let Some(px) = px {
                self.cap(&e.role, px, id);
            }
            let at = match self.element_at.get(&e.role) {
                Some(&i) => i,
                None => {
                    self.elements.push(Combined {
                        role: e.role.clone(),
                        within: None,
                        notes: Vec::new(),
                    });
                    self.element_at
                        .insert(e.role.clone(), self.elements.len() - 1);
                    self.elements.len() - 1
                }
            };
            let acc = &mut self.elements[at];
            if let Some(w) = e.within_rect_px {
                acc.within = Some(acc.within.map_or(w, |x| intersect(x, w)));
            }
            if !e.notes.is_empty() && !has(&acc.notes, &e.notes) {
                acc.notes.push(e.notes.clone());
            }
        }
    }

    fn rules(&mut self, q: &Requirements<'_>, id: &str) {
        let r = &q.rules;
        self.max_words.smaller(q.word_limit().map(|(n, _)| n), id);
        for c in r.restricted_categories.iter() {
            self.categories.add(c);
        }
        for p in r.rule_packs.iter() {
            self.packs.add(p);
        }
        for n in r.notes.iter() {
            self.rule_notes.add(n);
        }
        for c in r.conflicts.iter() {
            self.conflicts.add(c);
        }
        let m = &r.motion;
        let placed_off = q
            .screen
            .and_then(|s| s.placement.as_ref())
            .is_some_and(|p| p.motion_allowed == Some(false));
        match m.allowed {
            Some(false) => self.motion_off.raise(id),
            Some(true) if !placed_off => self.motion_on = true,
            _ => {}
        }
        if placed_off {
            self.motion_off.raise(id);
        }
        if m.no_flashing == Some(true) {
            self.no_flashing.raise(id);
        }
        self.flash_hz.smaller(m.max_flash_hz, id);
        self.frame_hold_s.larger(m.min_frame_hold_s, id);
        self.night.smaller(r.brightness.max_night_cd_m2, id);
        self.day.smaller(r.brightness.max_day_cd_m2, id);
        self.min_dwell.larger(r.playback.min_dwell_s, id);
        self.max_transition.smaller(r.playback.max_transition_s, id);
    }
}

impl Sink for Strictest {
    fn skip(&mut self, s: &Screen) -> bool {
        if self.error.is_some() || budget::tripped() {
            return true;
        }
        self.key.clear();
        screen_key(s, &mut self.key);
        if self.seen.contains(&self.key) {
            self.screens += 1;
            return true;
        }
        false
    }

    fn add(&mut self, q: &Requirements<'_>) {
        if self.error.is_some() {
            return;
        }
        // Each kind of screen is combined once; what that costs is in the
        // size of its lists.
        let units = (q.rules.required_elements.len()
            + q.rules.rule_packs.len()
            + q.rules.restricted_categories.len()
            + q.rules.conflicts.len()
            + q.blocked_regions.len()
            + 8) as u64
            * 24;
        if !budget::charge(Work::Strictest, q.network as *const _ as usize, units) {
            return;
        }
        if q.screen.is_some() {
            self.screens += 1;
        }
        self.key.clear();
        if q.key(&mut self.key) {
            self.seen.insert(self.key.clone());
        }
        self.fold(q);
    }
}

// The answer.

#[derive(Serialize)]
struct Out<'a> {
    #[serde(rename = "ref")]
    reference: &'a str,
    screens: usize,
    format: &'a str,
    pixels: [i64; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    physical_mm: Option<[f64; 2]>,
    media: MediaOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    safe_area: Option<Insets>,
    #[serde(skip_serializing_if = "<[Region]>::is_empty")]
    blocked_regions: &'a [Region],
    #[serde(skip_serializing_if = "<[Segment]>::is_empty")]
    segments: &'a [Segment],
    #[serde(skip_serializing_if = "Option::is_none")]
    viewing: Option<ViewingOut>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    min_cap_height_px: BTreeMap<&'a str, i64>,
    rules: RulesOut<'a>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    conditional_elements_left_out: Vec<Left<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
    sources: BTreeMap<String, &'a str>,
}

#[derive(Serialize)]
struct MediaOut {
    #[serde(rename = "static", skip_serializing_if = "Option::is_none")]
    static_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    motion: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slot_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    codecs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_bytes: Option<i64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    colour_space: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<bool>,
}

#[derive(Serialize)]
struct ViewingOut {
    /// The nearest near end and the farthest far end of the screens' ranges.
    #[serde(skip_serializing_if = "Option::is_none")]
    distance_m: Option<[f64; 2]>,
    /// The shortest dwell.
    #[serde(skip_serializing_if = "Option::is_none")]
    dwell_s: Option<f64>,
}

#[derive(Serialize)]
struct RulesOut<'a> {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    required_elements: Vec<RequiredElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_words: Option<i64>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    restricted_categories: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    motion: Option<MotionRules>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brightness: Option<Brightness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    playback: Option<Playback>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    rule_packs: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    notes: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    conflicts: &'a [String],
}

#[derive(Serialize)]
struct Left<'a> {
    role: &'a str,
    when: &'a str,
}

impl Strictest {
    /// How many screens the reference covers.
    pub fn screens(&self) -> usize {
        self.screens
    }

    /// The answer as JSON, as Go would write it.
    pub fn to_json(&self) -> Vec<u8> {
        self.render(false)
    }

    /// The answer as indented JSON, as Go's `MarshalIndent` writes it.
    pub fn to_json_pretty(&self) -> Vec<u8> {
        self.render(true)
    }

    fn render(&self, pretty: bool) -> Vec<u8> {
        let mut notes = self.notes.clone();
        let mut sources: BTreeMap<String, &str> = BTreeMap::new();
        let (pixels, pixels_at) = self
            .pixels
            .as_ref()
            .map_or(([0, 0], None), |(p, s)| (*p, Some(s)));
        if let Some(s) = pixels_at.filter(|s| !s.is_empty()) {
            sources.insert("pixels".into(), s);
        }

        // What a file may be: listed by every screen.
        let accepted = self.accepted.clone().unwrap_or_default();
        let (mut stat, mut motion) = (
            self.statics.clone().unwrap_or_default(),
            self.motions.clone().unwrap_or_default(),
        );
        let mut media = MediaOut {
            static_types: None,
            motion: None,
            slot_s: self.slot_s.value(),
            fps: self.fps.value(),
            codecs: self.codecs.clone(),
            max_bytes: self.max_bytes.value(),
            colour_space: String::new(),
            audio: if self.audio_off.0.is_some() {
                Some(false)
            } else {
                self.audio_on.then_some(true)
            },
        };
        if accepted.is_empty() {
            notes.push(
                "No file type is accepted by every screen, so no single file can satisfy them all. media.static is empty."
                    .into(),
            );
            media.static_types = Some(Vec::new());
        } else {
            // A type listed as static by one screen and motion by another
            // is placed by its kind.
            for t in &accepted {
                if !has(&stat, t) && !has(&motion, t) {
                    if t.starts_with("video/") {
                        motion.push(t.clone());
                    } else {
                        stat.push(t.clone());
                    }
                }
            }
            media.static_types = (!stat.is_empty()).then_some(stat);
            media.motion = (!motion.is_empty()).then_some(motion);
        }
        if self.codecs.as_ref().is_some_and(Vec::is_empty) {
            notes.push("No codec is accepted by every screen, so no single file can satisfy them all. media.codecs is empty.".into());
        }
        if let (Some((v, at)), Some((longest, _))) = (&self.slot_s.0, &self.slot_longest.0)
            && v != longest
        {
            notes.push(format!(
                "Screens have different slot lengths, {v} s to {longest} s. A motion file must run for the slot, so no single motion file can satisfy them all; media.slot_s is the shortest, from {}.",
                label(at)
            ));
        }
        if let Some((c, _)) = &self.colour {
            if self.colour_differs {
                notes.push("Screens name different colour spaces, so no single file can satisfy them all. media.colour_space is empty.".into());
            } else {
                media.colour_space = c.clone();
            }
        }
        from(&mut sources, "media.slot_s", &self.slot_s.0);
        from(&mut sources, "media.fps", &self.fps.0);
        from(&mut sources, "media.max_bytes", &self.max_bytes.0);
        if !self.colour_differs {
            from(&mut sources, "media.colour_space", &self.colour);
        }
        flag(&mut sources, "media.audio", &self.audio_off);

        let safe_area = self.safe.then(|| {
            let v = |i: usize| self.insets[i].value().unwrap_or(0);
            Insets {
                top: v(0),
                right: v(1),
                bottom: v(2),
                left: v(3),
            }
        });
        if self.safe {
            for (k, p) in ["top", "right", "bottom", "left"].iter().zip(&self.insets) {
                from(&mut sources, &format!("safe_area.{k}"), &p.0);
            }
        }

        let segments: &[Segment] = match &self.segments {
            Some(s) if self.segments_same => s,
            Some(_) => {
                notes.push("Screens have different segments, which are not combined; segments is left out.".into());
                &[]
            }
            None => &[],
        };

        let viewing = {
            let distance_m = self
                .near_m
                .value()
                .zip(self.far_m.value())
                .map(|(n, f)| [n, f]);
            let dwell_s = self.dwell_s.value();
            from(&mut sources, "viewing.distance_m.near", &self.near_m.0);
            from(&mut sources, "viewing.distance_m.far", &self.far_m.0);
            from(&mut sources, "viewing.dwell_s", &self.dwell_s.0);
            (distance_m.is_some() || dwell_s.is_some()).then_some(ViewingOut {
                distance_m,
                dwell_s,
            })
        };

        let mut min_cap_height_px = BTreeMap::new();
        for (role, (px, at)) in &self.caps {
            min_cap_height_px.insert(role.as_str(), *px);
            if !at.is_empty() {
                sources.insert(format!("min_cap_height_px.{role}"), at.as_str());
            }
        }

        let required_elements: Vec<RequiredElement> = self
            .elements
            .iter()
            .map(|e| {
                if e.within.is_some_and(|w| w.0[2] == 0 || w.0[3] == 0) {
                    notes.push(format!(
                        "The position limits of the {} element on different screens don't overlap, so no single file can satisfy them all.",
                        e.role
                    ));
                }
                RequiredElement {
                    role: e.role.clone(),
                    within_rect_px: e.within,
                    notes: e.notes.join("; "),
                    ..RequiredElement::default()
                }
            })
            .collect();

        if !self.packs.list.is_empty() {
            notes.push("Rule packs are listed, not combined: what they allow depends on each screen's placement (speed, road facing, zone), so check a file against the screens it is for.".into());
        }
        from(&mut sources, "rules.max_words", &self.max_words.0);
        flag(&mut sources, "rules.motion.allowed", &self.motion_off);
        flag(&mut sources, "rules.motion.no_flashing", &self.no_flashing);
        from(&mut sources, "rules.motion.max_flash_hz", &self.flash_hz.0);
        from(
            &mut sources,
            "rules.motion.min_frame_hold_s",
            &self.frame_hold_s.0,
        );
        from(
            &mut sources,
            "rules.brightness.max_night_cd_m2",
            &self.night.0,
        );
        from(&mut sources, "rules.brightness.max_day_cd_m2", &self.day.0);
        from(
            &mut sources,
            "rules.playback.min_dwell_s",
            &self.min_dwell.0,
        );
        from(
            &mut sources,
            "rules.playback.max_transition_s",
            &self.max_transition.0,
        );

        let motion_rules = MotionRules {
            allowed: if self.motion_off.0.is_some() {
                Some(false)
            } else {
                self.motion_on.then_some(true)
            },
            no_flashing: self.no_flashing.0.is_some().then_some(true),
            max_flash_hz: self.flash_hz.value(),
            min_frame_hold_s: self.frame_hold_s.value(),
        };
        let brightness = Brightness {
            max_night_cd_m2: self.night.value(),
            max_day_cd_m2: self.day.value(),
        };
        let playback = Playback {
            min_dwell_s: self.min_dwell.value(),
            max_transition_s: self.max_transition.value(),
        };
        let rules = RulesOut {
            required_elements,
            max_words: self.max_words.value(),
            restricted_categories: &self.categories.list,
            motion: (motion_rules != MotionRules::default()).then_some(motion_rules),
            brightness: (brightness != Brightness::default()).then_some(brightness),
            playback: (playback != Playback::default()).then_some(playback),
            rule_packs: &self.packs.list,
            notes: &self.rule_notes.list,
            conflicts: &self.conflicts.list,
        };

        let out = Out {
            reference: &self.reference,
            screens: self.screens,
            format: &self.format,
            pixels,
            physical_mm: if self.physical_same {
                self.physical.flatten()
            } else {
                None
            },
            media,
            safe_area,
            blocked_regions: &self.regions,
            segments,
            viewing,
            min_cap_height_px,
            rules,
            conditional_elements_left_out: self
                .conditional
                .iter()
                .map(|(role, when)| Left { role, when })
                .collect(),
            notes,
            sources,
        };
        if pretty {
            json::to_string_pretty(&out).into_bytes()
        } else {
            json::to_string(&out).into_bytes()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::decode_document;
    use serde_json::{Value, json};

    fn root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn strictest(raw: &[u8], reference: &str) -> Result<Value, String> {
        Strictest::from_text(raw, reference)
            .map(|s| serde_json::from_slice(&s.to_json()).unwrap())
            .map_err(|e| e.to_string())
    }

    /// The same combination from the decoded document, every screen folded
    /// in with none skipped.
    fn decoded(raw: &[u8], reference: &str) -> Result<Value, String> {
        let d = decode_document(raw).map_err(|e| e.to_string())?;
        let r = parse_ref(reference).map_err(|e| e.to_string())?;
        let mut s = Strictest::new(r.to_string());
        let reqs = d.resolve(&r).map_err(|e| e.to_string())?;
        for q in &reqs {
            s.add(q);
        }
        if let Some(e) = s.error.take() {
            return Err(e);
        }
        Ok(serde_json::from_slice(&s.to_json()).unwrap())
    }

    fn harbourline() -> Value {
        serde_json::from_slice(
            &std::fs::read(root().join("examples/harbourline/retail.screens.json")).unwrap(),
        )
        .unwrap()
    }

    /// Skipping screens of a kind already combined, and reading the text a
    /// screen at a time, give what combining every screen of the decoded
    /// document gives, errors included.
    #[test]
    fn streamed_matches_combining_every_screen() {
        let base = harbourline();
        let mut docs = vec![base.clone()];
        let mut with = |f: &dyn Fn(&mut Value)| {
            let mut v = base.clone();
            f(&mut v);
            docs.push(v);
        };
        with(&|v| {
            let s = v["networks"][0]["screens"].as_array().unwrap().clone();
            let mut all = Vec::new();
            for i in 0..30 {
                let mut x = s[i % 3].clone();
                x["id"] = format!("X{}", i % 20).into();
                if i % 4 == 0 {
                    x["overrides"] = json!({"safe_area": {"top": 3 * i}, "blocked_regions":
                        [{"name": format!("r{i}"), "rect_px": [0, 0, 5, 5]}]});
                }
                if i % 5 == 0 {
                    x["rules"] = json!({"max_words": 3 + i, "required_elements": [
                        {"role": "legal", "when": "offer", "within_rect_px": [0, 0, 10, 10]},
                        {"role": "logo", "min_cap_height_mm": 4, "within_rect_px": [0, 0, 500, 500]}]});
                }
                if i % 6 == 0 {
                    x["viewing"] = json!({"distance_m": [1, 3 + i], "dwell_s": 2 + i});
                }
                all.push(x);
            }
            v["networks"][0]["screens"] = all.into();
        });
        with(&|v| v["networks"][0]["screens"] = json!([]));
        with(&|v| v["networks"][0]["screens"][0]["format"] = "gone".into());
        with(&|v| v["networks"][0]["screens"][1]["venue"] = "nowhere".into());
        with(&|v| {
            let n = v["networks"][0].clone();
            let mut m = n.clone();
            m["id"] = "second".into();
            v["networks"] = json!([n, m]);
        });
        with(&|v| v["networks"][0]["screens"][1]["overrides"] = json!({"pixels": [100, 100]}));
        with(&|v| v["networks"][0]["screens"][0]["pixels"] = "typed badly".into());
        let refs = [
            "harbourline.example/retail/portrait-totem-75",
            "harbourline.example/retail/atrium-wrap",
            "harbourline.example/retail/screen/HC-L1-002",
            "harbourline.example/retail/screen/X3",
            "harbourline.example/retail/nope",
            "harbourline.example/second/portrait-totem-75",
            "other.example/retail/portrait-totem-75",
            "not a reference",
        ];
        for (i, v) in docs.iter().enumerate() {
            let text = serde_json::to_vec(v).unwrap();
            for r in refs {
                assert_eq!(strictest(&text, r), decoded(&text, r), "document {i}, {r}");
            }
        }
    }

    /// A screen reference gives that screen's own requirements, field for
    /// field, and a format gives the stricter of its screens.
    #[test]
    fn a_screen_is_its_own_strictest_and_a_format_the_stricter() {
        let raw = std::fs::read(root().join("examples/harbourline/retail.screens.json")).unwrap();
        let d = decode_document(&raw).unwrap();
        let one = strictest(&raw, "harbourline.example/retail/screen/HC-L1-002").unwrap();
        let q = d
            .resolve(&parse_ref("harbourline.example/retail/screen/HC-L1-002").unwrap())
            .unwrap();
        let view: Value = serde_json::from_str(&json::to_string(&q[0].view())).unwrap();
        assert_eq!(one["screens"], 1);
        for k in [
            "format",
            "pixels",
            "physical_mm",
            "media",
            "safe_area",
            "blocked_regions",
        ] {
            assert_eq!(one.get(k), view.get(k), "{k}");
        }
        assert_eq!(one["viewing"]["distance_m"], view["viewing"]["distance_m"]);
        let key = &view["derived"]["min_key_cap_height"]["px"];
        assert_eq!(&one["min_cap_height_px"]["headline"], key);
        assert_eq!(
            one["rules"]["max_words"],
            view["derived"]["max_words"]["value"]
        );

        // The two totems differ in viewing distance; the farther one drives.
        let both = strictest(&raw, "harbourline.example/retail/portrait-totem-75").unwrap();
        assert_eq!(both["screens"], 2);
        let a = strictest(&raw, "harbourline.example/retail/screen/HC-L1-001").unwrap();
        assert!(
            both["min_cap_height_px"]["headline"].as_i64()
                > a["min_cap_height_px"]["headline"].as_i64()
        );
        assert_eq!(both["sources"]["min_cap_height_px.headline"], "HC-L1-002");
    }

    /// The key written from a screen alone is the one its requirements write.
    #[test]
    fn screen_key_is_the_requirements_key() {
        let raw = std::fs::read(root().join("examples/harbourline/retail.screens.json")).unwrap();
        let d = decode_document(&raw).unwrap();
        for n in &d.networks {
            for s in &n.screens {
                let q = d
                    .resolve(
                        &parse_ref(&format!("harbourline.example/{}/screen/{}", n.id, s.id))
                            .unwrap(),
                    )
                    .unwrap();
                let (mut a, mut b) = (Vec::new(), Vec::new());
                assert!(q[0].key(&mut a));
                screen_key(s, &mut b);
                assert_eq!(a, b, "{}", s.id);
            }
        }
    }

    fn doc(format: Value, screens: Value) -> Vec<u8> {
        let v = json!({"opencrs": "0.1", "publisher": "p.example", "updated_at": "2026-01-01T00:00:00Z",
            "networks": [{"id": "n", "name": "N", "formats": [format], "screens": screens}]});
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn combines_each_kind_of_limit() {
        let f = json!({"id": "f", "pixels": [1000, 2000], "physical_mm": [1000, 2000],
            "media": {"static": ["image/png", "image/jpeg"], "motion": ["video/mp4"], "max_bytes": 900,
                "codecs": ["h264", "vp9"], "slot_s": 10},
            "safe_area": {"top": 10, "left": 5},
            "blocked_regions": [{"name": "logo", "rect_px": [0, 0, 10, 10]}],
            "rules": {"legibility": {"ratio": 100}, "max_words": 8,
                "required_elements": [{"role": "legal"}, {"role": "promo", "when": "offer"}]}});
        let screens = json!([
            {"id": "A", "format": "f", "viewing": {"distance_m": [1, 4], "dwell_s": 10}},
            {"id": "B", "format": "f", "viewing": {"distance_m": [2, 8], "dwell_s": 3},
             "overrides": {"safe_area": {"top": 4, "right": 7}, "media": {"static": ["image/png"],
                "max_bytes": 500, "codecs": ["h264"]},
                "blocked_regions": [{"name": "clock", "rect_px": [0, 0, 5, 5]}]},
             "rules": {"max_words": 5}}
        ]);
        let v = strictest(&doc(f, screens), "p.example/n/f").unwrap();
        assert_eq!(v["screens"], 2);
        assert_eq!(
            v["safe_area"],
            json!({"top": 10, "right": 7, "bottom": 0, "left": 5})
        );
        assert_eq!(v["sources"]["safe_area.top"], "A");
        assert_eq!(v["sources"]["safe_area.right"], "B");
        assert_eq!(v["blocked_regions"].as_array().unwrap().len(), 2);
        assert_eq!(v["media"]["static"], json!(["image/png"]));
        assert_eq!(v["media"]["motion"], json!(["video/mp4"]));
        assert_eq!(v["media"]["codecs"], json!(["h264"]));
        assert_eq!(v["media"]["max_bytes"], 500);
        assert_eq!(v["sources"]["media.max_bytes"], "B");
        assert_eq!(v["rules"]["max_words"], 5);
        // 8 m at ratio 100 is 80 mm, which is 80 px at 1 px/mm; the near end
        // of 1 m is 10 mm for legal text, but B's 2 m is 20.
        assert_eq!(v["min_cap_height_px"]["headline"], 80);
        assert_eq!(v["sources"]["min_cap_height_px.headline"], "B");
        assert_eq!(v["min_cap_height_px"]["legal"], 20);
        assert_eq!(v["rules"]["required_elements"], json!([{"role": "legal"}]));
        assert_eq!(
            v["conditional_elements_left_out"],
            json!([{"role": "promo", "when": "offer"}])
        );
        assert_eq!(v["viewing"], json!({"distance_m": [1, 8], "dwell_s": 3}));
        assert!(v.get("notes").is_none(), "{v}");
        // One slot length, so no note about it.
        assert_eq!(v["media"]["slot_s"], 10);
    }

    #[test]
    fn says_where_no_single_file_can_pass() {
        let f = json!({"id": "f", "pixels": [10, 10], "media": {"static": ["image/png"], "colour_space": "srgb",
            "codecs": ["h264"], "slot_s": 10}});
        let screens = json!([
            {"id": "A", "format": "f"},
            {"id": "B", "format": "f", "overrides": {"media": {"static": ["image/jpeg"], "codecs": ["vp9"],
                "colour_space": "p3", "slot_s": 15}}}
        ]);
        let v = strictest(&doc(f, screens), "p.example/n/f").unwrap();
        assert_eq!(v["media"]["static"], json!([]));
        assert_eq!(v["media"]["codecs"], json!([]));
        assert!(v["media"].get("colour_space").is_none());
        assert_eq!(v["media"]["slot_s"], 10);
        assert_eq!(v["notes"].as_array().unwrap().len(), 4, "{v}");
    }

    #[test]
    fn different_pixels_are_an_error() {
        let f = json!({"id": "f", "pixels": [10, 10], "media": {"static": ["image/png"]}});
        let screens = json!([{"id": "A", "format": "f"},
            {"id": "B", "format": "f", "overrides": {"pixels": [20, 10]}}]);
        let e = strictest(&doc(f, screens), "p.example/n/f").unwrap_err();
        assert!(
            e.contains("different pixel sizes") && e.contains("\"A\"") && e.contains("\"B\""),
            "{e}"
        );
    }

    /// A budget that runs out stops the combination with the work's reason.
    #[test]
    fn a_document_too_complex_to_combine_stops() {
        let mut els = Vec::new();
        for i in 0..2000 {
            els.push(json!({"role": format!("r{i}"), "min_cap_height_mm": 1}));
        }
        let f = json!({"id": "f", "pixels": [10, 10], "media": {"static": ["image/png"]},
            "rules": {"required_elements": els}});
        let screens: Vec<Value> = (0..6000)
            .map(|i| json!({"id": format!("s{i}"), "format": "f", "viewing": {"dwell_s": 1 + i}}))
            .collect();
        let raw = doc(f, screens.into());
        let r = budget::ensure(raw.len(), || strictest(&raw, "p.example/n/f"));
        assert!(r.unwrap_err().starts_with("CRS-TOO-COMPLEX: "));
    }
}
