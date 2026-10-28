//! Adversarial shapes: documents that are small but make the validator work
//! hard. Shapes that should be linear stay well inside the work budget (and
//! run in moments); shapes built to exceed it stop with CRS-TOO-COMPLEX and a
//! reason that names where the work came from.

use opencrs::batch::{check_manifest, lint};
use opencrs::budget::{self, Budget};
use opencrs::check::CODE_TOO_COMPLEX;
use opencrs::report::Options;
use opencrs::resolve::parse_ref;
use opencrs::{SchemaOptions, decode_document, decode_manifest};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Instant;

fn base() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/harbourline/retail.screens.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn elements(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({"role": format!("r{i}"), "min_cap_height_mm": 1}))
        .collect()
}

fn packs(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("p{i}@1")).collect()
}

/// A document whose network has `rules`, and `screens` screens on one format;
/// with `viewing`, each screen has its own viewing so none share a check.
fn doc(rules: Value, screens: usize, viewing: bool) -> Vec<u8> {
    let mut v = base();
    let fmt = v["networks"][0]["screens"][0]["format"].clone();
    v["networks"][0]["rules"] = rules;
    v["networks"][0]["screens"] = (0..screens)
        .map(|i| {
            let mut s = json!({"id": format!("s{i}"), "format": fmt});
            if viewing {
                s["viewing"] = json!({"distance_m": [1, 2 + i % 1000], "dwell_s": 1 + i % 50});
                s["placement"] = json!({"zone": format!("z{i}")});
            }
            s
        })
        .collect();
    serde_json::to_vec(&v).unwrap()
}

const MANIFEST: &str = r#"{"opencrs":"0.1","campaign":{"id":"c","advertiser":"A","category":"IAB8"},
  "files":[{"file":"a.png","media_type":"image/png","pixels":[1080,1920],"targets":["TARGET"]}]}"#;

struct Run {
    tripped: Vec<String>,
    used: u64,
    limit: u64,
    secs: f64,
}

fn lint_run(raw: &[u8]) -> Run {
    let t = Instant::now();
    let opt = SchemaOptions::default();
    let (res, b) = budget::measure(raw.len(), || lint(raw, None, &opt).unwrap());
    let tripped = res
        .document_checks
        .iter()
        .filter(|c| c.code == CODE_TOO_COMPLEX)
        .map(|c| c.message.clone())
        .collect();
    Run {
        tripped,
        used: b.used(),
        limit: b.limit(),
        secs: t.elapsed().as_secs_f64(),
    }
}

fn check_run(raw: &[u8], targets: &[String]) -> Run {
    let t = Instant::now();
    let d = decode_document(raw).unwrap();
    let list = targets
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(",");
    let m = decode_manifest(MANIFEST.replace("\"TARGET\"", &list).as_bytes()).unwrap();
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z").unwrap();
    let ((reports, _), b) = budget::measure(raw.len() + m.size, || {
        check_manifest(&[d], &m, &HashMap::new(), &Options::new(now)).unwrap()
    });
    let tripped = reports
        .iter()
        .flat_map(|r| r.checks.iter())
        .filter(|c| c.code == CODE_TOO_COMPLEX)
        .map(|c| c.message.clone())
        .collect();
    Run {
        tripped,
        used: b.used(),
        limit: b.limit(),
        secs: t.elapsed().as_secs_f64(),
    }
}

fn format_target() -> String {
    let v = base();
    format!(
        "harbourline.example/retail/{}",
        v["networks"][0]["screens"][0]["format"].as_str().unwrap()
    )
}

/// Work stays a small part of the allowance, and quick, for a shape that
/// should be linear.
fn linear(name: &str, r: &Run) {
    assert!(r.tripped.is_empty(), "{name} tripped: {:?}", r.tripped);
    assert!(
        r.used * 4 < r.limit,
        "{name}: {} of {} units",
        r.used,
        r.limit
    );
    assert!(r.secs < 20.0, "{name} took {:.1}s", r.secs);
}

