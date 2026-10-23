//! Rule packs: shared, versioned encodings of published regulatory rules.
//! Port of `crs/rulepack.go` and `rulepacks/embed.go`.

use crate::error::Error;
use crate::legibility::num;
use crate::load::decode;
use crate::model::{MotionRules, is_false};
use crate::schema::Kind;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Rule pack statuses. A pack's status says how far its values can be
/// trusted; validators report anything short of draft.
///
/// From a summary, not checked against the primary source.
pub const PACK_UNVERIFIED: &str = "unverified";
/// Checked against the primary source by the maintainer.
pub const PACK_DRAFT: &str = "draft";
/// Also independently reviewed.
pub const PACK_REVIEWED: &str = "reviewed";

/// A shared, versioned encoding of published regulatory rules, such as a
/// state's roadside signage guidelines.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RulePack {
    #[serde(rename = "rule_pack", default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_title: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status_note: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub citations: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default)]
    pub creative: PackCreative,
    #[serde(default)]
    pub playback: PackPlayback,
}

impl RulePack {
    /// The pack's reference, `name@version`.
    pub fn reference(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }
}

/// The rules checked when creative is validated.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PackCreative {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<MotionRules>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_traffic_device_imagery: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub max_words_at_glance: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_sequencing: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_device_interaction: bool,
}

/// The rules the screen platform enforces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PackPlayback {
    #[serde(default, skip_serializing_if = "DwellBands::is_empty")]
    pub min_dwell_s: DwellBands,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_transition_s: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub failure_state: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub school_zone: String,
    /// Keyed by lighting period, then by luminance zone ("default" for any
    /// zone). A `None` value means no limit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub max_luminance_cd_m2: BTreeMap<String, BTreeMap<String, Option<f64>>>,
}

/// A value that applies from `speed_kmh_from` (inclusive) to
/// `speed_kmh_below` (exclusive) km/h. A missing bound is open.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpeedBand {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_kmh_from: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_kmh_below: Option<f64>,
    pub value: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl SpeedBand {
    /// A band with no bounds.
    pub fn of(value: f64) -> Self {
        SpeedBand {
            value,
            ..Default::default()
        }
    }

    /// Says what a band covers, e.g. "at 80 km/h and over".
    pub fn describe(&self) -> String {
        match (self.speed_kmh_from, self.speed_kmh_below) {
            (Some(f), Some(b)) => format!("from {} to below {} km/h", num(f), num(b)),
            (Some(f), None) => format!("at {} km/h and over", num(f)),
            (None, Some(b)) => format!("below {} km/h", num(b)),
            (None, None) => String::new(),
        }
    }
}

/// A minimum dwell: one band with no bounds for a single number, or bands
/// chosen by speed.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DwellBands(pub Vec<SpeedBand>);

impl<'de> Deserialize<'de> for DwellBands {
    /// Accepts a number or a list of bands.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Number(f64),
            Bands(Vec<SpeedBand>),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Number(n) => DwellBands(vec![SpeedBand::of(n)]),
            Raw::Bands(b) => DwellBands(b),
        })
    }
}

impl DwellBands {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether the bands depend on speed.
    pub fn by_speed(&self) -> bool {
        self.0
            .iter()
            .any(|b| b.speed_kmh_from.is_some() || b.speed_kmh_below.is_some())
    }

    /// The first band that covers `speed`.
    pub fn at(&self, speed: f64) -> Option<&SpeedBand> {
        self.0.iter().find(|b| {
            b.speed_kmh_from.is_none_or(|f| speed >= f)
                && b.speed_kmh_below.is_none_or(|u| speed < u)
        })
    }
}

/// The packs a validator knows, keyed by `name@version`.
pub type RulePacks = BTreeMap<String, RulePack>;

/// The refs of packs with the given name, in order, for suggesting a known
/// version when an unknown one is referenced.
pub(crate) fn versions_of(packs: &RulePacks, name: &str) -> Vec<String> {
    packs
        .iter()
        .filter(|(_, p)| p.name == name)
        .map(|(r, _)| r.clone())
        .collect()
}

