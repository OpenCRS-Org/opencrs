//! The arithmetic behind legibility and glance rules, as plain functions, and
//! the calculators built from them. The checks use the same functions, so a
//! calculator's answer is always the validator's.

use crate::legibility::{DEFAULT_READING_RATE_WPS, num};
use crate::packcheck::{DEFAULT_LEGIBILITY_RATIO, glance_at};
use crate::rulepack::{RulePack, builtin_rule_packs};
use serde::Serialize;

/// The smallest cap height, in millimetres, readable from `distance_m` when
/// text is legible at `ratio` times its height.
pub fn cap_height_mm(distance_m: f64, ratio: f64) -> f64 {
    distance_m * 1000.0 / ratio
}

/// Converts a size in millimetres on the screen to pixels, rounding up: a
/// minimum must never round down to something smaller.
pub fn mm_to_px(mm: f64, px_per_mm: f64) -> i64 {
    // The epsilon stops float noise (46.0000001) from adding a pixel.
    (mm * px_per_mm - 1e-9).ceil() as i64
}

/// Whole words read in `seconds` at `rate` words per second.
pub fn words_in(seconds: f64, rate: f64) -> i64 {
    (seconds * rate + 1e-9).floor() as i64
}

fn round1(f: f64) -> f64 {
    (f * 10.0).round() / 10.0
}

fn round2(f: f64) -> f64 {
    (f * 100.0).round() / 100.0
}

/// Checks a calculator's inputs: every number finite, distances from near
/// to far, ratio and rate above zero.
fn check_inputs(
    distance_m: [f64; 2],
    ratio: Option<f64>,
    rate: Option<f64>,
    screen: Option<ScreenWidth>,
) -> Result<(), String> {
    let [near, far] = distance_m;
    if !near.is_finite() || !far.is_finite() || near < 0.0 || far <= 0.0 {
        return Err("viewing distances must be numbers of metres, the far one above zero".into());
    }
    if near > far {
        return Err(format!(
            "the near distance ({} m) is past the far one ({} m)",
            num(near),
            num(far)
        ));
    }
    for (name, v) in [("ratio", ratio), ("reading rate", rate)] {
        if v.is_some_and(|v| !v.is_finite() || v <= 0.0) {
            return Err(format!("the {name} must be a number above zero"));
        }
    }
    if screen.is_some_and(|s| !s.mm.is_finite() || s.mm <= 0.0 || s.px <= 0) {
        return Err("the screen's width must be above zero in both millimetres and pixels".into());
    }
    Ok(())
}

/// A screen's width in millimetres and in pixels, for converting sizes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenWidth {
    pub mm: f64,
    pub px: i64,
}

impl ScreenWidth {
    fn px_per_mm(self) -> Option<f64> {
        (self.mm > 0.0 && self.px > 0).then(|| self.px as f64 / self.mm)
    }
}

/// A minimum text size and its working.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextSize {
    /// Rounded to 0.1 mm, as reports show it.
    pub mm: f64,
    /// Whole pixels, rounded up; absent without a screen width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub px: Option<i64>,
    /// The exact pixel size before rounding up, to two places.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub px_exact: Option<f64>,
    /// How it was worked out, e.g. "at 6 m, ratio 150".
    pub basis: String,
}

fn text_size(distance_m: f64, ratio: f64, screen: Option<ScreenWidth>, basis: String) -> TextSize {
    let mm = cap_height_mm(distance_m, ratio);
    let d = screen.and_then(ScreenWidth::px_per_mm);
    TextSize {
        mm: round1(mm),
        px: d.map(|d| mm_to_px(mm, d)),
        px_exact: d.map(|d| round2(mm * d)),
        basis,
    }
}

/// A word limit and its working.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WordLimit {
    pub value: i64,
    pub basis: String,
}

/// What the legibility calculator answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Legibility {
    pub ratio: f64,
    /// The key message, read from the far end of the range.
    pub key: TextSize,
    /// Legal text, read from the near end.
    pub legal: TextSize,
    /// Words in the dwell time, when one is given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<WordLimit>,
}

/// Works out minimum text sizes for a screen seen from `distance_m` =
/// [near, far], as the legibility check does, and the word limit for a dwell
/// time. `ratio` and `rate` take the spec's defaults when absent. Inputs
/// that make no sense (a near distance past the far one, a ratio of zero)
/// are an error.
pub fn legibility(
    distance_m: [f64; 2],
    ratio: Option<f64>,
    screen: Option<ScreenWidth>,
    dwell_s: Option<f64>,
    rate: Option<f64>,
) -> Result<Legibility, String> {
    check_inputs(distance_m, ratio, rate, screen)?;
    if dwell_s.is_some_and(|t| !t.is_finite() || t < 0.0) {
        return Err("the dwell time must be a number of seconds, zero or more".into());
    }
    let ratio = ratio.unwrap_or(DEFAULT_LEGIBILITY_RATIO);
    let rate = rate.unwrap_or(DEFAULT_READING_RATE_WPS);
    let at = |d: f64| {
        text_size(
            d,
            ratio,
            screen,
            format!("at {} m, ratio {}", num(d), num(ratio)),
        )
    };
    Ok(Legibility {
        ratio,
        key: at(distance_m[1]),
        legal: at(distance_m[0]),
        words: dwell_s.map(|t| WordLimit {
            value: words_in(t, rate),
            basis: format!("{} s dwell at {} words/s", num(t), num(rate)),
        }),
    })
}

