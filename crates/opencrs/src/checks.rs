//! The creative checks: every check for one creative file against one
//! screen's requirements. Port of `crs/check.go` (the checker, the facts and
//! the helpers); the types and codes are in `check.rs`, and the rule-pack
//! checks are in `packcheck.rs`.

use crate::check::*;
use crate::legibility::{class_of, count_words, num};
use crate::model::{Campaign, CreativeFile, Element, Rect};
use crate::probe::Probe;
use crate::resolve::Requirements;
use crate::rulepack::RulePacks;

/// The measurable properties of a creative file, taken from the file where it
/// was available and from the manifest otherwise.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facts {
    pub media_type: String,
    pub media_src: String,
    pub pixels: Option<[i64; 2]>,
    pub pixels_src: String,
    pub bytes: Option<i64>,
    pub bytes_src: String,
    pub duration: Option<f64>,
    pub duration_src: String,
    pub sha256: String,
}

/// Gathers the facts for a file, preferring the probe, and reports where the
/// file and the manifest disagree.
pub(crate) fn gather_facts(f: &CreativeFile, p: Option<&Probe>) -> (Facts, Vec<Check>) {
    let mut x = Facts {
        media_type: f.media_type.clone(),
        media_src: "manifest".into(),
        pixels: f.pixels,
        pixels_src: "manifest".into(),
        bytes: f.bytes,
        bytes_src: "manifest".into(),
        duration: f.duration_s,
        duration_src: "manifest".into(),
        sha256: f.sha256.clone(),
    };
    let Some(p) = p else {
        return (x, Vec::new());
    };
    let mut cs = Vec::new();
    let mut mismatch = |sev: Severity, msg: String| {
        let mut c = Check::new(CODE_MANIFEST_MISMATCH, sev, msg);
        c.source = "file".into();
        cs.push(c);
    };
    if !f.sha256.is_empty() && !f.sha256.eq_ignore_ascii_case(&p.sha256) {
        mismatch(
            Severity::Error,
            format!(
                "File's SHA-256 is {}, but the manifest declares {}; this isn't the file that was declared",
                p.sha256, f.sha256
            ),
        );
    }
    x.sha256 = p.sha256.clone();
    if !p.media_type.is_empty() {
        if p.media_type != f.media_type {
            mismatch(
                Severity::Warning,
                format!(
                    "File is {}, but the manifest declares {}",
                    p.media_type, f.media_type
                ),
            );
        }
        x.media_type = p.media_type.clone();
        x.media_src = "file".into();
    }
    if let Some(pp) = p.pixels {
        if let Some(fp) = f.pixels
            && fp != pp
        {
            mismatch(
                Severity::Warning,
                format!(
                    "File is {}, but the manifest declares {}",
                    dims(pp),
                    dims(fp)
                ),
            );
        }
        x.pixels = Some(pp);
        x.pixels_src = "file".into();
    }
    if let Some(fb) = f.bytes
        && fb != p.bytes
    {
        mismatch(
            Severity::Warning,
            format!(
                "File is {} bytes, but the manifest declares {}",
                p.bytes, fb
            ),
        );
    }
    x.bytes = Some(p.bytes);
    x.bytes_src = "file".into();
    if let Some(pd) = p.duration_s {
        if let Some(fd) = f.duration_s
            && (fd - pd).abs() > 0.05
        {
            mismatch(
                Severity::Warning,
                format!(
                    "File runs {} s, but the manifest declares {} s",
                    num(pd),
                    num(fd)
                ),
            );
        }
        x.duration = Some(pd);
        x.duration_src = "file".into();
    }
    (x, cs)
}

/// Runs every check for one creative file against one screen's
/// requirements.
pub(crate) struct Checker<'a> {
    pub req: &'a Requirements<'a>,
    pub file: &'a CreativeFile,
    pub campaign: Option<&'a Campaign>,
    pub facts: Facts,
    pub margin: f64,
    /// The packs this validator knows, by name@version.
    pub rule_packs: &'a RulePacks,
    pub out: Vec<Check>,
}