#[test]
fn linear_shapes_stay_inside_the_budget() {
    // Many rules, shared by every screen.
    let raw = doc(json!({"required_elements": elements(2000)}), 2000, false);
    linear("elements, equal screens, lint", &lint_run(&raw));
    linear(
        "elements, equal screens, check",
        &check_run(&raw, &[format_target()]),
    );
    // Many packs on the network.
    let raw = doc(json!({"rule_packs": packs(5000)}), 2000, false);
    linear("packs, lint", &lint_run(&raw));
    linear(
        "packs, check",
        &check_run(
            &doc(json!({"rule_packs": packs(1000)}), 2000, false),
            &[format_target()],
        ),
    );
    // A huge number of unknown packs and nothing else.
    let raw = doc(json!({"rule_packs": packs(20_000)}), 1, false);
    linear("many unknown packs, lint", &lint_run(&raw));
    // Many screens on one format.
    let raw = doc(json!({}), 8_000, true);
    linear("many screens, lint", &lint_run(&raw));
    linear("many screens, check", &check_run(&raw, &[format_target()]));
    // Duplicate ids.
    let mut v: Value = serde_json::from_slice(&doc(json!({}), 3000, true)).unwrap();
    for s in v["networks"][0]["screens"].as_array_mut().unwrap() {
        s["id"] = "same".into();
    }
    let raw = serde_json::to_vec(&v).unwrap();
    linear("duplicate ids, lint", &lint_run(&raw));
    linear("duplicate ids, check", &check_run(&raw, &[format_target()]));
    // Rules at every level: venues x formats, each with their own.
    let mut v: Value = serde_json::from_slice(&doc(json!({}), 0, false)).unwrap();
    let fmt = v["networks"][0]["formats"][0].clone();
    let n = &mut v["networks"][0];
    n["venues"] = (0..30).map(|i| json!({"id": format!("v{i}"), "name": "V", "rules": {"required_elements": elements(40)}})).collect();
    n["formats"] = (0..30)
        .map(|i| {
            let mut f = fmt.clone();
            f["id"] = format!("f{i}").into();
            f["rules"] = json!({"required_elements": elements(40)});
            f
        })
        .collect();
    n["screens"] = (0..3000)
        .map(|i| json!({"id": format!("s{i}"), "format": format!("f{}", i % 30), "venue": format!("v{}", i / 100)}))
        .collect();
    let raw = serde_json::to_vec(&v).unwrap();
    linear("venue x format rules, lint", &lint_run(&raw));
    linear(
        "venue x format rules, check",
        &check_run(&raw, &["harbourline.example/retail/f1".to_string()]),
    );
    // Several targets on a modest document.
    let raw = doc(json!({}), 1000, false);
    let targets: Vec<String> = (0..20).map(|_| format_target()).collect();
    linear("a few targets", &check_run(&raw, &targets));
}

fn resolve_run(raw: &[u8], target: &str) -> Run {
    let t = Instant::now();
    let d = decode_document(raw).unwrap();
    let rf = parse_ref(target).unwrap();
    // As the module's resolve does: a view for each screen as it comes,
    // none once the budget is spent, then the budget is asked why.
    let (why, b) = budget::measure(raw.len(), || {
        let mut views = 0usize;
        match d.resolve_each(&rf, |q| {
            if !budget::tripped() {
                q.view();
                views += 1;
            }
        }) {
            Err(e) => Some(e.to_string()),
            Ok(()) => budget::explain(Some(&d)),
        }
    });
    Run {
        tripped: why.into_iter().collect(),
        used: b.used(),
        limit: b.limit(),
        secs: t.elapsed().as_secs_f64(),
    }
}

