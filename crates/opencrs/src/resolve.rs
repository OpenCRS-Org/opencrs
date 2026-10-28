//! Resolving a reference to the requirements for its screens, after
//! inheritance and overrides. Port of `crs/resolve.go`.

use crate::error::Error;
use crate::gofmt::quote;
use crate::model::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

/// A parsed global reference to a format or a screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ref {
    pub publisher: String,
    pub network: String,
    /// Set for a format reference.
    pub format: String,
    /// Set for a screen reference.
    pub screen: String,
}

/// Parses `<publisher>/<network>/<format>` or
/// `<publisher>/<network>/screen/<screen>`.
pub fn parse_ref(s: &str) -> Result<Ref, Error> {
    let parts: Vec<&str> = s.split('/').collect();
    match parts.as_slice() {
        [p, n, f] if !p.is_empty() && !n.is_empty() && !f.is_empty() => Ok(Ref {
            publisher: p.to_string(),
            network: n.to_string(),
            format: f.to_string(),
            screen: String::new(),
        }),
        [p, n, "screen", sc] if !p.is_empty() && !n.is_empty() && !sc.is_empty() => Ok(Ref {
            publisher: p.to_string(),
            network: n.to_string(),
            format: String::new(),
            screen: sc.to_string(),
        }),
        _ => Err(Error::msg(format!(
            "{} is not a format or screen reference",
            quote(s)
        ))),
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.screen.is_empty() {
            write!(
                f,
                "{}/{}/screen/{}",
                self.publisher, self.network, self.screen
            )
        } else {
            write!(f, "{}/{}/{}", self.publisher, self.network, self.format)
        }
    }
}

/// Why a reference didn't resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// A screen naming a format or venue its network doesn't define: a
    /// fault in the document, not in the creative's target.
    Reference(String),
    /// The reference names something the document doesn't have.
    Other(String),
    /// The work budget ran out; the text says why.
    TooComplex(String),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::Reference(m) | ResolveError::Other(m) | ResolveError::TooComplex(m) => {
                f.write_str(m)
            }
        }
    }
}

impl std::error::Error for ResolveError {}

impl From<ResolveError> for Error {
    fn from(e: ResolveError) -> Self {
        Error::Message(e.to_string())
    }
}

/// Everything one screen (or a format with no screens) asks of creative,
/// after inheritance and overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Requirements<'a> {
    pub publisher: &'a str,
    pub network: &'a Network,
    pub format_id: &'a str,
    /// None if the screen names no venue.
    pub venue: Option<&'a Venue>,
    /// None for a format with no screens.
    pub screen: Option<&'a Screen>,

    pub pixels: [i64; 2],
    pub physical_mm: Option<[f64; 2]>,
    pub media: Media,
    pub safe_area: Option<Insets>,
    pub blocked_regions: Vec<Region>,
    pub segments: Vec<Segment>,
    pub rules: Rules,
}

impl Requirements<'_> {
    /// A fingerprint of everything about a screen's requirements except its
    /// id: the venue and format it uses, and the parts of the screen that
    /// change what is asked of creative (viewing, placement, overrides and
    /// the screen's own rules). Screens with the same fingerprint have the
    /// same requirements, so a check or a view of one holds for all. None
    /// for a requirement that isn't a screen's.
    pub(crate) fn fingerprint(&self) -> Option<[u8; 32]> {
        use sha2::{Digest, Sha256};
        let s = self.screen?;
        let mut h = Sha256::new();
        let venue = self.venue.map_or("", |v| v.id.as_str());
        for part in [venue, self.format_id] {
            h.update((part.len() as u64).to_le_bytes());
            h.update(part.as_bytes());
        }
        let parts = serde_json::to_vec(&(&s.placement, &s.viewing, &s.overrides, &s.rules)).ok()?;
        h.update(&parts);
        Some(h.finalize().into())
    }

    /// The screen's id, or "" for a format-only requirement.
    pub fn screen_id(&self) -> &str {
        self.screen.map_or("", |s| s.id.as_str())
    }
}

fn arc_empty<T>(v: &Arc<Vec<T>>) -> bool {
    v.is_empty()
}

/// Which level of the chain a rule came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Network,
    Venue,
    Format,
    ScreenOverrides,
    Screen,
}

/// What a recorded conflict is about: the level that restated an element,
/// and the element, so the same conflict can be recognised whatever the
/// levels above it changed in its wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConflictCause {
    pub(crate) level: Level,
    pub(crate) role: String,
    pub(crate) when: String,
}

/// A RuleSet after merging network, venue, format, then screen. The long
/// lists are shared (`Arc`), so a screen that adds nothing to them doesn't
/// copy them.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Rules {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legibility: Option<Legibility>,
    #[serde(skip_serializing_if = "arc_empty")]
    pub required_elements: Arc<Vec<RequiredElement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_words: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading_rate_wps: Option<f64>,
    #[serde(skip_serializing_if = "arc_empty")]
    pub restricted_categories: Arc<Vec<String>>,
    pub motion: MotionRules,
    pub brightness: Brightness,
    pub playback: Playback,
    #[serde(skip_serializing_if = "arc_empty")]
    pub rule_packs: Arc<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Rules that tightening has made impossible to meet, such as position
    /// limits that don't overlap.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// What each of `conflicts` is about, in the same order.
    #[serde(skip)]
    pub(crate) causes: Vec<ConflictCause>,
}