fn contains(list: &Option<Vec<String>>, s: &str) -> bool {
    list.as_deref().unwrap_or_default().iter().any(|t| t == s)
}

impl<'a> Checker<'a> {
    pub(crate) fn add(&mut self, c: Check) {
        self.out.push(c);
    }

    pub(crate) fn report(&mut self, code: &str, sev: Severity, msg: impl Into<String>) {
        self.out.push(Check::new(code, sev, msg));
    }

    /// Whether the file is motion: a declared motion type, or a video type the
    /// format doesn't list as static.
    pub(crate) fn is_motion(&self) -> bool {
        let mt = self.facts.media_type.as_str();
        contains(&self.req.media.motion, mt)
            || (!contains(&self.req.media.static_types, mt) && mt.starts_with("video/"))
    }

    pub(crate) fn run(mut self) -> Vec<Check> {
        self.pixels();
        self.media_type();
        self.file_size();
        self.duration();
        self.safe_area();
        self.blocked_regions();
        self.legibility();
        self.max_words();
        self.required_elements();
        self.category();
        self.motion();
        self.rule_packs_check();
        self.rule_conflicts();
        self.out
    }

    fn pixels(&mut self) {
        let want = self.req.pixels;
        match self.facts.pixels {
            None => self.report(
                CODE_PIXELS,
                Severity::NotChecked,
                "Canvas size unknown: no file to read and no pixels in the manifest",
            ),
            Some(got) if got != want => {
                let mut c = Check::new(
                    CODE_PIXELS,
                    Severity::Error,
                    format!("Canvas is {}; the format needs {}", dims(got), dims(want)),
                );
                c.source = self.facts.pixels_src.clone();
                self.add(c);
            }
            _ => {}
        }
    }

    fn media_type(&mut self) {
        let m = &self.req.media;
        let mt = self.facts.media_type.as_str();
        if contains(&m.static_types, mt) || contains(&m.motion, mt) {
            return;
        }
        let accepted: Vec<&str> = m
            .static_types
            .iter()
            .chain(m.motion.iter())
            .flatten()
            .map(String::as_str)
            .collect();
        let mut c = Check::new(
            CODE_MEDIA_TYPE,
            Severity::Error,
            format!("{mt} isn't accepted; use {}", accepted.join(", ")),
        );
        c.source = self.facts.media_src.clone();
        self.add(c);
    }

    fn file_size(&mut self) {
        let Some(max_b) = self.req.media.max_bytes else {
            return;
        };
        match self.facts.bytes {
            None => self.report(
                CODE_FILE_SIZE,
                Severity::NotChecked,
                "File size unknown: no file to read and no bytes in the manifest",
            ),
            Some(b) if b > max_b => {
                let mut c = Check::new(
                    CODE_FILE_SIZE,
                    Severity::Error,
                    format!("File is {}; the maximum is {}", size(b), size(max_b)),
                );
                c.source = self.facts.bytes_src.clone();
                self.add(c);
            }
            _ => {}
        }
    }

    fn duration(&mut self) {
        let Some(slot) = self.req.media.slot_s else {
            return;
        };
        if !self.is_motion() {
            return;
        }
        let Some(d) = self.facts.duration else {
            self.report(
                CODE_DURATION,
                Severity::NotChecked,
                "Duration unknown: couldn't read it from the file and the manifest has no duration_s",
            );
            return;
        };
        // Allow one frame either way.
        let mut tol = 0.04;
        if let Some(fps) = self.req.media.fps
            && fps > 0.0
        {
            tol = 1.0 / fps;
        }
        if (d - slot).abs() > tol + 1e-9 {
            let mut c = Check::new(
                CODE_DURATION,
                Severity::Error,
                format!("Runs {} s; the slot is {} s", num(d), num(slot)),
            );
            c.source = self.facts.duration_src.clone();
            self.add(c);
        }
    }