/// A format's lists are not copied for each of its screens: a format of
/// many blocked regions on many screens is linear to lint, check and
/// resolve, and a screen whose overrides change the regions pays for the
/// rebuilt list.
#[test]
fn a_format_with_many_regions_and_many_screens_stays_linear() {
    let mut v = base();
    let fmt = v["networks"][0]["screens"][0]["format"].clone();
    let regions: Vec<Value> = (0..20_000)
        .map(|i| json!({"name": format!("r{i}"), "rect_px": [0, 0, 1, 1]}))
        .collect();
    v["networks"][0]["formats"][0]["blocked_regions"] = regions.into();
    v["networks"][0]["screens"] = (0..20_000)
        .map(|i| json!({"id": format!("s{i}"), "format": fmt}))
        .collect();
    let raw = serde_json::to_vec(&v).unwrap();
    linear("regions, lint", &lint_run(&raw));
    linear("regions, check", &check_run(&raw, &[format_target()]));
    // A view for each screen lists the regions again: that answer is
    // 20,000 times the document, and is stopped.
    let r = resolve_run(&raw, &format_target());
    assert!(!r.tripped.is_empty(), "used {} of {}", r.used, r.limit);
    assert!(r.tripped[0].contains("screen views"), "{}", r.tripped[0]);
    assert!(r.secs < 20.0, "stopped after {:.1}s", r.secs);
    // Each screen overriding one region has the list rebuilt for it:
    // 400 million region copies for a 2 MB document, which is stopped and
    // named.
    for s in v["networks"][0]["screens"].as_array_mut().unwrap() {
        s["overrides"] = json!({"blocked_regions": [{"name": "own", "rect_px": [1, 1, 1, 1]}]});
    }
    let raw = serde_json::to_vec(&v).unwrap();
    let r = lint_run(&raw);
    assert!(!r.tripped.is_empty(), "used {} of {}", r.used, r.limit);
    assert!(r.tripped[0].contains("blocked regions"), "{}", r.tripped[0]);
    assert!(
        r.tripped[0].contains("/networks/0/formats/0"),
        "{}",
        r.tripped[0]
    );
    assert!(r.secs < 20.0, "stopped after {:.1}s", r.secs);
}

/// A screen restating one element gets its own copy of the inherited list
/// to put it in: many such screens under a long list are stopped, and the
/// cost is named for the list.
#[test]
fn screens_restating_a_long_list_trip_the_guard() {
    let mut v: Value = serde_json::from_slice(&doc(
        json!({"required_elements": elements(3000)}),
        20_000,
        false,
    ))
    .unwrap();
    for s in v["networks"][0]["screens"].as_array_mut().unwrap() {
        s["rules"] = json!({"required_elements": [{"role": "own"}]});
    }
    let raw = serde_json::to_vec(&v).unwrap();
    let r = lint_run(&raw);
    assert!(!r.tripped.is_empty(), "used {} of {}", r.used, r.limit);
    assert!(
        r.tripped[0].contains("3,000 required elements"),
        "{}",
        r.tripped[0]
    );
    assert!(r.secs < 20.0, "stopped after {:.1}s", r.secs);
    let r = check_run(&raw, &[format_target()]);
    assert!(!r.tripped.is_empty(), "used {} of {}", r.used, r.limit);
    assert!(r.secs < 20.0, "stopped after {:.1}s", r.secs);
}

/// A format whose rules conflict with the network's on many elements has
/// those conflicts reported once, and its screens don't each go through
/// them: the shape is linear.
#[test]
fn a_format_with_many_conflicts_and_many_screens_stays_linear() {
    let el = |i: usize, rect: [i64; 4]| json!({"role": format!("r{i}"), "within_rect_px": rect});
    let net: Vec<Value> = (0..2000).map(|i| el(i, [0, 0, 10, 10])).collect();
    let fmt: Vec<Value> = (0..2000).map(|i| el(i, [20, 20, 10, 10])).collect();
    let mut v: Value =
        serde_json::from_slice(&doc(json!({"required_elements": net}), 20_000, false)).unwrap();
    v["networks"][0]["formats"][0]["rules"] = json!({"required_elements": fmt});
    let raw = serde_json::to_vec(&v).unwrap();
    let t = Instant::now();
    let opt = SchemaOptions::default();
    let (res, b) = budget::measure(raw.len(), || lint(&raw, None, &opt).unwrap());
    let conflicts = res
        .document_checks
        .iter()
        .filter(|c| c.code == "CRS-RULE-CONFLICT")
        .count();
    assert_eq!(conflicts, 2000, "{:?}", &res.document_checks[..3]);
    assert!(
        res.document_checks
            .iter()
            .all(|c| c.code != CODE_TOO_COMPLEX && c.screens.is_empty()),
        "{:?}",
        res.document_checks
            .iter()
            .find(|c| c.code == CODE_TOO_COMPLEX)
    );
    assert!(b.used() * 4 < b.limit(), "{} of {}", b.used(), b.limit());
    assert!(t.elapsed().as_secs_f64() < 20.0, "{:?}", t.elapsed());
    linear("conflicts, check", &check_run(&raw, &[format_target()]));
}

