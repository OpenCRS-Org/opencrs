//! Text-size and word-count rules: how small text may be and how many words
//! a message may hold. Port of `crs/legibility.go`.

use crate::resolve::Requirements;
use unicode_general_category::{GeneralCategory as G, get_general_category};

/// The assumed reading rate, in words per second, for deriving a word limit
/// from dwell time. A working assumption until it is calibrated against real
/// campaigns.
pub const DEFAULT_READING_RATE_WPS: f64 = 2.0;

/// Separates the key message from fine print: the key message must read from
/// the far end of the viewing range, legal text only from the near end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextClass {
    Key,
    Legal,
}

const KEY_ROLES: [&str; 7] = [
    "headline", "subhead", "body", "cta", "offer", "price", "text",
];
const LEGAL_ROLES: [&str; 4] = ["legal", "disclaimer", "fine_print", "terms"];

/// Reports whether a role is key text, legal text, or not text at all (logos,
/// images).
pub fn class_of(role: &str) -> Option<TextClass> {
    if KEY_ROLES.contains(&role) {
        Some(TextClass::Key)
    } else if LEGAL_ROLES.contains(&role) {
        Some(TextClass::Legal)
    } else {
        None
    }
}

/// A minimum text size and how it was worked out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MinCapHeight {
    pub mm: f64,
    pub px: i64,
    /// Human-readable, e.g. "at 6 m, ratio 150".
    pub basis: String,
}

impl Requirements<'_> {
    /// The screen's pixel density, if its physical size is known.
    pub fn px_per_mm(&self) -> Option<f64> {
        match self.physical_mm {
            Some(p) if p[0] > 0.0 && self.pixels[0] != 0 => Some(self.pixels[0] as f64 / p[0]),
            _ => None,
        }
    }

    /// Converts a minimum size in millimetres on the screen to pixels,
    /// rounding up: a minimum must never round down to something smaller.
    pub fn mm_to_px(&self, mm: f64) -> Option<i64> {
        Some(crate::calc::mm_to_px(mm, self.px_per_mm()?))
    }

    /// Works out the minimum cap height for a class of text. The bool is
    /// `applies`: false when no legibility rule covers the class. When a
    /// rule applies but the screen lacks the data to evaluate it, the last
    /// value says what is `missing` (empty otherwise). If both a direct size
    /// and a ratio apply, the larger size wins.
    ///
    /// When the ratio can't be evaluated but a direct size can, the first
    /// value still holds the direct minimum alongside `missing`: the true
    /// minimum is unknown, but it is at least that, so text below it fails
    /// regardless.
    pub fn min_cap_height(&self, class: TextClass) -> (MinCapHeight, bool, &'static str) {
        let mut m = MinCapHeight::default();
        let Some(l) = &self.rules.legibility else {
            return (m, false, "");
        };
        let direct = if class == TextClass::Legal {
            l.min_legal_cap_height_mm
        } else {
            l.min_cap_height_mm
        };
        if direct.is_none() && l.ratio.is_none() {
            return (m, false, "");
        }
        if let Some(d) = direct {
            m.mm = d;
            m.basis = "set by the publisher".into();
        }
        if let Some(ratio) = l.ratio {
            let Some(dist) = self
                .screen
                .and_then(|s| s.viewing.as_ref())
                .and_then(|v| v.distance_m)
            else {
                if direct.is_some() {
                    // Zero if physical_mm is missing too.
                    m.px = self.mm_to_px(m.mm).unwrap_or(0);
                }
                return (m, true, "no viewing distance for the legibility ratio");
            };
            let d = if class == TextClass::Legal {
                dist[0]
            } else {
                dist[1]
            };
            let mm = crate::calc::cap_height_mm(d, ratio);
            if mm > m.mm {
                m.mm = mm;
                m.basis = format!("at {} m, ratio {}", num(d), num(ratio));
            }
        }
        match self.mm_to_px(m.mm) {
            Some(px) => {
                m.px = px;
                (m, true, "")
            }
            None => (m, true, "no physical_mm to convert millimetres to pixels"),
        }
    }

    /// The main-message word limit: `max_words` if set, otherwise dwell time
    /// times reading rate. Returns the limit and how it was worked out.
    pub fn word_limit(&self) -> Option<(i64, String)> {
        if let Some(w) = self.rules.max_words {
            return Some((w, "set by the publisher".into()));
        }
        let dwell = self.screen?.viewing.as_ref()?.dwell_s?;
        let rate = self
            .rules
            .reading_rate_wps
            .unwrap_or(DEFAULT_READING_RATE_WPS);
        Some((
            crate::calc::words_in(dwell, rate),
            format!("{} s dwell at {} words/s", num(dwell), num(rate)),
        ))
    }
}