impl Document {
    /// Returns the requirements for a reference. A screen reference gives
    /// one result; a format reference gives one per screen using that
    /// format, or one format-only result if no screen uses it.
    pub fn resolve(&self, r: &Ref) -> Result<Vec<Requirements<'_>>, ResolveError> {
        let mut out = Vec::new();
        self.resolve_each(r, |q| out.push(q))?;
        Ok(out)
    }

    /// [`resolve`](Self::resolve), handing each result to `each` as it is
    /// worked out instead of collecting them, so a format with hundreds of
    /// thousands of screens never has all its requirements at once. On an
    /// error, `each` may already have seen the results before the fault.
    pub fn resolve_each<'a>(
        &'a self,
        r: &Ref,
        mut each: impl FnMut(Requirements<'a>),
    ) -> Result<(), ResolveError> {
        if r.publisher != self.publisher {
            return Err(ResolveError::Other(format!(
                "{r} belongs to {}, not {}",
                r.publisher, self.publisher
            )));
        }
        let n = self.network(&r.network).ok_or_else(|| {
            ResolveError::Other(format!(
                "network {} not found in {}",
                quote(&r.network),
                self.publisher
            ))
        })?;
        if !r.screen.is_empty() {
            let s = n.screen(&r.screen).ok_or_else(|| {
                ResolveError::Other(format!(
                    "screen {} not found in network {}",
                    quote(&r.screen),
                    quote(&n.id)
                ))
            })?;
            each(self.resolve_screen_with(&mut RuleCache::default(), n, s)?);
            return self.if_tripped();
        }
        let f = n.format(&r.format).ok_or_else(|| {
            ResolveError::Other(format!(
                "format {} not found in network {}",
                quote(&r.format),
                quote(&n.id)
            ))
        })?;
        let mut cache = RuleCache::default();
        let mut any = false;
        for s in n.screens_of(&f.id) {
            // Each reference to a format walks its screens.
            if !crate::budget::charge(crate::budget::Work::Screens, f as *const Format as usize, 8)
            {
                break;
            }
            each(self.resolve_screen_with(&mut cache, n, s)?);
            any = true;
        }
        if !any {
            each(self.build_with(&mut cache, n, f, None, None));
        }
        self.if_tripped()
    }

    /// Ok unless the work budget is spent, then the reason.
    pub(crate) fn if_tripped(&self) -> Result<(), ResolveError> {
        match crate::budget::explain(Some(self)) {
            Some(why) => Err(ResolveError::TooComplex(why)),
            None => Ok(()),
        }
    }

    fn network(&self, id: &str) -> Option<&Network> {
        self.networks.iter().find(|n| n.id == id)
    }

    pub(crate) fn resolve_screen_with<'a>(
        &'a self,
        cache: &mut RuleCache,
        n: &'a Network,
        s: &'a Screen,
    ) -> Result<Requirements<'a>, ResolveError> {
        let f = n.format(&s.format).ok_or_else(|| {
            ResolveError::Reference(format!(
                "screen {} uses format {}, which network {} doesn't define",
                quote(&s.id),
                quote(&s.format),
                quote(&n.id)
            ))
        })?;
        let mut v = None;
        if !s.venue.is_empty() {
            v = Some(n.venue(&s.venue).ok_or_else(|| {
                ResolveError::Reference(format!(
                    "screen {} names venue {}, which network {} doesn't define",
                    quote(&s.id),
                    quote(&s.venue),
                    quote(&n.id)
                ))
            })?);
        }
        Ok(self.build_with(cache, n, f, v, Some(s)))
    }

    /// The rules of a network and a venue of it, which hold for every screen
    /// at the venue whatever its format.
    pub(crate) fn venue_rules(&self, cache: &mut RuleCache, n: &Network, v: &Venue) -> Rules {
        cache.prefix(n, None, Some(v))
    }

    /// The rules of a network and one of its formats.
    pub(crate) fn format_rules(&self, cache: &mut RuleCache, n: &Network, f: &Format) -> Rules {
        cache.prefix(n, Some(f), None)
    }

    pub(crate) fn build_with<'a>(
        &'a self,
        cache: &mut RuleCache,
        n: &'a Network,
        f: &'a Format,
        v: Option<&'a Venue>,
        s: Option<&'a Screen>,
    ) -> Requirements<'a> {
        let overrides = s.and_then(|s| s.overrides.as_ref());
        let ff = match overrides {
            Some(o) => apply_overrides(&f.fields, o),
            None => f.fields.clone(),
        };
        // Rules merge network, then venue, then format, then screen. The
        // levels above the screen are the same for every screen with this
        // venue and format, so they are merged once.
        let mut rules = cache.prefix(n, Some(f), v);
        if let Some(s) = s {
            if let Some(o) = &s.overrides {
                rules.merge_at(o.fields.rules.as_ref(), Level::ScreenOverrides, || {
                    format!("screen {} overrides", s.id)
                });
            }
            rules.merge_at(s.rules.as_deref(), Level::Screen, || {
                format!("screen {}", s.id)
            });
        }
        Requirements {
            publisher: &self.publisher,
            network: n,
            format_id: &f.id,
            venue: v,
            screen: s,
            pixels: ff.pixels.unwrap_or_default(),
            physical_mm: ff.physical_mm,
            media: ff.media.unwrap_or_default(),
            safe_area: ff.safe_area,
            blocked_regions: ff.blocked_regions,
            segments: ff.segments.unwrap_or_default(),
            rules,
        }
    }
}

/// The position of `item` in `list`, if it is an element of it.
pub(crate) fn position_of<T>(list: &[T], item: &T) -> Option<usize> {
    let size = std::mem::size_of::<T>();
    let (base, at) = (list.as_ptr() as usize, item as *const T as usize);
    (size > 0 && at >= base && at < base + std::mem::size_of_val(list) && (at - base) % size == 0)
        .then(|| (at - base) / size)
}

