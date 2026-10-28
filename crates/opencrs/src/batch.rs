//! Checking a manifest against documents, linting a single document, and
//! the requirements view. Port of `crs/batch.go`.

use crate::check::{Check, Severity};
use crate::document::check_document_text;
use crate::error::Error;
use crate::legibility::TextClass;
use crate::model::{CreativeFile, Document, Insets, Manifest, Media, Region, Segment, Viewing};
use crate::probe::Probe;
use crate::report::{Input, Options, Report, ValidateError, validate};
use crate::resolve::parse_ref;
use crate::resolve::{Requirements, Rules};
use crate::schema::{Kind, SchemaError, SchemaOptions, detect_kind, validate_schema};
use serde::{Serialize, Serializer};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// Checks every file in a manifest against every target that one of `docs`
/// describes. `probes` holds what could be read from each file, keyed by its
/// manifest path; a file without a probe is checked on the manifest's
/// declared values. The notes explain targets no document describes, which
/// are skipped rather than failed.
pub fn check_manifest(
    docs: &[Document],
    m: &Manifest,
    probes: &HashMap<String, Probe>,
    opt: &Options,
) -> Result<(Vec<Report>, Vec<String>), Error> {
    let notes = manifest_notes(docs, m)?;
    let mut reports = Vec::new();
    check_manifest_each(docs, m, probes, opt, |r| reports.push(r))?;
    Ok((reports, notes))
}

/// What [`check_manifest`] would say about skipped targets, found without
/// checking anything. Also the up-front test that every target is a
/// reference: when this returns `Ok`, [`check_manifest_each`] cannot fail,
/// so a caller can stream the reports knowing none will be withdrawn.
pub fn manifest_notes(docs: &[Document], m: &Manifest) -> Result<Vec<String>, Error> {
    let mut notes = Vec::new();
    if docs.is_empty() {
        // Nothing describes any target.
        for f in &m.files {
            for target in &f.targets {
                notes.push(skipped(f, target));
            }
        }
        return Ok(notes);
    }
    for f in &m.files {
        for target in &f.targets {
            let r = parse_ref(target).map_err(|e| {
                Error::msg(format!(
                    "{}: {}",
                    f.file,
                    ValidateError::BadTarget(e.to_string())
                ))
            })?;
            if !docs.iter().any(|d| d.publisher == r.publisher) {
                notes.push(skipped(f, target));
            }
        }
    }
    Ok(notes)
}

fn skipped(f: &CreativeFile, target: &str) -> String {
    format!(
        "{} targets {target}, which none of the given specs describe; skipped",
        f.file
    )
}

/// How many (file, target) pairs are checked, then handed to the caller,
/// at a time. Bounds the reports held in memory when streaming.
const CHUNK: usize = 2048;

/// [`check_manifest`], handing each report to `sink` in the same order
/// instead of collecting them. With the `parallel` feature the pairs of a
/// chunk are checked on all cores; the order, and so the output, is the same
/// either way.
pub fn check_manifest_each(
    docs: &[Document],
    m: &Manifest,
    probes: &HashMap<String, Probe>,
    opt: &Options,
    mut sink: impl FnMut(Report),
) -> Result<(), Error> {
    // One unit of work per (file, target) pair.
    let pairs: Vec<(&CreativeFile, &String)> = m
        .files
        .iter()
        .flat_map(|f| f.targets.iter().map(move |t| (f, t)))
        .collect();
    let one = |&(f, target): &(&CreativeFile, &String)| -> Result<Vec<Report>, Error> {
        let input = Input {
            file: f,
            campaign: m.campaign.as_ref(),
            probe: probes.get(&f.file),
        };
        let mut reports = Vec::new();
        for d in docs {
            match validate(d, &input, target, opt) {
                Ok(rep) => reports.push(rep),
                Err(ValidateError::OtherPublisher(_)) => {}
                Err(e) => return Err(Error::msg(format!("{}: {e}", f.file))),
            }
        }
        Ok(reports)
    };
    for chunk in pairs.chunks(CHUNK) {
        #[cfg(feature = "parallel")]
        let results: Vec<Result<Vec<Report>, Error>> = {
            use rayon::prelude::*;
            chunk.par_iter().map(one).collect()
        };
        #[cfg(not(feature = "parallel"))]
        let results: Vec<Result<Vec<Report>, Error>> = chunk.iter().map(one).collect();
        for r in results {
            r?.into_iter().for_each(&mut sink);
        }
    }
    Ok(())
}

