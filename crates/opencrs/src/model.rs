//! The OpenCRS 0.1 document model, ported from `crs/model.go`. Field names,
//! order and omission rules match Go's JSON output.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

/// Go's `omitempty` on a slice that Rust keeps as `Option` so an empty
/// array stays distinct from an absent one.
fn none_or_empty<T>(v: &Option<Vec<T>>) -> bool {
    v.as_ref().is_none_or(|v| v.is_empty())
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Document is the root of a screens.json requirements document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub opencrs: String,
    pub publisher: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expires_at: String,
    pub networks: Vec<Network>,
}

/// A set of screens sold or managed together.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Network {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<RuleSet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub venues: Vec<Venue>,
    pub formats: Vec<Format>,
    pub screens: Vec<Screen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
}

/// A centre, airport or station.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Venue {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub venue_type: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<RuleSet>,
}

/// The fields a Format defines and a Screen can override.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FormatFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels: Option<[i64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_mm: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<Media>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_area: Option<Insets>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_regions: Vec<Region>,
    #[serde(default, skip_serializing_if = "none_or_empty")]
    pub segments: Option<Vec<Segment>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<RuleSet>,
}

/// Shared by every screen of one kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Format {
    pub id: String,
    #[serde(flatten)]
    pub fields: FormatFields,
}

/// The format fields that differ for one screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Overrides {
    #[serde(flatten)]
    pub fields: FormatFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<OverridesRemove>,
}

/// What an override drops from the format.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OverridesRemove {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_regions: Vec<String>,
}

/// Accepted file types and playback limits.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Media {
    #[serde(default, rename = "static", skip_serializing_if = "none_or_empty")]
    pub static_types: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "none_or_empty")]
    pub motion: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(default, skip_serializing_if = "none_or_empty")]
    pub codecs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub colour_space: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
}

/// Margins in pixels. A margin left out is zero, as in Go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Insets {
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
    pub left: i64,
}

/// `[x, y, width, height]` in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect(pub [i64; 4]);

impl fmt::Display for Rect {
    /// Go's `%v` of an `[4]int`: `[x y w h]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [x, y, w, h] = self.0;
        write!(f, "[{x} {y} {w} {h}]")
    }
}

/// An area the screen covers or overlays.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub name: String,
    pub rect_px: Rect,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

/// One panel of a composite surface.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub id: String,
    pub rect_px: Rect,
    #[serde(default, skip_serializing_if = "is_false")]
    pub primary: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub crossing: String,
}

/// One physical screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub id: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub venue: String,
    // `placement`, `overrides` and `rules` are boxed: a network lists tens of
    // thousands of screens, most of which set none of them, and inline they
    // made a Screen over a kilobyte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<Box<Placement>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewing: Option<Viewing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Box<Overrides>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Box<RuleSet>>,
    #[serde(default, skip_serializing_if = "PlatformIds::is_empty")]
    pub platform_ids: PlatformIds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

/// A screen's id on each playback platform, by platform. Kept as a sorted
/// list: a network lists hundreds of thousands of screens, and a one-entry
/// `BTreeMap` costs each of them a node of several hundred bytes. Reads and
/// writes as a JSON object, as a map would (a repeated key keeps the last).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlatformIds(Vec<(String, String)>);

impl PlatformIds {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, platform: &str) -> Option<&str> {
        let i = self
            .0
            .binary_search_by(|(k, _)| k.as_str().cmp(platform))
            .ok()?;
        Some(&self.0[i].1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl Serialize for PlatformIds {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(self.iter())
    }
}

impl<'de> Deserialize<'de> for PlatformIds {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = PlatformIds;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a map of platform names to ids")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut m: A,
            ) -> Result<Self::Value, A::Error> {
                let mut v: Vec<(String, String)> = Vec::new();
                while let Some(kv) = m.next_entry()? {
                    v.push(kv);
                }
                // Stable, so of repeated keys the last stays last.
                v.sort_by(|a, b| a.0.cmp(&b.0));
                let mut out: Vec<(String, String)> = Vec::with_capacity(v.len());
                for kv in v {
                    match out.last_mut() {
                        Some(last) if last.0 == kv.0 => *last = kv,
                        _ => out.push(kv),
                    }
                }
                Ok(PlatformIds(out))
            }
        }
        d.deserialize_map(V)
    }
}

/// A screen's coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
}