/// Rules merged for the levels above a screen, by the venue and format in
/// play, for one network and one pass over its screens. Not kept anywhere:
/// every operation makes its own, so nothing goes stale.
#[derive(Default)]
pub(crate) struct RuleCache {
    network: usize,
    net: Option<Rules>,
    venue: HashMap<usize, Rules>,
    /// (venue position or `usize::MAX`, format position)
    pair: HashMap<(usize, usize), Rules>,
}

impl RuleCache {
    fn net(&mut self, n: &Network) -> &Rules {
        let id = n as *const Network as usize;
        if self.network != id {
            *self = RuleCache {
                network: id,
                ..Default::default()
            };
        }
        self.net.get_or_insert_with(|| {
            let mut r = Rules::default();
            r.merge_at(n.rules.as_ref(), Level::Network, || {
                format!("network {}", n.id)
            });
            r
        })
    }

    fn venue(&mut self, n: &Network, vp: usize, v: &Venue) -> Rules {
        if let Some(r) = self.venue.get(&vp) {
            return r.clone();
        }
        let mut r = self.net(n).clone();
        r.merge_at(v.rules.as_ref(), Level::Venue, || format!("venue {}", v.id));
        self.venue.insert(vp, r.clone());
        r
    }

    /// The network's rules, then the venue's, then the format's. A venue or
    /// format that isn't an element of the network's lists is merged but not
    /// remembered.
    pub(crate) fn prefix(&mut self, n: &Network, f: Option<&Format>, v: Option<&Venue>) -> Rules {
        let vp = v.map(|v| position_of(&n.venues, v));
        let fp = f.map(|f| position_of(&n.formats, f));
        let key = match (vp, fp) {
            (None, Some(Some(fp))) => Some((usize::MAX, fp)),
            (Some(Some(vp)), Some(Some(fp))) => Some((vp, fp)),
            (Some(Some(vp)), None) => return self.venue(n, vp, v.expect("a venue")),
            (None, None) => return self.net(n).clone(),
            _ => None,
        };
        if let Some(r) = key.and_then(|k| self.pair.get(&k)) {
            return r.clone();
        }
        let mut r = match (v, vp) {
            (Some(v), Some(Some(vp))) => self.venue(n, vp, v),
            (Some(v), _) => {
                let mut r = self.net(n).clone();
                r.merge_at(v.rules.as_ref(), Level::Venue, || format!("venue {}", v.id));
                r
            }
            (None, _) => self.net(n).clone(),
        };
        if let Some(f) = f {
            r.merge_at(f.fields.rules.as_ref(), Level::Format, || {
                format!("format {}", f.id)
            });
        }
        if let Some(k) = key {
            self.pair.insert(k, r.clone());
        }
        r
    }
}

/// Positions of `items` sorted by `key`, then by position.
fn sorted_by<T, K: Ord + ?Sized>(items: &[T], key: impl Fn(&T) -> &K) -> Vec<u32> {
    let mut at: Vec<u32> = (0..items.len() as u32).collect();
    at.sort_unstable_by(|&a, &b| {
        key(&items[a as usize])
            .cmp(key(&items[b as usize]))
            .then(a.cmp(&b))
    });
    at
}

/// The items of `items` whose key is `want`, in position order, found
/// through `sorted` (an index from [`sorted_by`] over `items`). The index
/// is checked as it is used, so a list changed since it was built (a
/// different length, or ids that no longer sort) is read by scanning
/// instead. A document should not be edited after its first lookup, but a
/// stale index never gives fewer matches than the list has at the ends of
/// the run it finds.
fn matching<'a: 'w, 'w, T>(
    items: &'a [T],
    sorted: &'a [u32],
    want: &'w str,
    key: impl Fn(&T) -> &str + Copy + 'w,
) -> Box<dyn Iterator<Item = &'a T> + 'w> {
    let scan = move || -> Box<dyn Iterator<Item = &'a T> + 'w> {
        Box::new(items.iter().filter(move |t| key(t) == want))
    };
    let get = |i: u32| items.get(i as usize).map(key);
    if sorted.len() != items.len() {
        return scan();
    }
    let lo = sorted.partition_point(|&i| get(i).is_some_and(|k| k < want));
    let hi = lo + sorted[lo..].partition_point(|&i| get(i) == Some(want));
    let run = &sorted[lo..hi];
    let ordered = run.windows(2).all(|w| w[0] < w[1])
        && run.iter().all(|&i| get(i) == Some(want))
        && (lo == 0 || get(sorted[lo - 1]).is_some_and(|k| k < want))
        && (hi == sorted.len() || get(sorted[hi]).is_some_and(|k| k > want));
    if !ordered {
        return scan();
    }
    Box::new(run.iter().map(move |&i| &items[i as usize]))
}

impl Network {
    pub(crate) fn screen<'a>(&'a self, id: &str) -> Option<&'a Screen> {
        let ix = self
            .lookup
            .screens
            .get_or_init(|| sorted_by(&self.screens, |s| &s.id));
        matching(&self.screens, ix, id, |s| s.id.as_str()).next()
    }

    pub(crate) fn format<'a>(&'a self, id: &str) -> Option<&'a Format> {
        let ix = self
            .lookup
            .formats
            .get_or_init(|| sorted_by(&self.formats, |f| &f.id));
        matching(&self.formats, ix, id, |f| f.id.as_str()).next()
    }

    pub(crate) fn venue<'a>(&'a self, id: &str) -> Option<&'a Venue> {
        let ix = self
            .lookup
            .venues
            .get_or_init(|| sorted_by(&self.venues, |v| &v.id));
        matching(&self.venues, ix, id, |v| v.id.as_str()).next()
    }

    /// The screens using a format, in document order.
    pub(crate) fn screens_of<'a, 'w>(
        &'a self,
        format: &'w str,
    ) -> impl Iterator<Item = &'a Screen> + 'w
    where
        'a: 'w,
    {
        let ix = self
            .lookup
            .by_format
            .get_or_init(|| sorted_by(&self.screens, |s| &s.format));
        matching(&self.screens, ix, format, |s| s.format.as_str())
    }
}