/// What a driver can read, rounded for display.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GlanceView {
    pub speed_m_s: f64,
    /// Seconds spent crossing the viewing range.
    pub crossing_s: f64,
    /// Reading time: one glance, or the crossing time if shorter.
    pub seconds: f64,
    pub words: i64,
    /// Where reading must start to finish by the near end.
    pub read_from_m: f64,
}

/// One rule pack's limits for a sign at a given speed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackLimits {
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub status_note: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_url: String,
    /// The minimum dwell at this speed, with the band it came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_dwell_s: Option<f64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub dwell_basis: String,
    /// Set when the pack sets dwell by speed but has no band for this one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub dwell_unknown: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_transition_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motion_allowed: Option<bool>,
    pub no_flashing: bool,
    pub max_words_at_glance: bool,
    pub no_sequencing: bool,
    pub no_traffic_device_imagery: bool,
    pub no_device_interaction: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub failure_state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub school_zone: String,
}

impl PackLimits {
    fn of(p: &RulePack, speed_kmh: f64) -> PackLimits {
        let bands = &p.playback.min_dwell_s;
        let (mut min_dwell_s, mut dwell_basis, mut dwell_unknown) =
            (None, String::new(), String::new());
        if !bands.0.is_empty() {
            if !bands.by_speed() {
                min_dwell_s = Some(bands.0[0].value);
            } else if let Some(b) = bands.at(speed_kmh) {
                min_dwell_s = Some(b.value);
                dwell_basis = b.describe();
            } else {
                dwell_unknown = format!("sets no dwell for {} km/h", num(speed_kmh));
            }
        }
        let motion = p.creative.motion.as_ref();
        PackLimits {
            reference: p.reference(),
            title: p.title.clone(),
            status: p.status.clone(),
            status_note: p.status_note.clone(),
            source_title: p.source_title.clone(),
            source_url: p.source_url.clone(),
            min_dwell_s,
            dwell_basis,
            dwell_unknown,
            max_transition_s: p.playback.max_transition_s,
            motion_allowed: motion.and_then(|m| m.allowed),
            no_flashing: motion.is_some_and(|m| m.no_flashing == Some(true)),
            max_words_at_glance: p.creative.max_words_at_glance,
            no_sequencing: p.creative.no_sequencing,
            no_traffic_device_imagery: p.creative.no_traffic_device_imagery,
            no_device_interaction: p.creative.no_device_interaction,
            failure_state: p.playback.failure_state.clone(),
            school_zone: p.playback.school_zone.clone(),
        }
    }
}

/// What the dwell calculator answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Dwell {
    pub speed_kmh: f64,
    pub ratio: f64,
    pub rate: f64,
    pub glance: GlanceView,
    /// Key text must be legible from where reading starts.
    pub key: TextSize,
    pub packs: Vec<PackLimits>,
}