    /// The elements that have a position and matter for layout: everything
    /// except backgrounds and decoration.
    fn placed(&self) -> Vec<&'a Element> {
        let file: &'a CreativeFile = self.file;
        file.elements
            .iter()
            .filter(|e| e.rect_px.is_some() && e.role != "background" && e.role != "decoration")
            .collect()
    }

    fn safe_area(&mut self) {
        let Some(sa) = self.req.safe_area else {
            return;
        };
        let els = self.placed();
        if els.is_empty() {
            self.report(
                CODE_SAFE_AREA,
                Severity::NotChecked,
                "No element positions in the manifest, so the safe area can't be checked",
            );
            return;
        }
        let [w, h] = self.req.pixels;
        let safe = Rect([
            sa.left,
            sa.top,
            w.wrapping_sub(sa.left).wrapping_sub(sa.right),
            h.wrapping_sub(sa.top).wrapping_sub(sa.bottom),
        ]);
        for e in els {
            let rect = e.rect_px.expect("placed elements have a rect");
            if within(rect, safe) {
                continue;
            }
            let (r, s) = (rect.0, safe.0);
            let mut sides = Vec::new();
            if r[1] < s[1] {
                sides.push(format!("top {} px", sa.top));
            }
            if r[0].wrapping_add(r[2]) > s[0].wrapping_add(s[2]) {
                sides.push(format!("right {} px", sa.right));
            }
            if r[1].wrapping_add(r[3]) > s[1].wrapping_add(s[3]) {
                sides.push(format!("bottom {} px", sa.bottom));
            }
            if r[0] < s[0] {
                sides.push(format!("left {} px", sa.left));
            }
            let mut c = Check::new(
                CODE_SAFE_AREA,
                Severity::Error,
                format!(
                    "{} extends into the safe-area margin ({})",
                    title(&e.role),
                    sides.join(", ")
                ),
            );
            c.element = e.role.clone();
            c.rect_px = Some(rect);
            self.add(c);
        }
    }

    fn blocked_regions(&mut self) {
        let req: &'a Requirements<'a> = self.req;
        if req.blocked_regions.is_empty() {
            return;
        }
        let els = self.placed();
        if els.is_empty() {
            self.report(
                CODE_BLOCKED_REGION,
                Severity::NotChecked,
                "No element positions in the manifest, so blocked regions can't be checked",
            );
            return;
        }
        for e in els {
            let rect = e.rect_px.expect("placed elements have a rect");
            for br in &req.blocked_regions {
                if overlaps(rect, br.rect_px) {
                    let mut c = Check::new(
                        CODE_BLOCKED_REGION,
                        Severity::Error,
                        format!("{} overlaps the {} area", title(&e.role), br.name),
                    );
                    c.element = e.role.clone();
                    c.rect_px = Some(br.rect_px);
                    self.add(c);
                }
            }
        }
    }

    fn legibility(&mut self) {
        if self.req.rules.legibility.is_none() {
            return;
        }
        let file: &'a CreativeFile = self.file;
        let texts: Vec<&Element> = file
            .elements
            .iter()
            .filter(|e| class_of(&e.role).is_some())
            .collect();
        if texts.is_empty() {
            self.report(
                CODE_LEGIBILITY,
                Severity::NotChecked,
                "No text elements with cap heights in the manifest, so legibility can't be checked",
            );
            return;
        }
        for e in texts {
            let class = class_of(&e.role).expect("filtered to text");
            let (min, applies, missing) = self.req.min_cap_height(class);
            if !applies {
                continue;
            }
            let mut c = Check::new(CODE_LEGIBILITY, Severity::NotChecked, "");
            c.element = e.role.clone();
            let Some(got) = e.cap_height_px else {
                c.message = format!("{} has no cap_height_px in the manifest", title(&e.role));
                self.add(c);
                continue;
            };
            if !missing.is_empty() {
                // Below a known floor fails whatever the rest of the rule says.
                if min.px > 0 && got < min.px as f64 {
                    c.severity = Severity::Error;
                    c.rect_px = e.rect_px;
                    c.message = format!(
                        "{} text is {} px; {} px minimum {}, and more may be needed ({})",
                        title(&e.role),
                        num(got),
                        min.px,
                        min.basis,
                        missing
                    );
                } else {
                    c.message = format!("{} size can't be checked: {}", title(&e.role), missing);
                }
                self.add(c);
                continue;
            }
            let msg = format!(
                "{} text is {} px; {} px minimum {}",
                title(&e.role),
                num(got),
                min.px,
                min.basis
            );
            if got < min.px as f64 {
                c.severity = Severity::Error;
                c.rect_px = e.rect_px;
                c.message = msg;
                self.add(c);
            } else if got < min.px as f64 * self.margin {
                c.severity = Severity::Warning;
                c.rect_px = e.rect_px;
                c.message = msg + ". Passes with little margin";
                self.add(c);
            }
        }
    }

    fn max_words(&mut self) {
        let Some((limit, basis)) = self.req.word_limit() else {
            return;
        };
        let parts: Vec<&str> = self
            .file
            .elements
            .iter()
            .filter(|e| (e.role == "headline" || e.role == "subhead") && !e.text.is_empty())
            .map(|e| e.text.as_str())
            .collect();
        if parts.is_empty() {
            self.report(
                CODE_MAX_WORDS,
                Severity::NotChecked,
                "No headline text in the manifest, so the word limit can't be checked",
            );
            return;
        }
        let n = count_words(&parts.join(" ")) as i64;
        if n > limit {
            self.report(
                CODE_MAX_WORDS,
                Severity::Error,
                format!("Main message is {n} words; the limit is {limit} ({basis})"),
            );
        }
    }

    /// The creative's declared tags; `known` is false when neither the file
    /// nor the campaign declares any.
    fn tags(&self) -> (Vec<&str>, bool) {
        let mut tags = Vec::new();
        let mut known = false;
        if let Some(t) = &self.file.tags {
            tags.extend(t.iter().map(String::as_str));
            known = true;
        }
        if let Some(t) = self.campaign.and_then(|c| c.tags.as_ref()) {
            tags.extend(t.iter().map(String::as_str));
            known = true;
        }
        (tags, known)
    }

    fn required_elements(&mut self) {
        let req: &'a Requirements<'a> = self.req;
        let file: &'a CreativeFile = self.file;
        for want in &req.rules.required_elements {
            let mut what = title(&want.role);
            if !want.when.is_empty() {
                what += &format!(" (on {} creative)", want.when);
            }
            let mut known = true;
            if !want.when.is_empty() {
                let (tags, ok) = self.tags();
                known = ok;
                if known && !tags.contains(&want.when.as_str()) {
                    continue;
                }
            }
            let Some(found) = file.elements.iter().find(|e| e.role == want.role) else {
                if file.elements.is_empty() {
                    self.report(
                        CODE_REQUIRED_ELEMENT,
                        Severity::NotChecked,
                        format!("{what} is required, but the manifest lists no elements to check"),
                    );
                } else if !known {
                    self.report(
                        CODE_REQUIRED_ELEMENT,
                        Severity::NotChecked,
                        format!(
                            "{} is required if the creative is {}; declare tags in the manifest to check",
                            title(&want.role),
                            want.when
                        ),
                    );
                } else {
                    let mut c = Check::new(
                        CODE_REQUIRED_ELEMENT,
                        Severity::Error,
                        format!("{what} is required but missing"),
                    );
                    c.element = want.role.clone();
                    self.add(c);
                }
                continue;
            };
            // A zero-area limit is a rule conflict, reported once by
            // rule_conflicts.
            if let Some(w) = want.within_rect_px
                && w.0[2] > 0
                && w.0[3] > 0
                && let Some(fr) = found.rect_px
                && !within(fr, w)
            {
                let mut c = Check::new(
                    CODE_REQUIRED_ELEMENT,
                    Severity::Error,
                    format!("{what} must sit within {w}"),
                );
                c.element = want.role.clone();
                c.rect_px = Some(fr);
                self.add(c);
            }
            let (min_px, basis) = if let Some(px) = want.min_cap_height_px {
                (px, String::new())
            } else if let Some(mm) = want.min_cap_height_mm {
                match req.mm_to_px(mm) {
                    Some(px) => (px, format!(" ({} mm)", num(mm))),
                    None => {
                        let mut c = Check::new(
                            CODE_REQUIRED_ELEMENT,
                            Severity::NotChecked,
                            format!(
                                "{what} size can't be checked: no physical_mm to convert millimetres to pixels"
                            ),
                        );
                        c.element = want.role.clone();
                        self.add(c);
                        continue;
                    }
                }
            } else {
                continue;
            };
            let Some(cap) = found.cap_height_px else {
                let mut c = Check::new(
                    CODE_REQUIRED_ELEMENT,
                    Severity::NotChecked,
                    format!("{what} has no cap_height_px in the manifest"),
                );
                c.element = want.role.clone();
                self.add(c);
                continue;
            };
            if cap < min_px as f64 {
                let mut c = Check::new(
                    CODE_REQUIRED_ELEMENT,
                    Severity::Error,
                    format!(
                        "{what} is {} px; it must be at least {min_px} px{basis}",
                        num(cap)
                    ),
                );
                c.element = want.role.clone();
                c.rect_px = found.rect_px;
                self.add(c);
            }
        }
    }

    fn category(&mut self) {
        let req: &'a Requirements<'a> = self.req;
        let restricted = &req.rules.restricted_categories;
        if restricted.is_empty() {
            return;
        }
        let cat = self.campaign.map_or("", |c| c.category.as_str());
        if cat.is_empty() {
            self.report(
                CODE_CATEGORY,
                Severity::NotChecked,
                format!(
                    "No campaign category in the manifest; this network restricts {}",
                    restricted.join(", ")
                ),
            );
            return;
        }
        for r in restricted {
            if category_within(cat, r) {
                self.report(
                    CODE_CATEGORY,
                    Severity::Error,
                    format!("Category {cat} isn't allowed on this screen (restricted: {r})"),
                );
                return;
            }
        }
    }

    fn motion(&mut self) {
        // A rule pack that prohibits motion is reported once, as
        // CRS-MOTION-PROHIBITED; see motion_prohibited.
        if !self.is_motion() || self.pack_prohibits_motion() {
            return;
        }
        let req: &'a Requirements<'a> = self.req;
        let m = &req.rules.motion;
        if let Some(p) = req.screen.and_then(|s| s.placement.as_ref())
            && p.motion_allowed == Some(false)
        {
            self.report(
                CODE_MOTION,
                Severity::Error,
                "This screen doesn't allow motion; supply a static file",
            );
            return;
        }
        if m.allowed == Some(false) {
            self.report(
                CODE_MOTION,
                Severity::Error,
                "Motion isn't allowed here; supply a static file",
            );
            return;
        }
        // no_flashing (from the rules or a rule pack) is a 0 Hz limit, the
        // strictest there is.
        let mut limit = m.max_flash_hz;
        if m.no_flashing == Some(true) || self.pack_no_flashing() {
            limit = Some(0.0);
        }
        let Some(limit) = limit else {
            return;
        };
        let Some(hz) = self.file.max_flash_hz else {
            self.report(
                CODE_MOTION,
                Severity::NotChecked,
                format!(
                    "Flash rate limit is {} Hz; checking it needs frame analysis or max_flash_hz in the manifest",
                    num(limit)
                ),
            );
            return;
        };
        if hz > limit {
            let mut c = Check::new(
                CODE_MOTION,
                Severity::Error,
                format!("Flashes at {} Hz; the limit is {} Hz", num(hz), num(limit)),
            );
            c.source = "manifest".into();
            self.add(c);
        }
    }

    /// Repeats document conflicts in creative reports, so a creative that
    /// can't comply says why.
    fn rule_conflicts(&mut self) {
        let req: &'a Requirements<'a> = self.req;
        for msg in &req.rules.conflicts {
            self.report(CODE_RULE_CONFLICT, Severity::Error, msg.clone());
        }
    }
}