/// Where and how a screen is mounted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub environment: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub zone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mounted_height_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_allowed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_kmh: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faces_road: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub school_zone: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub luminance_zone: String,
}

/// How people see a screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Viewing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dwell_s: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub approach: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub light: String,
}

/// Rules that can sit on a network, venue, format or screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legibility: Option<Legibility>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_elements: Vec<RequiredElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_words: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_rate_wps: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restricted_categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<MotionRules>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<Brightness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback: Option<Playback>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_packs: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<RuleRemove>,
}

/// Drops inherited items that aren't restrictions. Restricted categories and
/// rule packs can't be removed: restrictions only tighten.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleRemove {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_elements: Vec<String>,
}

/// Minimum text size, directly or as a distance ratio.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Legibility {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_cap_height_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_legal_cap_height_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
}

/// Something every creative must include.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequiredElement {
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub when: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_cap_height_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_cap_height_px: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within_rect_px: Option<Rect>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

/// Limits on animation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MotionRules {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_flashing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_flash_hz: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_frame_hold_s: Option<f64>,
}

/// Luminance limits, enforced by the playback platform.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Brightness {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_night_cd_m2: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_day_cd_m2: Option<f64>,
}

/// How the platform shows creative, such as a site consent's minimum dwell.
/// Validators report these as info.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Playback {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_dwell_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_transition_s: Option<f64>,
}

/// How creative reaches a network.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_time_hours: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub submit: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_naming: String,
}

/// A creative.json.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub opencrs: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign: Option<Campaign>,
    pub files: Vec<CreativeFile>,
}

/// The advertiser and what the creative contains.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Campaign {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub advertiser: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// One file in a manifest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreativeFile {
    pub file: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels: Option<[i64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_flash_hz: Option<f64>,
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<Element>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attestations: Vec<Attestation>,
}

/// A person's statement that they reviewed a file for a check only a person
/// can judge, such as CRS-TRAFFIC-DEVICE. It counts only for the file whose
/// SHA-256 it names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Attestation {
    pub check: String,
    pub by: String,
    pub date: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The Dynamic Creative condition a file is built for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    pub dimension: String,
    pub value: String,
}

/// A piece of content placed in a creative.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect_px: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_height_px: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn rect_displays_like_go() {
        assert_eq!(Rect([0, 10, 200, 30]).to_string(), "[0 10 200 30]");
    }

    #[test]
    fn omitempty_and_integer_floats_match_go() {
        let f = Format {
            id: "f".into(),
            fields: FormatFields {
                pixels: Some([1080, 1920]),
                physical_mm: Some([940.0, 1670.5]),
                media: Some(Media {
                    static_types: Some(vec!["image/png".into()]),
                    slot_s: Some(10.0),
                    ..Media::default()
                }),
                ..FormatFields::default()
            },
        };
        assert_eq!(
            json::to_string(&f),
            r#"{"id":"f","pixels":[1080,1920],"physical_mm":[940,1670.5],"media":{"static":["image/png"],"slot_s":10}}"#
        );
    }

    #[test]
    fn overrides_flatten_and_roundtrip() {
        let o: Overrides =
            serde_json::from_str(r#"{"pixels":[1,2],"remove":{"blocked_regions":["a"]}}"#).unwrap();
        assert_eq!(o.fields.pixels, Some([1, 2]));
        assert_eq!(
            json::to_string(&o),
            r#"{"pixels":[1,2],"remove":{"blocked_regions":["a"]}}"#
        );
    }

    #[test]
    fn platform_ids_read_and_write_like_a_map() {
        let s: Screen = serde_json::from_str(
            r#"{"id":"a","format":"f","platform_ids":{"z":"1","cms-a":"2","z":"3"}}"#,
        )
        .unwrap();
        assert_eq!(s.platform_ids.get("z"), Some("3"));
        assert_eq!(s.platform_ids.get("cms-a"), Some("2"));
        assert_eq!(s.platform_ids.get("x"), None);
        assert_eq!(
            json::to_string(&s),
            r#"{"id":"a","format":"f","platform_ids":{"cms-a":"2","z":"3"}}"#
        );
        let bare: Screen = serde_json::from_str(r#"{"id":"a","format":"f"}"#).unwrap();
        assert_eq!(json::to_string(&bare), r#"{"id":"a","format":"f"}"#);
    }
}