/// Works out what a driver at `speed_kmh` can read of a sign seen from
/// `distance_m` = [near, far], as CRS-GLANCE-TEXT does, and the limits each
/// named built-in pack sets at that speed. An unknown pack, or inputs that
/// make no sense, are an error.
pub fn dwell(
    speed_kmh: f64,
    distance_m: [f64; 2],
    ratio: Option<f64>,
    rate: Option<f64>,
    screen: Option<ScreenWidth>,
    packs: &[String],
) -> Result<Dwell, String> {
    check_inputs(distance_m, ratio, rate, screen)?;
    if !speed_kmh.is_finite() || speed_kmh < 0.0 {
        return Err("the speed must be a number of km/h, zero or more".into());
    }
    let ratio = ratio.unwrap_or(DEFAULT_LEGIBILITY_RATIO);
    let rate = rate.unwrap_or(DEFAULT_READING_RATE_WPS);
    let g = glance_at(speed_kmh, distance_m, rate);
    let v = speed_kmh / 3.6;
    let known = builtin_rule_packs();
    let packs = packs
        .iter()
        .map(|r| {
            known
                .get(r)
                .map(|p| PackLimits::of(p, speed_kmh))
                .ok_or_else(|| format!("unknown rule pack {r}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Dwell {
        speed_kmh,
        ratio,
        rate,
        glance: GlanceView {
            speed_m_s: round2(v),
            crossing_s: if v > 0.0 {
                round2((distance_m[1] - distance_m[0]) / v)
            } else {
                0.0
            },
            seconds: round2(g.seconds),
            words: g.words,
            read_from_m: round2(g.read_from_m),
        },
        key: text_size(
            g.read_from_m,
            ratio,
            screen,
            format!(
                "to read from {} m, the start of a {} s glance at {} km/h (ratio {})",
                num(g.read_from_m.round()),
                num(round2(g.seconds)),
                num(speed_kmh),
                num(ratio),
            ),
        ),
        packs,
    })
}

/// A built-in pack, for choosing one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackSummary {
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_url: String,
}

/// The packs built into the validator, in ref order.
pub fn pack_list() -> Vec<PackSummary> {
    builtin_rule_packs()
        .values()
        .map(|p| PackSummary {
            reference: p.reference(),
            title: p.title.clone(),
            status: p.status.clone(),
            source_url: p.source_url.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The spec's worked examples.

    #[test]
    fn legibility_worked_example() {
        // 6 m at ratio 150 is 40 mm, which is 45.96 px, so 46 px.
        let l = legibility(
            [2.0, 6.0],
            None,
            Some(ScreenWidth {
                mm: 940.0,
                px: 1080,
            }),
            Some(4.0),
            None,
        )
        .unwrap();
        assert_eq!(l.key.mm, 40.0);
        assert_eq!(l.key.px_exact, Some(45.96));
        assert_eq!(l.key.px, Some(46));
        assert_eq!(l.key.basis, "at 6 m, ratio 150");
        // Legal text: 13.3 mm, which is 15.3 px, so 16 px.
        assert_eq!(l.legal.mm, 13.3);
        assert_eq!(l.legal.px_exact, Some(15.32));
        assert_eq!(l.legal.px, Some(16));
        assert_eq!(l.legal.basis, "at 2 m, ratio 150");
        assert_eq!(l.words.unwrap().value, 8);
    }

    #[test]
    fn legibility_without_a_screen_gives_millimetres_only() {
        let l = legibility([3.0, 10.0], Some(250.0), None, None, None).unwrap();
        assert_eq!(l.key.mm, 40.0);
        assert_eq!(l.key.px, None);
        assert!(l.words.is_none());
    }

    #[test]
    fn dwell_worked_examples() {
        // At 80 km/h (22.22 m/s), 20–150 m takes 5.85 s to cross, so one
        // 2 s glance: 4 words, read from 20 + 44.44 = 64.44 m, at least
        // 429.6 mm at ratio 150.
        let d = dwell(80.0, [20.0, 150.0], None, None, None, &[]).unwrap();
        assert_eq!(d.glance.speed_m_s, 22.22);
        assert_eq!(d.glance.crossing_s, 5.85);
        assert_eq!(d.glance.seconds, 2.0);
        assert_eq!(d.glance.words, 4);
        assert_eq!(d.glance.read_from_m, 64.44);
        assert_eq!(d.key.mm, 429.6);
        // A short range cuts the glance: 1.44 s, 2 words, read from 50 m.
        let d = dwell(100.0, [10.0, 50.0], None, None, None, &[]).unwrap();
        assert_eq!(d.glance.seconds, 1.44);
        assert_eq!(d.glance.words, 2);
        assert_eq!(d.glance.read_from_m, 50.0);
    }

    #[test]
    fn dwell_reads_the_packs_band_for_the_speed() {
        let r = "au-nsw-transport-corridor@2017-11".to_string();
        let slow = dwell(
            60.0,
            [20.0, 150.0],
            None,
            None,
            None,
            std::slice::from_ref(&r),
        )
        .unwrap();
        assert_eq!(slow.packs[0].min_dwell_s, Some(10.0));
        assert_eq!(slow.packs[0].dwell_basis, "below 80 km/h");
        assert_eq!(slow.packs[0].motion_allowed, Some(false));
        let fast = dwell(80.0, [20.0, 150.0], None, None, None, &[r]).unwrap();
        assert_eq!(fast.packs[0].min_dwell_s, Some(25.0));
        assert!(dwell(80.0, [20.0, 150.0], None, None, None, &["nope@1".into()]).is_err());
    }

    #[test]
    fn nonsense_inputs_are_errors() {
        let w = Some(ScreenWidth {
            mm: 940.0,
            px: 1080,
        });
        assert!(legibility([6.0, 2.0], None, w, None, None).is_err());
        assert!(legibility([2.0, 6.0], Some(0.0), w, None, None).is_err());
        assert!(legibility([2.0, f64::NAN], None, w, None, None).is_err());
        assert!(
            legibility(
                [2.0, 6.0],
                None,
                Some(ScreenWidth { mm: 0.0, px: 1080 }),
                None,
                None
            )
            .is_err()
        );
        assert!(legibility([2.0, 6.0], None, w, Some(-1.0), None).is_err());
        assert!(dwell(-5.0, [20.0, 150.0], None, None, None, &[]).is_err());
        assert!(dwell(80.0, [20.0, 150.0], None, Some(0.0), None, &[]).is_err());
        // Standing still: the full glance, read from the near end.
        let d = dwell(0.0, [20.0, 150.0], None, None, None, &[]).unwrap();
        assert_eq!((d.glance.seconds, d.glance.read_from_m), (2.0, 20.0));
    }

    #[test]
    fn pack_list_names_every_builtin_pack() {
        let l = pack_list();
        assert_eq!(l.len(), builtin_rule_packs().len());
        assert!(
            l.iter()
                .all(|p| !p.title.is_empty() && !p.status.is_empty())
        );
    }
}