/// Reads each `*.json` file as a rule pack, checking it against the rule-pack
/// schema and that its file name is its `name@version`. Files whose names
/// don't end in `.json` are ignored.
pub fn load_rule_packs(files: &[(&str, &[u8])]) -> Result<RulePacks, Error> {
    let mut sorted: Vec<&(&str, &[u8])> = files
        .iter()
        .filter(|(name, _)| name.ends_with(".json"))
        .collect();
    sorted.sort_by_key(|(name, _)| *name);
    let mut out = RulePacks::new();
    for (name, raw) in sorted {
        let p = decode_rule_pack(raw).map_err(|e| Error::Message(format!("{name}: {e}")))?;
        let base = name.rsplit('/').next().unwrap_or(name);
        let want = base.strip_suffix(".json").unwrap_or(base);
        let reference = p.reference();
        if reference != want {
            return Err(Error::Message(format!(
                "{name} holds rule pack {reference}; name the file {reference}.json"
            )));
        }
        out.insert(reference, p);
    }
    Ok(out)
}

/// Parses a rule pack, first checking it against the schema.
pub fn decode_rule_pack(raw: &[u8]) -> Result<RulePack, Error> {
    let mut p: RulePack = decode(Kind::RulePack, raw)?;
    if p.status.is_empty() {
        // The schema's default: unchecked until someone says otherwise.
        p.status = PACK_UNVERIFIED.into();
    }
    Ok(p)
}

macro_rules! embed {
    ($($name:literal),+ $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../../rulepacks/", $name)) as &[u8])),+]
    };
}

/// The pack files published with OpenCRS, so every build of the validator
/// knows them by `name@version` with no extra files or flags.
const BUILTIN_FILES: &[(&str, &[u8])] = embed![
    "au-nsw-transport-corridor@2017-11.json",
    "au-qld-tmr-roadside-advertising@2022-12.json",
    "au-sa-dit-advertising-signs@2014-08.json",
    "au-tas-spp-signs-code@2024-06.json",
    "au-vic-dtp-illuminated-signage@2023-10.json",
    "au-wa-mrwa-digital-signs@2025-05.json",
];