fn serialize_kind<S: Serializer>(k: &Kind, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(k.as_str())
}

/// What's wrong with one OpenCRS document.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LintResult {
    #[serde(serialize_with = "serialize_kind")]
    pub kind: Kind,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub schema_errors: Vec<SchemaError>,
    /// A screens.json's own problems, found only once it matches the
    /// schema: broken references and rule conflicts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub document_checks: Vec<Check>,
}

impl LintResult {
    /// Whether the document has no errors. Warnings, such as a rule pack
    /// this validator doesn't know, don't count.
    pub fn ok(&self) -> bool {
        self.schema_errors.is_empty()
            && self
                .document_checks
                .iter()
                .all(|c| c.severity != Severity::Error)
    }
}

/// Checks a document against its schema (detecting the kind unless one is
/// given) and, for a valid screens.json, runs `check_document`. An error
/// means the document couldn't be read at all.
pub fn lint(raw: &[u8], kind: Option<Kind>, opt: &SchemaOptions) -> Result<LintResult, Error> {
    let kind = match kind {
        Some(k) => k,
        None => detect_kind(raw)?,
    };
    let errs = validate_schema(kind, raw, opt)?;
    let mut res = LintResult {
        kind,
        schema_errors: errs,
        document_checks: Vec::new(),
    };
    if res.schema_errors.is_empty() && kind == Kind::Screens {
        // A list of screens documents has no single document to check. The
        // checks read the text a screen at a time, so a very large document
        // is never decoded whole.
        if let Some(checks) = std::str::from_utf8(raw).ok().and_then(check_document_text) {
            res.document_checks = checks;
        }
    }
    Ok(res)
}

/// A JSON-friendly summary of one screen's resolved requirements, with
/// derived minimum text sizes and word limit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequirementsView {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub screen: String,
    pub format: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub venue: String,
    pub pixels: [i64; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_mm: Option<[f64; 2]>,
    pub media: Media,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe_area: Option<Insets>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocked_regions: Vec<Region>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<Segment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewing: Option<Viewing>,
    pub rules: Rules,
    /// Keys sort, as Go's map encoding does.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub derived: BTreeMap<String, Value>,
}