/// Lays a screen's overrides over its format's specs, where the most
/// specific level wins. Scalar and object fields replace the format's;
/// blocked regions are added (an override with the same name replaces that
/// region) and can be dropped with `remove.blocked_regions`. `overrides.rules`
/// isn't handled here: it is merged as a rules level, so it can only tighten.
pub(crate) fn apply_overrides(f: &FormatFields, o: &Overrides) -> FormatFields {
    let of = &o.fields;
    let mut out = f.clone();
    if of.pixels.is_some() {
        out.pixels = of.pixels;
    }
    if of.physical_mm.is_some() {
        out.physical_mm = of.physical_mm;
    }
    if let Some(om) = &of.media {
        let base = f.media.clone().unwrap_or_default();
        out.media = Some(merge_media(base, om));
    }
    if of.safe_area.is_some() {
        out.safe_area = of.safe_area;
    }
    if of.segments.is_some() {
        out.segments = of.segments.clone();
    }

    let drop: &[String] = o.remove.as_ref().map_or(&[], |r| &r.blocked_regions);
    let mut regions: Vec<Region> = f
        .blocked_regions
        .iter()
        .filter(|br| !drop.contains(&br.name) && !has_region(&of.blocked_regions, &br.name))
        .cloned()
        .collect();
    regions.extend(of.blocked_regions.iter().cloned());
    out.blocked_regions = regions;
    out
}

fn has_region(rs: &[Region], name: &str) -> bool {
    rs.iter().any(|r| r.name == name)
}

fn merge_media(mut base: Media, o: &Media) -> Media {
    if o.static_types.is_some() {
        base.static_types = o.static_types.clone();
    }
    if o.motion.is_some() {
        base.motion = o.motion.clone();
    }
    if o.slot_s.is_some() {
        base.slot_s = o.slot_s;
    }
    if o.fps.is_some() {
        base.fps = o.fps;
    }
    if o.codecs.is_some() {
        base.codecs = o.codecs.clone();
    }
    if o.max_bytes.is_some() {
        base.max_bytes = o.max_bytes;
    }
    if !o.colour_space.is_empty() {
        base.colour_space = o.colour_space.clone();
    }
    if o.audio.is_some() {
        base.audio = o.audio;
    }
    base
}

impl Rules {
    /// Lays one level's RuleSet over what's been inherited so far.
    /// Restrictions only tighten: categories and rule packs accumulate and
    /// can't be removed, and every numeric limit takes the stricter of the
    /// inherited and new values. Only required elements can be removed.
    #[cfg(test)]
    fn merge(&mut self, rs: &RuleSet) {
        self.merge_at(Some(rs), Level::Screen, || "a lower level".to_string());
    }

    /// `merge`, naming the level in any conflict it records. Lists are
    /// looked up through sets and maps once they are long, so merging a long
    /// list is linear, and a list the level doesn't touch isn't copied.
    pub(crate) fn merge_at(
        &mut self,
        rs: Option<&RuleSet>,
        level: Level,
        name: impl FnOnce() -> String,
    ) {
        let Some(rs) = rs else { return };
        // Counted by the size of what is merged in, and of what it is
        // merged into when that has to be looked through.
        let incoming = rs.required_elements.len()
            + rs.rule_packs.len()
            + rs.restricted_categories.len()
            + rs.remove.as_ref().map_or(0, |r| r.required_elements.len());
        let against = if incoming > 8 {
            self.required_elements.len() + self.rule_packs.len() + self.restricted_categories.len()
        } else {
            0
        };
        if !crate::budget::charge(
            crate::budget::Work::RuleMerge,
            rs as *const RuleSet as usize,
            (incoming + against + 1) as u64 * 8,
        ) {
            return;
        }
        if let Some(rm) = &rs.remove
            && !rm.required_elements.is_empty()
            && !self.required_elements.is_empty()
        {
            let roles: HashSet<&str> = rm.required_elements.iter().map(String::as_str).collect();
            Arc::make_mut(&mut self.required_elements).retain(|e| !roles.contains(e.role.as_str()));
        }
        if let Some(l) = &rs.legibility {
            let mut merged = self.legibility.clone().unwrap_or_default();
            merged.min_cap_height_mm = larger(merged.min_cap_height_mm, l.min_cap_height_mm);
            merged.min_legal_cap_height_mm =
                larger(merged.min_legal_cap_height_mm, l.min_legal_cap_height_mm);
            // A smaller ratio means bigger text.
            merged.ratio = smaller(merged.ratio, l.ratio);
            self.legibility = Some(merged);
        }
        if !rs.required_elements.is_empty() {
            self.merge_elements(rs, level, name);
        }
        self.max_words = smaller(self.max_words, rs.max_words);
        self.reading_rate_wps = smaller(self.reading_rate_wps, rs.reading_rate_wps);
        add_new(&mut self.restricted_categories, &rs.restricted_categories);
        self.motion.merge(rs.motion.as_ref());
        if let Some(b) = &rs.brightness {
            self.brightness.max_night_cd_m2 =
                smaller(self.brightness.max_night_cd_m2, b.max_night_cd_m2);
            self.brightness.max_day_cd_m2 = smaller(self.brightness.max_day_cd_m2, b.max_day_cd_m2);
        }
        if let Some(p) = &rs.playback {
            self.playback.min_dwell_s = larger(self.playback.min_dwell_s, p.min_dwell_s);
            self.playback.max_transition_s =
                smaller(self.playback.max_transition_s, p.max_transition_s);
        }
        add_new(&mut self.rule_packs, &rs.rule_packs);
        if !rs.notes.is_empty() {
            self.notes.push(rs.notes.clone());
        }
    }