/// Counts whitespace-separated tokens that contain a letter or digit.
pub fn count_words(s: &str) -> usize {
    s.split_whitespace()
        .filter(|f| {
            f.chars().any(|c| {
                matches!(
                    get_general_category(c),
                    G::UppercaseLetter
                        | G::LowercaseLetter
                        | G::TitlecaseLetter
                        | G::ModifierLetter
                        | G::OtherLetter
                        | G::DecimalNumber
                )
            })
        })
        .count()
}

/// Formats a number without trailing zeros: whole numbers as integers,
/// others to two decimals with trailing zeros trimmed.
pub(crate) fn num(f: f64) -> String {
    if f == f.trunc() {
        return format!("{}", f as i64);
    }
    let s = format!("{f:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::resolve::parse_ref;

    use crate::testutil::harbourline;

    fn resolve_one<'a>(d: &'a Document, r: &str) -> Requirements<'a> {
        let mut reqs = d.resolve(&parse_ref(r).unwrap()).unwrap();
        assert_eq!(reqs.len(), 1);
        reqs.remove(0)
    }

    // The spec's worked example: a 1080 x 1920 totem 940 mm wide, viewed from
    // 2-6 m at ratio 150, needs a 46 px headline and 16 px legal text: the
    // legal minimum is 15.3 px, and minimums round up.
    #[test]
    fn spec_worked_legibility_example() {
        let d = harbourline();
        let r = resolve_one(&d, "harbourline.example/retail/screen/HC-L1-001");
        let (key, applies, missing) = r.min_cap_height(TextClass::Key);
        assert!(applies && missing.is_empty() && key.px == 46 && key.mm as i64 == 40);
        let (legal, _, _) = r.min_cap_height(TextClass::Legal);
        assert_eq!(legal.px, 16);
    }

    // The spec says the second totem, seen from 3-10 m, needs larger text.
    #[test]
    fn further_screen_needs_larger_text() {
        let d = harbourline();
        let r = resolve_one(&d, "harbourline.example/retail/screen/HC-L1-002");
        let (key, _, _) = r.min_cap_height(TextClass::Key);
        let (legal, _, _) = r.min_cap_height(TextClass::Legal);
        assert_eq!((key.px, legal.px), (77, 23));
    }

    // 4 s dwell at 2 words per second allows 8 words.
    #[test]
    fn word_limit_from_dwell() {
        let d = harbourline();
        let r = resolve_one(&d, "harbourline.example/retail/screen/HC-L1-001");
        let (n, basis) = r.word_limit().unwrap();
        assert_eq!(n, 8);
        assert_eq!(basis, "4 s dwell at 2 words/s");
    }

    #[test]
    fn word_limit_prefers_max_words() {
        let d = harbourline();
        let mut r = resolve_one(&d, "harbourline.example/retail/screen/HC-L1-001");
        r.rules.max_words = Some(5);
        assert_eq!(r.word_limit(), Some((5, "set by the publisher".into())));
        r.rules.max_words = None;
        r.rules.reading_rate_wps = Some(1.5);
        assert_eq!(r.word_limit(), Some((6, "4 s dwell at 1.5 words/s".into())));
        r.screen = None;
        assert_eq!(r.word_limit(), None);
    }

    #[test]
    fn direct_legibility() {
        let d = harbourline();
        let mut r = resolve_one(&d, "harbourline.example/retail/screen/HC-L1-001");
        r.pixels = [1080, 1920];
        r.physical_mm = Some([940.0, 1670.0]);
        r.rules.legibility = Some(Legibility {
            min_cap_height_mm: Some(40.0),
            min_legal_cap_height_mm: Some(14.0),
            ratio: None,
        });
        let (key, _, _) = r.min_cap_height(TextClass::Key);
        let (legal, _, _) = r.min_cap_height(TextClass::Legal);
        assert_eq!((key.px, legal.px), (46, 17)); // 16.09 px rounds up
        // Without physical size the rule applies but can't be evaluated.
        r.physical_mm = None;
        let (_, applies, missing) = r.min_cap_height(TextClass::Key);
        assert!(applies && !missing.is_empty());
        // No legibility rule: not applicable at all.
        r.rules.legibility = None;
        let (_, applies, _) = r.min_cap_height(TextClass::Key);
        assert!(!applies);
    }

    // With both a direct size and a ratio, the larger minimum wins.
    #[test]
    fn direct_and_ratio_take_larger() {
        let d = harbourline();
        let mut r = resolve_one(&d, "harbourline.example/retail/screen/HC-L1-001");
        r.rules.legibility = Some(Legibility {
            ratio: Some(150.0),
            min_cap_height_mm: Some(50.0),
            min_legal_cap_height_mm: Some(10.0),
        });
        let (key, _, _) = r.min_cap_height(TextClass::Key);
        let (legal, _, _) = r.min_cap_height(TextClass::Legal);
        assert_eq!(key.mm, 50.0);
        assert_eq!(key.basis, "set by the publisher");
        assert_eq!(legal.px, 16);
        assert_eq!(legal.basis, "at 2 m, ratio 150");
    }

    // A ratio that can't be evaluated leaves the true minimum unknown, but the
    // direct minimum is still reported with it.
    #[test]
    fn direct_floor_survives_missing_viewing_distance() {
        let d = harbourline();
        let mut r = resolve_one(&d, "harbourline.example/retail/screen/HC-L1-001");
        r.rules.legibility = Some(Legibility {
            ratio: Some(150.0),
            min_legal_cap_height_mm: Some(14.0),
            min_cap_height_mm: None,
        });
        let mut s = r.screen.unwrap().clone();
        s.viewing = None;
        r.screen = Some(&s);
        let (legal, applies, missing) = r.min_cap_height(TextClass::Legal);
        assert!(applies);
        assert_eq!(missing, "no viewing distance for the legibility ratio");
        assert_eq!(legal.px, 17); // 14 mm
        // The headline has a ratio only: nothing to fall back on.
        let (key, _, _) = r.min_cap_height(TextClass::Key);
        assert_eq!((key.px, key.mm), (0, 0.0));
    }

    // A ratio that can't be evaluated leaves the true minimum unknown, but
    // text below the direct minimum fails regardless.
    #[test]
    fn direct_floor_fails_without_viewing_distance() {
        use crate::check::{CODE_LEGIBILITY, Severity};
        use crate::testutil::*;
        let mut d = harbourline();
        // 14 mm = 17 px
        d.networks[0].rules.as_mut().unwrap().legibility = Some(Legibility {
            ratio: Some(150.0),
            min_legal_cap_height_mm: Some(14.0),
            min_cap_height_mm: None,
        });
        d.networks[0].screens[0].viewing = None;
        let target = "harbourline.example/retail/screen/HC-L1-001";
        let c = offer_campaign();

        let mut f = lunch_file();
        f.elements[1].cap_height_px = Some(16.0);
        let rep = check_doc(&d, &f, Some(&c), target);
        let errs = find(&rep, CODE_LEGIBILITY, Severity::Error);
        assert_eq!(errs.len(), 1, "below the direct floor: {:?}", rep.checks);
        assert_eq!(errs[0].element, "legal");
        assert_eq!(
            errs[0].message,
            "Legal text is 16 px; 17 px minimum set by the publisher, and more may be needed (no viewing distance for the legibility ratio)"
        );

        // Above the floor, the ratio still decides: not checked.
        f.elements[1].cap_height_px = Some(30.0);
        let rep = check_doc(&d, &f, Some(&c), target);
        let nc = find(&rep, CODE_LEGIBILITY, Severity::NotChecked);
        assert!(find(&rep, CODE_LEGIBILITY, Severity::Error).is_empty());
        assert_eq!(
            nc.len(),
            2,
            "headline (ratio only) and legal: {:?}",
            rep.checks
        );
    }

    #[test]
    fn ratio_without_physical_size_says_so() {
        let d = harbourline();
        let mut r = resolve_one(&d, "harbourline.example/retail/screen/HC-L1-001");
        r.physical_mm = None;
        let (_, applies, missing) = r.min_cap_height(TextClass::Key);
        assert!(applies);
        assert_eq!(missing, "no physical_mm to convert millimetres to pixels");
    }

    #[test]
    fn count_words_cases() {
        for (s, want) in [
            ("Lunch specials.", 2),
            ("  Fresh \u{2014} hot & ready!  ", 3),
            ("", 0),
            ("11:30 to 2:30", 3),
            ("\u{5348}\u{9910}", 1),
            ("a\u{a0}b\u{3000}c\u{2028}d", 4),
            ("\u{2014} & !", 0),
        ] {
            assert_eq!(count_words(s), want, "count_words({s:?})");
        }
    }

    #[test]
    fn class_of_roles() {
        assert_eq!(class_of("headline"), Some(TextClass::Key));
        assert_eq!(class_of("cta"), Some(TextClass::Key));
        assert_eq!(class_of("fine_print"), Some(TextClass::Legal));
        assert_eq!(class_of("logo"), None);
    }

    #[test]
    fn num_matches_go() {
        assert_eq!(num(150.0), "150");
        assert_eq!(num(-3.0), "-3");
        assert_eq!(num(0.1), "0.1");
        assert_eq!(num(1.44), "1.44");
        assert_eq!(num(64.4444), "64.44");
        assert_eq!(num(0.72), "0.72");
        assert_eq!(num(2.5), "2.5");
        assert_eq!(num(100.001), "100");
        assert_eq!(num(0.004), "0");
        assert_eq!(num(-0.001), "-0");
        // Exact ties round to even, as in Go's %.2f.
        assert_eq!(num(0.125), "0.12");
        assert_eq!(num(0.375), "0.38");
    }
}