#[test]
fn rule_items_checked_for_many_different_screens_trip_the_guard() {
    // 2,000 required elements, 20,000 screens that each need their own check:
    // 40 million rule items to look at, for a document of a couple of MB.
    let raw = doc(json!({"required_elements": elements(2000)}), 5_000, true);
    let r = check_run(&raw, &[format_target()]);
    assert!(!r.tripped.is_empty(), "used {} of {}", r.used, r.limit);
    let why = &r.tripped[0];
    assert!(why.contains("too complex"), "{why}");
    assert!(why.contains("2,000 required elements"), "{why}");
    assert!(why.contains("/networks/0/rules/required_elements"), "{why}");
    assert!(why.contains("format `portrait-totem-75`"), "{why}");
    assert!(r.secs < 20.0, "stopped after {:.1}s", r.secs);
}

#[test]
fn rule_packs_checked_for_many_different_screens_trip_the_guard() {
    let raw = doc(json!({"rule_packs": packs(50_000)}), 5_000, true);
    let r = check_run(&raw, &[format_target()]);
    assert!(!r.tripped.is_empty(), "used {} of {}", r.used, r.limit);
    assert!(
        r.tripped[0].contains("50,000 rule packs"),
        "{}",
        r.tripped[0]
    );
    assert!(
        r.tripped[0].contains("/networks/0/rules/rule_packs"),
        "{}",
        r.tripped[0]
    );
}

#[test]
fn many_targets_on_a_big_format_trip_the_guard() {
    let raw = doc(json!({}), 1500, false);
    let targets: Vec<String> = (0..500).map(|_| format_target()).collect();
    let r = check_run(&raw, &targets);
    assert!(!r.tripped.is_empty(), "used {} of {}", r.used, r.limit);
    assert!(r.tripped[0].contains("visited"), "{}", r.tripped[0]);
    assert!(r.tripped[0].contains("format `"), "{}", r.tripped[0]);
}

#[test]
fn a_venue_with_a_huge_list_is_named() {
    // A venue listing 30,000 packs, and 3,000 screens there that each need
    // their own check: the reason names the venue's list and its path.
    let mut v: Value = serde_json::from_slice(&doc(json!({}), 3_000, true)).unwrap();
    v["networks"][0]["venues"][0]["rules"] = json!({"rule_packs": packs(30_000)});
    for s in v["networks"][0]["screens"].as_array_mut().unwrap() {
        s["venue"] = "harbour-central".into();
    }
    let raw = serde_json::to_vec(&v).unwrap();
    let r = check_run(&raw, &[format_target()]);
    assert!(!r.tripped.is_empty(), "used {} of {}", r.used, r.limit);
    assert!(
        r.tripped[0].contains("30,000 rule packs"),
        "{}",
        r.tripped[0]
    );
    assert!(
        r.tripped[0].contains("/networks/0/venues/0/rules/rule_packs"),
        "{}",
        r.tripped[0]
    );
}