    fn merge_elements(&mut self, rs: &RuleSet, level: Level, name: impl FnOnce() -> String) {
        let list = Arc::make_mut(&mut self.required_elements);
        // The same element (role and condition) restated lower down merges
        // with the inherited one, keeping the stricter limits. Found by a
        // map once there are many to find; the first of equals wins.
        let mut by_key: Option<HashMap<(String, String), usize>> = (rs.required_elements.len() > 8)
            .then(|| {
                let mut m = HashMap::new();
                for (i, x) in list.iter().enumerate() {
                    m.entry((x.role.clone(), x.when.clone())).or_insert(i);
                }
                m
            });
        let mut name = Some(name);
        for e in &rs.required_elements {
            let found = match &by_key {
                Some(m) => m.get(&(e.role.clone(), e.when.clone())).copied(),
                None => list
                    .iter()
                    .position(|x| x.role == e.role && x.when == e.when),
            };
            let Some(i) = found else {
                if let Some(m) = by_key.as_mut() {
                    m.insert((e.role.clone(), e.when.clone()), list.len());
                }
                list.push(e.clone());
                continue;
            };
            let prev = list[i].clone();
            list[i] = stricter_element(&prev, e);
            if let (Some(w), Some(p), Some(er)) = (
                list[i].within_rect_px,
                prev.within_rect_px,
                e.within_rect_px,
            ) && (w.0[2] == 0 || w.0[3] == 0)
            {
                let level_name = name.take().map_or_else(String::new, |f| f());
                self.conflicts.push(format!(
                    "{level_name} restates the {} element within {er}, which doesn't overlap the inherited {p}; no creative can comply",
                    describe_element(e),
                ));
                self.causes.push(ConflictCause {
                    level,
                    role: e.role.clone(),
                    when: e.when.clone(),
                });
            }
        }
    }
}

/// Adds the items of `add` that `list` lacks, in order, without copying
/// `list` when there is nothing to add.
fn add_new(list: &mut Arc<Vec<String>>, add: &[String]) {
    if add.is_empty() {
        return;
    }
    let new: Vec<String> = if add.len() > 8 || list.len() > 64 {
        let mut have: HashSet<&str> = list.iter().map(String::as_str).collect();
        add.iter()
            .filter(|p| have.insert(p.as_str()))
            .cloned()
            .collect()
    } else {
        let mut new: Vec<String> = Vec::new();
        for p in add {
            if !list.contains(p) && !new.contains(p) {
                new.push(p.clone());
            }
        }
        new
    };
    if !new.is_empty() {
        Arc::make_mut(list).extend(new);
    }
}

impl MotionRules {
    /// Tightens motion limits: once disallowed, motion stays disallowed,
    /// and flash and hold limits take the stricter value.
    fn merge(&mut self, o: Option<&MotionRules>) {
        let Some(o) = o else { return };
        if o.allowed.is_some() && (self.allowed.is_none() || self.allowed == Some(true)) {
            self.allowed = o.allowed;
        }
        if o.no_flashing.is_some()
            && (self.no_flashing.is_none() || self.no_flashing == Some(false))
        {
            self.no_flashing = o.no_flashing;
        }
        self.max_flash_hz = smaller(self.max_flash_hz, o.max_flash_hz);
        self.min_frame_hold_s = larger(self.min_frame_hold_s, o.min_frame_hold_s);
    }
}

fn stricter_element(a: &RequiredElement, b: &RequiredElement) -> RequiredElement {
    let mut out = a.clone();
    out.min_cap_height_mm = larger(a.min_cap_height_mm, b.min_cap_height_mm);
    out.min_cap_height_px = larger(a.min_cap_height_px, b.min_cap_height_px);
    match (a.within_rect_px, b.within_rect_px) {
        (None, w) => out.within_rect_px = w,
        (Some(x), Some(y)) => out.within_rect_px = Some(intersect(x, y)),
        (Some(_), None) => {}
    }
    if !b.notes.is_empty() {
        out.notes = b.notes.clone();
    }
    out
}

fn describe_element(e: &RequiredElement) -> String {
    if e.when.is_empty() {
        e.role.clone()
    } else {
        format!("{} ({})", e.role, e.when)
    }
}

/// The overlap of two rectangles; zero-sized if none.
fn intersect(a: Rect, b: Rect) -> Rect {
    let (a, b) = (a.0, b.0);
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = (
        a[0].wrapping_add(a[2]).min(b[0].wrapping_add(b[2])),
        a[1].wrapping_add(a[3]).min(b[1].wrapping_add(b[3])),
    );
    Rect([
        x0,
        y0,
        x1.wrapping_sub(x0).max(0),
        y1.wrapping_sub(y0).max(0),
    ])
}

/// Picks the stricter (larger) of two optional limits.
fn larger<T: PartialOrd + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (None, _) => b,
        (Some(x), Some(y)) if y > x => b,
        _ => a,
    }
}