impl Requirements<'_> {
    /// Summarises the requirements for display.
    pub fn view(&self) -> RequirementsView {
        let mut derived = BTreeMap::new();
        for (name, class) in [
            ("min_key_cap_height", TextClass::Key),
            ("min_legal_cap_height", TextClass::Legal),
        ] {
            let (m, applies, missing) = self.min_cap_height(class);
            if applies && missing.is_empty() {
                // Keys are written in sorted order, as Go's map encoding sorts them.
                derived.insert(
                    name.to_string(),
                    json!({
                        "basis": m.basis,
                        "mm": ((m.mm * 10.0 + 0.5) as i64) as f64 / 10.0,
                        "px": m.px,
                    }),
                );
            }
        }
        if let Some((n, basis)) = self.word_limit() {
            derived.insert("max_words".into(), json!({"basis": basis, "value": n}));
        }
        RequirementsView {
            screen: self.screen_id().to_string(),
            format: self.format_id.to_string(),
            venue: self.venue.map_or_else(String::new, |v| v.id.clone()),
            pixels: self.pixels,
            physical_mm: self.physical_mm,
            media: self.media.clone(),
            safe_area: self.safe_area,
            blocked_regions: self.blocked_regions.clone(),
            segments: self.segments.clone(),
            viewing: self.screen.and_then(|s| s.viewing.clone()),
            rules: self.rules.clone(),
            derived,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::CODE_REFERENCE;
    use crate::testutil::{harbourline, test_now};

    fn example(rel: &str) -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel);
        std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    #[test]
    fn lint() {
        let opt = SchemaOptions::default();
        let res = super::lint(
            &example("examples/harbourline/retail.screens.json"),
            None,
            &opt,
        )
        .unwrap();
        assert_eq!(res.kind, Kind::Screens);
        assert!(res.ok(), "{res:?}");
        let broken = r#"{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-10-20T09:00:00Z","networks":[{"id":"n","name":"N",
	  "formats":[{"id":"f","pixels":[1,1],"media":{"static":["image/png"]}}],"screens":[{"id":"s","format":"g"}]}]}"#;
        let res = super::lint(broken.as_bytes(), None, &opt).unwrap();
        assert!(!res.ok());
        assert_eq!(res.document_checks.len(), 1);
        assert_eq!(res.document_checks[0].code, CODE_REFERENCE);
        assert!(super::lint(br#"{"hello":1}"#, None, &opt).is_err());
        // A rule pack the validator doesn't know is a warning, not an error.
        let typo = r#"{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-10-20T09:00:00Z","networks":[{"id":"n","name":"N",
	  "rules":{"rule_packs":["au-nsw-transport-corridor@2017-12"]},
	  "formats":[{"id":"f","pixels":[1,1],"media":{"static":["image/png"]}}],"screens":[{"id":"s","format":"f"}]}]}"#;
        let res = super::lint(typo.as_bytes(), None, &opt).unwrap();
        assert!(res.ok(), "{res:?}");
        assert_eq!(res.document_checks.len(), 1);
        let c = &res.document_checks[0];
        assert_eq!(
            (c.code.as_str(), c.severity),
            ("CRS-RULE-PACK", Severity::Warning)
        );
        assert_eq!(
            c.message,
            "Rule pack au-nsw-transport-corridor@2017-12 isn't known to this validator, so creative won't be checked against it (known: au-nsw-transport-corridor@2017-11)"
        );
        // The pack is in the network's rules, so the finding is the network's,
        // reported once, not the screen's.
        assert_eq!(
            (c.network.as_str(), c.venue.as_str(), c.format.as_str()),
            ("n", "", "")
        );
        assert!(c.screens.is_empty());
    }

    fn harbourline_manifest() -> Manifest {
        crate::decode_manifest(&example("examples/harbourline/creative.json")).unwrap()
    }

    #[test]
    fn check_manifest_probes_and_skips() {
        let mut m = harbourline_manifest();
        m.files[0]
            .targets
            .push("other.example/retail/portrait-totem-75".into());
        let png = crate::probe::probe_bytes(&example("examples/harbourline/lunch-portrait.png"));
        let mut probes = HashMap::new();
        probes.insert("lunch-portrait.png".to_string(), png.clone());
        let opt = Options::new(test_now());
        let (reports, notes) = super::check_manifest(&[harbourline()], &m, &probes, &opt).unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(
            notes,
            [
                "lunch-portrait.png targets other.example/retail/portrait-totem-75, which none of the given specs describe; skipped"
            ]
        );
        assert_eq!(reports[0].file_sha256, png.sha256);
        assert_eq!(
            reports[1].file_sha256, "",
            "the unprobed file cites no hash"
        );
    }

    #[test]
    fn harbourline_reports_match_fixture() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T12:00:00+11:00").unwrap();
        let mut probes = HashMap::new();
        for name in ["lunch-portrait.png", "lunch-portrait-logo-right.png"] {
            let bytes = example(&format!("examples/harbourline/{name}"));
            probes.insert(name.to_string(), crate::probe::probe_bytes(&bytes));
        }
        let (reports, notes) = super::check_manifest(
            &[harbourline()],
            &harbourline_manifest(),
            &probes,
            &Options::new(now),
        )
        .unwrap();
        assert!(notes.is_empty());
        let got = serde_json::to_value(&reports).unwrap();
        let want: Value =
            serde_json::from_slice(&example("examples/harbourline/reports.json")).unwrap();
        assert_eq!(got, want);
    }

    /// The network `bench/gen-network.py` writes (20,000 screens over 400
    /// formats), with one manifest file per format aimed at its screens.
    fn bench_network() -> (Document, Manifest) {
        const N: usize = 20_000;
        const F: usize = 400;
        let sizes = [
            (1080, 1920),
            (1920, 1080),
            (3840, 2160),
            (1080, 1080),
            (5760, 1080),
            (768, 1280),
            (2160, 3840),
        ];
        let formats: Vec<Value> = (0..F)
            .map(|i| {
                let (w, h) = sizes[i % sizes.len()];
                json!({"id": format!("f{i}"), "pixels": [w, h], "physical_mm": [w / 2, h / 2],
                    "media": {"static": ["image/png", "image/jpeg"], "motion": ["video/mp4"],
                              "slot_s": 10, "fps": 30, "max_bytes": 52_428_800},
                    "safe_area": {"top": 60, "right": 40, "bottom": 60, "left": 40},
                    "blocked_regions": [{"name": "logo", "rect_px": [w - 200, 20, 160, 80]}]})
            })
            .collect();
        let screens: Vec<Value> = (0..N)
            .map(|s| {
                let mut sc = json!({"id": format!("S-{s:05}"), "format": format!("f{}", s % F),
                    "venue": "v1",
                    "viewing": {"distance_m": [2, 2 + s % 20], "dwell_s": 4, "light": "indoor"}});
                if s % 7 == 0 {
                    sc["overrides"] = json!({"blocked_regions": [{"name": "column", "rect_px": [0, 100, 50, 50]}]});
                }
                sc
            })
            .collect();
        let doc = json!({"opencrs": "0.1", "publisher": "network.example",
            "updated_at": "2026-10-20T09:00:00+11:00", "expires_at": "2027-01-20T00:00:00+11:00",
            "networks": [{"id": "net", "name": "Big", "rules": {"legibility": {"ratio": 150}},
                "venues": [{"id": "v1", "name": "V", "venue_type": 205}],
                "formats": formats, "screens": screens}]});
        let files: Vec<Value> = (0..F)
            .map(|i| {
                let targets: Vec<String> = (0..N / F)
                    .map(|k| format!("network.example/net/screen/S-{:05}", i + F * k))
                    .collect();
                json!({"file": format!("c{i}.png"), "media_type": "image/png", "pixels": [1080, 1920],
                    "targets": targets,
                    "elements": [
                        {"role": "headline", "text": "Hi", "rect_px": [90, 210, 700, 260], "cap_height_px": 92},
                        {"role": "logo", "rect_px": [90, 160, 260, 60]}]})
            })
            .collect();
        let manifest = json!({"opencrs": "0.1",
            "campaign": {"id": "c", "advertiser": "A", "category": "IAB8"}, "files": files});
        (
            crate::decode_document(doc.to_string().as_bytes()).unwrap(),
            crate::decode_manifest(manifest.to_string().as_bytes()).unwrap(),
        )
    }

    #[test]
    fn streaming_matches_collecting() {
        let (doc, m) = bench_network();
        let opt = Options::new(test_now());
        let probes = HashMap::new();
        let (all, notes) =
            super::check_manifest(std::slice::from_ref(&doc), &m, &probes, &opt).unwrap();
        assert_eq!(all.len(), 20_000);
        assert!(notes.is_empty());
        let mut streamed = Vec::new();
        super::check_manifest_each(std::slice::from_ref(&doc), &m, &probes, &opt, |r| {
            streamed.push(r)
        })
        .unwrap();
        assert_eq!(all, streamed);
    }

    /// Output order and content don't depend on the number of threads.
    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_order_matches_serial() {
        let (doc, m) = bench_network();
        let opt = Options::new(test_now());
        let probes = HashMap::new();
        let run = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    super::check_manifest(std::slice::from_ref(&doc), &m, &probes, &opt).unwrap()
                })
        };
        let serial = run(1);
        let parallel = run(0); // 0: all cores
        assert_eq!(serial.0.len(), 20_000);
        assert_eq!(serial, parallel);
        let order: Vec<&str> = serial.0.iter().map(|r| r.target.as_str()).collect();
        let want: Vec<&str> = m
            .files
            .iter()
            .flat_map(|f| f.targets.iter().map(String::as_str))
            .collect();
        assert_eq!(order, want);
    }

    #[test]
    fn view() {
        let d = harbourline();
        let reqs = d
            .resolve(
                &crate::resolve::parse_ref("harbourline.example/retail/screen/HC-L1-002").unwrap(),
            )
            .unwrap();
        let v = reqs[0].view();
        assert_eq!(v.screen, "HC-L1-002");
        assert_eq!(v.venue, "harbour-central");
        assert_eq!(v.blocked_regions.len(), 2);
        assert_eq!(
            v.derived["min_legal_cap_height"]["px"], 23,
            "{:?}",
            v.derived
        );
        let out = crate::json::to_string(&v);
        assert!(out.contains(r#""derived":{"max_words":"#), "{out}");
    }
}
