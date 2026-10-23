//! Shared fixtures for tests that run `validate`.

use crate::check::{Check, Severity};
use crate::load::decode_document;
use crate::model::{Campaign, CreativeFile, Document, Element, Rect};
use crate::report::{Input, Options, Report, validate};
use chrono::{DateTime, FixedOffset};

pub(crate) fn test_now() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z").unwrap()
}

fn read(rel: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

pub(crate) fn harbourline() -> Document {
    decode_document(&read("../../examples/harbourline/retail.screens.json")).unwrap()
}

pub(crate) fn element(role: &str, text: &str, rect: Option<[i64; 4]>, cap: Option<f64>) -> Element {
    Element {
        role: role.into(),
        text: text.into(),
        rect_px: rect.map(Rect),
        cap_height_px: cap,
    }
}

pub(crate) fn lunch_file() -> CreativeFile {
    CreativeFile {
        file: "lunch-portrait.png".into(),
        media_type: "image/png".into(),
        pixels: Some([1080, 1920]),
        bytes: Some(2_400_000),
        targets: vec!["harbourline.example/retail/portrait-totem-75".into()],
        elements: vec![
            element(
                "headline",
                "Lunch specials.",
                Some([90, 210, 700, 260]),
                Some(92.0),
            ),
            element(
                "legal",
                "Weekdays 11:30 to 2:30",
                Some([90, 1640, 520, 40]),
                Some(18.0),
            ),
            element("logo", "", Some([90, 160, 260, 60]), None),
        ],
        ..Default::default()
    }
}

pub(crate) fn offer_campaign() -> Campaign {
    Campaign {
        category: "IAB8".into(),
        tags: Some(vec!["offer".into()]),
        ..Default::default()
    }
}

/// Validates against the Harbourline document at the fixed test time.
pub(crate) fn check(f: &CreativeFile, c: Option<&Campaign>, target: &str) -> Report {
    check_doc(&harbourline(), f, c, target)
}

pub(crate) fn check_doc(
    d: &Document,
    f: &CreativeFile,
    c: Option<&Campaign>,
    target: &str,
) -> Report {
    let input = Input {
        file: f,
        campaign: c,
        probe: None,
    };
    validate(d, &input, target, &Options::new(test_now())).unwrap()
}

pub(crate) fn find<'a>(rep: &'a Report, code: &str, sev: Severity) -> Vec<&'a Check> {
    rep.checks
        .iter()
        .filter(|c| c.code == code && c.severity == sev)
        .collect()
}