/// Picks the stricter (smaller) of two optional limits.
fn smaller<T: PartialOrd + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (None, _) => b,
        (Some(x), Some(y)) if y < x => b,
        _ => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::CODE_RULE_CONFLICT;
    use crate::document::check_document;
    use crate::schema::{Kind, SchemaOptions, validate_schema};
    use crate::testutil::harbourline;

    fn resolve_one<'a>(d: &'a Document, r: &str) -> Requirements<'a> {
        let mut reqs = d.resolve(&parse_ref(r).unwrap()).unwrap();
        assert_eq!(reqs.len(), 1, "{r} should resolve to one requirement");
        reqs.remove(0)
    }

    #[test]
    fn parse_ref_forms() {
        let r = parse_ref("harbourline.example/retail/portrait-totem-75").unwrap();
        assert_eq!(r.format, "portrait-totem-75");
        assert_eq!(r.screen, "");
        let r = parse_ref("harbourline.example/retail/screen/HC-L2-014").unwrap();
        assert_eq!(r.screen, "HC-L2-014");
        assert_eq!(r.format, "");
        assert_eq!(r.to_string(), "harbourline.example/retail/screen/HC-L2-014");
        for bad in ["", "a/b", "a/b/c/d", "a/b/screen/", "a//c"] {
            assert!(parse_ref(bad).is_err(), "parse_ref({bad:?}) should fail");
        }
        assert_eq!(
            parse_ref("a/b").unwrap_err().to_string(),
            "\"a/b\" is not a format or screen reference"
        );
    }

    #[test]
    fn format_ref_resolves_every_screen() {
        let d = harbourline();
        let reqs = d
            .resolve(&parse_ref("harbourline.example/retail/portrait-totem-75").unwrap())
            .unwrap();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].screen_id(), "HC-L1-001");
        assert_eq!(reqs[1].screen_id(), "HC-L1-002");
    }

    // HC-L1-002 overrides blocked_regions with a column. Overrides add to the
    // format's regions, so the network logo still applies.
    #[test]
    fn override_blocked_regions_add() {
        let d = harbourline();
        let r = resolve_one(&d, "harbourline.example/retail/screen/HC-L1-002");
        let names: Vec<&str> = r.blocked_regions.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["network-logo", "column"]);
        assert_eq!(r.venue.map(|v| v.id.as_str()), Some("harbour-central"));
    }

    #[test]
    fn override_remove_and_replace_region() {
        let region = |name: &str, x| Region {
            name: name.into(),
            rect_px: Rect([x, 0, 10, 10]),
            reason: String::new(),
        };
        let f = FormatFields {
            blocked_regions: vec![region("logo", 0), region("clock", 20)],
            ..Default::default()
        };
        let o = Overrides {
            fields: FormatFields {
                blocked_regions: vec![Region {
                    rect_px: Rect([5, 5, 10, 10]),
                    ..region("logo", 0)
                }],
                ..Default::default()
            },
            remove: Some(OverridesRemove {
                blocked_regions: vec!["clock".into()],
            }),
        };
        let got = apply_overrides(&f, &o).blocked_regions;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "logo");
        assert_eq!(got[0].rect_px.0[0], 5);
    }

    // The schema allows empty arrays, and in Go an override of [] clears the
    // format's value rather than inheriting it.
    #[test]
    fn empty_override_arrays_clear_the_format_value() {
        let f = FormatFields {
            media: Some(Media {
                static_types: Some(vec!["a".into()]),
                codecs: Some(vec!["c".into()]),
                motion: Some(vec!["m".into()]),
                ..Default::default()
            }),
            segments: Some(vec![Segment {
                id: "x".into(),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let o: Overrides =
            serde_json::from_str(r#"{"media":{"static":[],"codecs":[]},"segments":[]}"#).unwrap();
        let out = apply_overrides(&f, &o);
        let m = out.media.unwrap();
        assert_eq!(m.static_types, Some(vec![]));
        assert_eq!(m.codecs, Some(vec![]));
        assert_eq!(m.motion, Some(vec!["m".into()]));
        assert_eq!(out.segments, Some(vec![]));
        // An absent field still inherits.
        let o: Overrides = serde_json::from_str(r#"{"media":{}}"#).unwrap();
        let out = apply_overrides(&f, &o);
        assert_eq!(out.media.unwrap().static_types, Some(vec!["a".into()]));
        assert_eq!(out.segments.unwrap().len(), 1);
    }

    fn png_format(id: &str, pixels: [i64; 2], rules: Option<RuleSet>) -> Format {
        Format {
            id: id.into(),
            fields: FormatFields {
                pixels: Some(pixels),
                media: Some(Media {
                    static_types: Some(vec!["image/png".into()]),
                    ..Default::default()
                }),
                rules,
                ..Default::default()
            },
        }
    }

    fn motion(allowed: Option<bool>, hz: f64) -> Option<MotionRules> {
        Some(MotionRules {
            allowed,
            max_flash_hz: Some(hz),
            ..Default::default()
        })
    }

    // Specs follow precedence (screen > format > venue > network); rules only
    // tighten, so a network-wide format can't loosen a venue's restrictions.
    #[test]
    fn restrictions_only_tighten() {
        let screen =
            |id: &str, venue: &str, rules: Option<RuleSet>, overrides: Option<Overrides>| Screen {
                id: id.into(),
                format: "f".into(),
                venue: venue.into(),
                rules: rules.map(Box::new),
                overrides: overrides.map(Box::new),
                ..Default::default()
            };
        let d = Document {
            publisher: "p.example".into(),
            networks: vec![Network {
                id: "n".into(),
                rules: Some(RuleSet {
                    max_words: Some(10),
                    restricted_categories: vec!["IAB7".into()],
                    rule_packs: vec!["au-nsw-transport-corridor@2017-11".into()],
                    notes: "network".into(),
                    ..Default::default()
                }),
                venues: vec![Venue {
                    id: "v".into(),
                    rules: Some(RuleSet {
                        max_words: Some(6),
                        restricted_categories: vec!["IAB8-5".into()],
                        motion: motion(Some(false), 2.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                formats: vec![png_format(
                    "f",
                    [100, 100],
                    Some(RuleSet {
                        max_words: Some(8),
                        motion: motion(Some(true), 3.0),
                        ..Default::default()
                    }),
                )],
                screens: vec![
                    screen("s1", "v", None, None),
                    screen(
                        "s2",
                        "v",
                        Some(RuleSet {
                            max_words: Some(5),
                            motion: motion(None, 1.0),
                            ..Default::default()
                        }),
                        None,
                    ),
                    screen(
                        "s3",
                        "",
                        None,
                        Some(Overrides {
                            fields: FormatFields {
                                pixels: Some([50, 50]),
                                rules: Some(RuleSet {
                                    max_words: Some(20),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                            remove: None,
                        }),
                    ),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let s1 = resolve_one(&d, "p.example/n/screen/s1");
        assert_eq!(s1.rules.max_words, Some(6), "the venue's stricter 6");
        assert_eq!(s1.rules.motion.allowed, Some(false));
        assert_eq!(s1.rules.motion.max_flash_hz, Some(2.0));
        assert_eq!(s1.rules.restricted_categories.len(), 2);
        assert_eq!(s1.rules.rule_packs.len(), 1);
        assert_eq!(s1.rules.notes, ["network"]);
        let s2 = resolve_one(&d, "p.example/n/screen/s2");
        assert_eq!(s2.rules.max_words, Some(5));
        assert_eq!(s2.rules.motion.max_flash_hz, Some(1.0));
        // Overrides still replace specs, but overrides.rules only tightens.
        let s3 = resolve_one(&d, "p.example/n/screen/s3");
        assert_eq!(s3.pixels, [50, 50]);
        assert_eq!(s3.rules.max_words, Some(8));
    }

    // remove can't drop restricted categories or rule packs; the schema says so.
    #[test]
    fn schema_rejects_removing_restrictions() {
        for field in [
            r#""restricted_categories":["IAB7"]"#,
            r#""rule_packs":["au-nsw-transport-corridor@2017-11"]"#,
        ] {
            let doc = format!(
                r#"{{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-10-20T09:00:00Z","networks":[{{"id":"n","name":"N",
		  "formats":[{{"id":"f","pixels":[1,1],"media":{{"static":["image/png"]}}}}],
		  "screens":[{{"id":"s","format":"f","rules":{{"remove":{{{field}}}}}}}]}}]}}"#
            );
            let errs =
                validate_schema(Kind::Screens, doc.as_bytes(), &SchemaOptions::default()).unwrap();
            assert!(
                !errs.is_empty(),
                "remove {{{field}}} should be a schema error"
            );
        }
    }

    #[test]
    fn legibility_fields_tighten() {
        let mut r = Rules::default();
        r.merge(&RuleSet {
            legibility: Some(Legibility {
                ratio: Some(150.0),
                min_cap_height_mm: Some(40.0),
                ..Default::default()
            }),
            ..Default::default()
        });
        r.merge(&RuleSet {
            legibility: Some(Legibility {
                ratio: Some(200.0),
                min_cap_height_mm: Some(30.0),
                min_legal_cap_height_mm: Some(14.0),
            }),
            ..Default::default()
        });
        let l = r.legibility.unwrap();
        assert_eq!(l.ratio, Some(150.0));
        assert_eq!(l.min_cap_height_mm, Some(40.0));
        assert_eq!(l.min_legal_cap_height_mm, Some(14.0));
    }

    #[test]
    fn required_element_restated_tightens() {
        let legal = |mm, rect| RequiredElement {
            role: "legal".into(),
            when: "offer".into(),
            min_cap_height_mm: Some(mm),
            within_rect_px: Some(Rect(rect)),
            ..Default::default()
        };
        let mut r = Rules::default();
        r.merge(&RuleSet {
            required_elements: vec![
                legal(16.0, [0, 1500, 1080, 420]),
                RequiredElement {
                    role: "logo".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        r.merge(&RuleSet {
            required_elements: vec![legal(12.0, [0, 1700, 1080, 220])],
            ..Default::default()
        });
        assert_eq!(r.required_elements.len(), 2);
        let l = &r.required_elements[0];
        assert_eq!(l.min_cap_height_mm, Some(16.0));
        assert_eq!(l.within_rect_px, Some(Rect([0, 1700, 1080, 220])));
        r.merge(&RuleSet {
            remove: Some(RuleRemove {
                required_elements: vec!["legal".into()],
            }),
            ..Default::default()
        });
        assert_eq!(r.required_elements.len(), 1);
        assert_eq!(r.required_elements[0].role, "logo");
    }

    // Position limits that don't overlap can never be met: the document check
    // reports them. (The creative-report half of the Go test belongs with
    // `validate`.)
    #[test]
    fn non_overlapping_position_limits_conflict() {
        let mut d = harbourline();
        let n = &mut d.networks[0];
        n.rules.as_mut().unwrap().required_elements[0].within_rect_px =
            Some(Rect([0, 1600, 1080, 320]));
        let legal = |rect| RuleSet {
            required_elements: vec![RequiredElement {
                role: "legal".into(),
                when: "offer".into(),
                within_rect_px: Some(Rect(rect)),
                ..Default::default()
            }],
            ..Default::default()
        };
        n.venues[0].rules = Some(legal([0, 0, 1080, 300]));

        let checks = check_document(&d);
        assert_eq!(checks.len(), 1, "{checks:?}");
        assert_eq!(checks[0].code, CODE_RULE_CONFLICT);
        // Caused by the venue's rules, so it is the venue's: once, with no screens.
        assert!(checks[0].screens.is_empty());
        assert_eq!(
            (
                checks[0].network.as_str(),
                checks[0].venue.as_str(),
                checks[0].format.as_str()
            ),
            ("retail", "harbour-central", "")
        );
        assert_eq!(
            checks[0].message,
            "venue harbour-central restates the legal (offer) element within [0 0 1080 300], which doesn't overlap the inherited [0 1600 1080 320]; no creative can comply"
        );

        // Overlapping limits narrow to the overlap without a conflict.
        d.networks[0].venues[0].rules = Some(legal([0, 1500, 1080, 200]));
        assert!(check_document(&d).is_empty());
    }

    #[test]
    fn clean_document_has_no_document_errors() {
        assert!(check_document(&harbourline()).is_empty());
    }

    #[test]
    fn broken_references_are_reported_per_screen() {
        let mut d = harbourline();
        d.networks[0].screens[0].format = "gone".into();
        d.networks[0].screens[1].format = "gone".into();
        let checks = check_document(&d);
        assert_eq!(checks.len(), 2, "{checks:?}");
        assert_eq!(checks[0].code, crate::check::CODE_REFERENCE);
        assert_eq!(
            checks[0].message,
            format!(
                "screen \"{}\" uses format \"gone\", which network \"retail\" doesn't define",
                d.networks[0].screens[0].id
            )
        );
        assert_eq!(checks[0].screens.len(), 1);
    }

    // Ids are case-sensitive and matched exactly.
    #[test]
    fn ids_are_case_sensitive() {
        let r = parse_ref("harbourline.example/retail/screen/hc-l1-001").unwrap();
        assert!(harbourline().resolve(&r).is_err());
    }

    #[test]
    fn resolve_errors() {
        let d = harbourline();
        for r in [
            "other.example/retail/portrait-totem-75",
            "harbourline.example/nope/portrait-totem-75",
            "harbourline.example/retail/nope",
            "harbourline.example/retail/screen/NOPE",
        ] {
            let e = d.resolve(&parse_ref(r).unwrap()).unwrap_err();
            assert!(matches!(e, ResolveError::Other(_)), "{r}");
        }
        let e = d
            .resolve(&parse_ref("harbourline.example/retail/nope").unwrap())
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "format \"nope\" not found in network \"retail\""
        );
    }

    #[test]
    fn intersect_wraps_instead_of_panicking() {
        let big = Rect([i64::MAX, i64::MIN, i64::MAX, i64::MAX]);
        let _ = intersect(big, big);
        let r = intersect(Rect([0, 0, 10, 10]), Rect([5, 5, 10, 10]));
        assert_eq!(r.0, [5, 5, 5, 5]);
    }

    /// Lookups by id are indexed: tens of thousands of venues, formats and
    /// screens resolve at once, and of repeated ids the first still wins.
    #[test]
    fn lookups_scale_and_the_first_repeated_id_wins() {
        const N: usize = 50_000;
        let format = |id: String, w: i64| Format {
            id,
            fields: FormatFields {
                pixels: Some([w, 10]),
                ..Default::default()
            },
        };
        let mut formats: Vec<Format> = (0..N).map(|i| format(format!("f-{i}"), 1)).collect();
        formats.push(format("f-7".into(), 2));
        let mut venues: Vec<Venue> = (0..N)
            .map(|i| Venue {
                id: format!("v-{i}"),
                name: "first".into(),
                ..Default::default()
            })
            .collect();
        venues.push(Venue {
            id: "v-7".into(),
            name: "second".into(),
            ..Default::default()
        });
        let mut screens: Vec<Screen> = (0..N)
            .map(|i| Screen {
                id: format!("s-{}", i % (N - 1)),
                format: format!("f-{}", N - 1 - i),
                venue: format!("v-{}", N - 1 - i),
                ..Default::default()
            })
            .collect();
        screens.push(Screen {
            id: "s-0".into(),
            format: "f-3".into(),
            ..Default::default()
        });
        let d = Document {
            publisher: "p.example".into(),
            networks: vec![Network {
                id: "n".into(),
                formats,
                venues,
                screens,
                ..Default::default()
            }],
            ..Default::default()
        };
        let n = &d.networks[0];
        assert_eq!(n.format("f-7").unwrap().fields.pixels, Some([1, 10]));
        assert_eq!(n.venue("v-7").unwrap().name, "first");
        assert_eq!(n.format("nope").map(|f| &f.id), None);
        assert_eq!(n.screen("s-0").unwrap().format, format!("f-{}", N - 1));
        assert_eq!(n.screens_of("f-3").count(), 2);
        assert_eq!(n.screens_of("f-3").next().unwrap().id, "s-49996");
        let start = std::time::Instant::now();
        assert!(check_document(&d).is_empty());
        assert!(start.elapsed().as_secs() < 30, "{:?}", start.elapsed());
        // A clone, or an edit that changes a list's length, is still right.
        let mut c = n.clone();
        c.venues.truncate(5);
        assert!(c.venue("v-7").is_none());
        assert_eq!(&c, &c.clone());
        let mut e = n.clone();
        assert!(e.venue("v-1").is_some());
        e.venues.truncate(1);
        assert!(e.venue("v-1").is_none());
    }
}