/// Reports whether IAB category `cat` is restricted by `r`: either the same
/// code, or a subcategory of it (IAB7 covers IAB7-5 but not IAB70).
pub fn category_within(cat: &str, r: &str) -> bool {
    cat == r
        || cat
            .strip_prefix(r)
            .is_some_and(|rest| rest.starts_with('-'))
}

fn within(inner: Rect, outer: Rect) -> bool {
    let (i, o) = (inner.0, outer.0);
    // Wrapping arithmetic mirrors Go's int64 and never panics in debug builds.
    i[0] >= o[0]
        && i[1] >= o[1]
        && i[0].wrapping_add(i[2]) <= o[0].wrapping_add(o[2])
        && i[1].wrapping_add(i[3]) <= o[1].wrapping_add(o[3])
}

fn overlaps(a: Rect, b: Rect) -> bool {
    let (a, b) = (a.0, b.0);
    a[0] < b[0].wrapping_add(b[2])
        && b[0] < a[0].wrapping_add(a[2])
        && a[1] < b[1].wrapping_add(b[3])
        && b[1] < a[1].wrapping_add(a[3])
}

fn dims(p: [i64; 2]) -> String {
    format!("{} \u{d7} {} px", p[0], p[1])
}

fn size(b: i64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if b as f64 >= MB {
        format!("{} MB", num(((b as f64 / MB) * 10.0).round() / 10.0))
    } else {
        format!("{b} bytes")
    }
}