/// The packs published with OpenCRS, embedded in the validator and loaded
/// once.
///
/// # Panics
/// If an embedded pack is invalid, which the tests rule out.
pub fn builtin_rule_packs() -> &'static RulePacks {
    static PACKS: OnceLock<RulePacks> = OnceLock::new();
    PACKS.get_or_init(|| {
        load_rule_packs(BUILTIN_FILES).unwrap_or_else(|e| panic!("embedded rule packs: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::lint;
    use crate::schema::{SchemaOptions, detect_kind, validate_schema};
    use std::path::Path;

    fn repo(path: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    #[test]
    fn builtin_rule_packs_load() {
        let packs = builtin_rule_packs();
        let want = [
            ("au-nsw-transport-corridor@2017-11", PACK_DRAFT),
            ("au-qld-tmr-roadside-advertising@2022-12", PACK_DRAFT),
            ("au-vic-dtp-illuminated-signage@2023-10", PACK_DRAFT),
            ("au-wa-mrwa-digital-signs@2025-05", PACK_DRAFT),
            ("au-sa-dit-advertising-signs@2014-08", PACK_DRAFT),
            ("au-tas-spp-signs-code@2024-06", PACK_UNVERIFIED), // later amendments unread
        ];
        assert_eq!(packs.len(), want.len());
        for (r, status) in want {
            let p = packs.get(r).unwrap_or_else(|| panic!("{r} not loaded"));
            assert_eq!(p.status, status, "{r}");
            assert!(
                !p.source_url.is_empty() && !p.status_note.is_empty() && !p.citations.is_empty(),
                "{r}: needs source_url, status_note and citations"
            );
        }
    }

    // Every file in rulepacks/ must be embedded, and nothing else.
    #[test]
    fn every_pack_file_is_embedded() {
        let mut on_disk: Vec<String> = std::fs::read_dir(repo("rulepacks"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".json"))
            .collect();
        on_disk.sort();
        let embedded: Vec<&str> = BUILTIN_FILES.iter().map(|(n, _)| *n).collect();
        assert_eq!(on_disk, embedded);
    }

    // Every embedded pack must pass the rule-pack schema, be named for its
    // name@version, and cite a source for every rule it encodes.
    #[test]
    fn embedded_packs_match_schema() {
        assert!(!BUILTIN_FILES.is_empty());
        for (name, raw) in BUILTIN_FILES {
            assert_eq!(detect_kind(raw).unwrap(), Kind::RulePack, "{name}");
            let errs = validate_schema(Kind::RulePack, raw, &SchemaOptions::default()).unwrap();
            assert!(errs.is_empty(), "{name}: {errs:?}");
            let generic: serde_json::Value = serde_json::from_slice(raw).unwrap();
            let citations = &generic["citations"];
            let cited = |k: &str| citations[k].as_str().is_some_and(|s| !s.is_empty());
            for section in ["creative", "playback"] {
                for key in generic[section]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(k, _)| k)
                {
                    assert!(
                        cited(&format!("{section}.{key}"))
                            || cited(&format!("{section}.{key}.allowed")),
                        "{name}: {section}.{key} has no citation"
                    );
                }
            }
        }
    }

    const GOOD: &str =
        r#"{"rule_pack":"x-test","version":"2026-01","source_url":"https://a.example/x.pdf"}"#;

    #[test]
    fn load_rule_packs_rejects() {
        let wrong_name = [("x-test@2025-01.json", GOOD.as_bytes())];
        let e = load_rule_packs(&wrong_name).unwrap_err().to_string();
        assert_eq!(
            e,
            "x-test@2025-01.json holds rule pack x-test@2026-01; name the file x-test@2026-01.json"
        );
        let bad_schema = [(
            "x-test@2026-01.json",
            br#"{"rule_pack":"x-test","version":"2026-01"}"# as &[u8],
        )];
        let e = load_rule_packs(&bad_schema).unwrap_err().to_string();
        assert!(
            e.starts_with("x-test@2026-01.json: not a valid OpenCRS rule-pack document:"),
            "{e}"
        );
    }

    #[test]
    fn pack_without_status_defaults_to_unverified() {
        let files = [
            ("x-test@2026-01.json", GOOD.as_bytes()),
            ("README.md", b"ignored, not json"),
        ];
        let packs = load_rule_packs(&files).unwrap();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs["x-test@2026-01"].status, PACK_UNVERIFIED);
    }

    // The spec's rule-pack example must say exactly what the NSW pack does.
    #[test]
    fn spec_rule_pack_example_matches_nsw_pack() {
        let spec = std::fs::read_to_string(repo("OpenCRS 0.1 Specification.md")).unwrap();
        let mut block = None;
        for b in spec.split("```json\n").skip(1) {
            let b = &b[..b.find("```").unwrap()];
            if b.contains(r#""rule_pack": "au-nsw-transport-corridor""#) {
                block = Some(b);
            }
        }
        let block = block.expect("no NSW rule-pack example in the spec");
        let (_, raw) = BUILTIN_FILES
            .iter()
            .find(|(n, _)| *n == "au-nsw-transport-corridor@2017-11.json")
            .unwrap();
        let in_spec: serde_json::Value = serde_json::from_str(block).unwrap();
        let in_pack: serde_json::Value = serde_json::from_slice(raw).unwrap();
        for k in [
            "rule_pack",
            "version",
            "source_url",
            "status",
            "creative",
            "playback",
        ] {
            assert_eq!(
                in_spec[k], in_pack[k],
                "spec example and pack differ in {k}"
            );
        }
    }

    #[test]
    fn speed_bands() {
        let packs = builtin_rule_packs();
        let cases: [(&str, f64, f64); 18] = [
            ("au-nsw-transport-corridor@2017-11", 60.0, 10.0),
            ("au-nsw-transport-corridor@2017-11", 79.0, 10.0),
            ("au-nsw-transport-corridor@2017-11", 80.0, 25.0), // "80 km/h and over"
            ("au-nsw-transport-corridor@2017-11", 110.0, 25.0),
            ("au-qld-tmr-roadside-advertising@2022-12", 60.0, 10.0),
            ("au-qld-tmr-roadside-advertising@2022-12", 79.0, 10.0), // "less than and not equal to 80 km/h"
            ("au-qld-tmr-roadside-advertising@2022-12", 80.0, 15.0), // "80 km/h and above"
            ("au-qld-tmr-roadside-advertising@2022-12", 110.0, 15.0),
            ("au-wa-mrwa-digital-signs@2025-05", 40.0, 10.0),
            ("au-wa-mrwa-digital-signs@2025-05", 70.0, 10.0), // Table 6: 70 km/h or less
            ("au-wa-mrwa-digital-signs@2025-05", 79.0, 10.0),
            ("au-wa-mrwa-digital-signs@2025-05", 80.0, 15.0), // 80-90 km/h
            ("au-wa-mrwa-digital-signs@2025-05", 90.0, 15.0),
            ("au-wa-mrwa-digital-signs@2025-05", 99.0, 15.0),
            ("au-wa-mrwa-digital-signs@2025-05", 100.0, 20.0), // 100 km/h or more
            ("au-wa-mrwa-digital-signs@2025-05", 110.0, 20.0),
            ("au-sa-dit-advertising-signs@2014-08", 40.0, 45.0),
            ("au-sa-dit-advertising-signs@2014-08", 110.0, 45.0),
        ];
        for (pack, speed, want) in cases {
            let got = packs[pack].playback.min_dwell_s.at(speed).map(|b| b.value);
            assert_eq!(got, Some(want), "{pack} at {speed} km/h");
        }
        let gap = DwellBands(vec![
            SpeedBand {
                speed_kmh_from: Some(60.0),
                ..SpeedBand::of(40.0)
            },
            SpeedBand {
                speed_kmh_below: Some(50.0),
                ..SpeedBand::of(45.0)
            },
        ]);
        assert!(
            gap.at(55.0).is_none(),
            "a speed no band covers has no value"
        );
        // Packs with no dwell rule have no bands at all.
        for r in [
            "au-vic-dtp-illuminated-signage@2023-10",
            "au-tas-spp-signs-code@2024-06",
        ] {
            assert!(
                packs[r].playback.min_dwell_s.is_empty(),
                "{r} sets no dwell"
            );
        }
    }

    #[test]
    fn dwell_bands_decode_a_number_or_a_list() {
        let n: DwellBands = serde_json::from_str("45").unwrap();
        assert_eq!(n, DwellBands(vec![SpeedBand::of(45.0)]));
        assert!(!n.by_speed());
        let l: DwellBands = serde_json::from_str(
            r#"[{"speed_kmh_below":80,"value":10},{"speed_kmh_from":80,"value":25,"note":"n"}]"#,
        )
        .unwrap();
        assert!(l.by_speed());
        assert_eq!(l.0[0].describe(), "below 80 km/h");
        assert_eq!(l.0[1].describe(), "at 80 km/h and over");
        let both = SpeedBand {
            speed_kmh_from: Some(80.0),
            speed_kmh_below: Some(100.5),
            ..SpeedBand::of(1.0)
        };
        assert_eq!(both.describe(), "from 80 to below 100.5 km/h");
        assert!(serde_json::from_str::<DwellBands>("\"x\"").is_err());
    }

    #[test]
    fn versions_of_lists_known_refs_in_order() {
        let packs = builtin_rule_packs();
        assert_eq!(
            versions_of(packs, "au-nsw-transport-corridor"),
            ["au-nsw-transport-corridor@2017-11"]
        );
        assert!(versions_of(packs, "au-nt-roadside").is_empty());
    }

    #[test]
    fn roadside_fixture_matches_schema() {
        let raw = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/roadside.screens.json"),
        )
        .unwrap();
        let res = lint(&raw, None, &SchemaOptions::default()).unwrap();
        assert!(res.ok(), "{res:?}");
    }
}