#[test]
fn resolve_gives_the_reason_when_the_guard_trips() {
    let raw = doc(json!({"required_elements": elements(3000)}), 30_000, true);
    let d = decode_document(&raw).unwrap();
    let rf = parse_ref(&format_target()).unwrap();
    let (res, b) = budget::measure(raw.len(), || {
        d.resolve(&rf)
            .map(|reqs| reqs.iter().map(|q| q.view()).collect::<Vec<_>>().len())
    });
    // Resolving builds a view for every screen, 3,000 items each.
    assert!(
        b.tripped() || res.is_err(),
        "used {} of {}",
        b.used(),
        b.limit()
    );
    if let Err(e) = res {
        assert!(e.to_string().contains("too complex"), "{e}");
    }
}

#[test]
fn work_is_counted_the_same_each_time() {
    let raw = doc(json!({"required_elements": elements(300)}), 2000, true);
    let a = check_run(&raw, &[format_target()]);
    let b = check_run(&raw, &[format_target()]);
    assert_eq!(a.used, b.used);
    let _ = Budget::for_input(0);
}

/// Prints the work, as a multiple of the input's size, that real documents
/// take: the examples, the conformance cases and any files named in
/// `OPENCRS_RATIO_FILES` (colon separated). `UNITS_PER_BYTE` is chosen well
/// above the biggest. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn report_legit_ratios() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files: Vec<std::path::PathBuf> =
        vec![root.join("examples/harbourline/retail.screens.json")];
    for e in std::fs::read_dir(root.join("conformance/0.1/cases")).unwrap() {
        let p = e.unwrap().path().join("screens.json");
        if p.exists() {
            files.push(p);
        }
    }
    let extra = std::env::var("OPENCRS_RATIO_FILES").unwrap_or_default();
    files.extend(extra.split(':').filter(|s| !s.is_empty()).map(Into::into));
    let mut worst = 0f64;
    for f in files {
        let raw = std::fs::read(&f).unwrap();
        let Ok(d) = decode_document(&raw) else {
            continue;
        };
        let opt = SchemaOptions::default();
        let (_, lb) = budget::measure(raw.len(), || lint(&raw, None, &opt).unwrap());
        let n = &d.networks[0];
        let target = format!("{}/{}/{}", d.publisher, n.id, n.formats[0].id);
        let m = decode_manifest(
            MANIFEST
                .replace("\"TARGET\"", &format!("\"{target}\""))
                .as_bytes(),
        )
        .unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z").unwrap();
        let (_, cb) = budget::measure(raw.len() + m.size, || {
            check_manifest(
                std::slice::from_ref(&d),
                &m,
                &HashMap::new(),
                &Options::new(now),
            )
            .unwrap()
        });
        let rf = parse_ref(&target).unwrap();
        let (_, rb) = budget::measure(raw.len(), || {
            d.resolve(&rf)
                .map(|r| r.iter().map(|q| q.view()).collect::<Vec<_>>().len())
        });
        let ratio = |b: &Budget, extra: usize| b.used() as f64 / (raw.len() + extra) as f64;
        let r = [ratio(&lb, 0), ratio(&cb, m.size), ratio(&rb, 0)];
        worst = worst.max(r[0]).max(r[1]).max(r[2]);
        println!(
            "{:>9} bytes  lint {:6.2}  check {:6.2}  resolve {:6.2}  units/byte  {}",
            raw.len(),
            r[0],
            r[1],
            r[2],
            f.display()
        );
    }
    println!(
        "worst: {worst:.2} units per byte; allowed {}",
        budget::UNITS_PER_BYTE
    );
}

/// Prints the reason each attack shape is stopped with. Run with
/// `--ignored --nocapture`.
#[test]
#[ignore]
fn show_explanations() {
    let raw = doc(json!({"required_elements": elements(2000)}), 5_000, true);
    println!("{}\n", check_run(&raw, &[format_target()]).tripped[0]);
    let raw = doc(json!({"rule_packs": packs(50_000)}), 5_000, true);
    println!("{}\n", check_run(&raw, &[format_target()]).tripped[0]);
    let raw = doc(json!({}), 1500, false);
    let targets: Vec<String> = (0..500).map(|_| format_target()).collect();
    println!("{}", check_run(&raw, &targets).tripped[0]);
}