/// Go's `title`: underscores to spaces, first character upper-cased.
///
/// Go works on the first byte, which garbles a multi-byte first character;
/// this maps the first character instead (an accepted difference).
pub(crate) fn title(role: &str) -> String {
    map_first_char(&role.replace('_', " "), |c, out| {
        out.extend(c.to_uppercase())
    })
}

pub(crate) fn map_first_char(s: &str, f: impl Fn(char, &mut String)) -> String {
    let mut chars = s.chars();
    let Some(c) = chars.next() else {
        return String::new();
    };
    let mut out = String::with_capacity(s.len());
    f(c, &mut out);
    out.push_str(chars.as_str());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_within_cases() {
        for (cat, r, want) in [
            ("IAB7-5", "IAB7-5", true),
            ("IAB7-5", "IAB7", true),
            ("IAB70", "IAB7", false),
            ("IAB7", "IAB7-5", false),
            ("IAB8", "IAB7-5", false),
        ] {
            assert_eq!(category_within(cat, r), want, "{cat} within {r}");
        }
    }

    #[test]
    fn sizes_and_dims_print_like_go() {
        assert_eq!(dims([1080, 1920]), "1080 \u{d7} 1920 px");
        assert_eq!(size(512), "512 bytes");
        assert_eq!(size(60 * 1024 * 1024), "60 MB");
        assert_eq!(size(1_572_864), "1.5 MB");
    }

    #[test]
    fn rect_arithmetic_wraps_like_go_without_panicking() {
        let big = Rect([i64::MAX, i64::MAX, 100, 100]);
        let unit = Rect([0, 0, 10, 10]);
        // Go's int64 sums wrap negative, so the far edge compares as inside
        // (the review's "pass"); the point is that nothing panics.
        assert!(within(big, unit));
        assert!(!within(unit, big));
        assert!(!overlaps(big, unit));
        let _ = overlaps(big, big);
    }

    #[test]
    fn safe_area_survives_overflowing_rects() {
        use crate::testutil::{check, lunch_file};
        let mut f = lunch_file();
        f.elements[0].rect_px = Some(Rect([i64::MAX, 0, 100, 5]));
        f.elements[1].rect_px = Some(Rect([0, i64::MAX, i64::MAX, i64::MAX]));
        let _ = check(&f, None, "harbourline.example/retail/portrait-totem-75");
    }
}
