//! A property test of strictest requirements: for random documents of
//! several screens, a file that exactly meets the strictest result must pass
//! `validate` with no error on every one of the screens.

use crate::check::Severity;
use crate::load::decode_document;
use crate::model::*;
use crate::report::{Input, Options, validate};
use crate::strictest::Strictest;
use serde_json::{Value, json};

/// A small deterministic generator, so a failure repeats.
struct Rng(u64);

impl Rng {
    fn n(&mut self, m: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % m
    }

    fn b(&mut self) -> bool {
        self.n(2) == 0
    }
}

const PACK: &str = "au-nsw-transport-corridor@2017-11";

fn gen_doc(r: &mut Rng) -> (Value, usize) {
    // image/gif is a type either list may hold, and both may at once.
    let types_s = ["image/png", "image/gif", "video/mp4"];
    let types_m = ["video/mp4", "image/gif"];
    let pick = |r: &mut Rng, xs: &[&str]| -> Vec<String> {
        xs.iter()
            .filter(|_| r.n(3) != 0)
            .map(|s| s.to_string())
            .collect()
    };
    let nscreens = 2 + r.n(4) as usize;
    let mut screens = vec![];
    for i in 0..nscreens {
        let mut s = json!({"id": format!("S{i}"), "format": "f"});
        if r.b() {
            s["viewing"] = json!({"distance_m": [1 + r.n(3), 4 + r.n(8)]});
            if r.b() {
                s["viewing"]["dwell_s"] = json!(1 + r.n(6));
            }
        }
        if r.n(3) == 0 {
            s["placement"] = json!({"motion_allowed": r.b()});
        }
        let mut ov = json!({});
        if r.b() {
            ov["physical_mm"] = json!([500 + r.n(1000), 1000]);
        }
        if r.b() {
            ov["safe_area"] =
                json!({"top": r.n(120), "right": r.n(80), "bottom": r.n(120), "left": r.n(80)});
        }
        if r.b() {
            ov["blocked_regions"] = json!([{"name": format!("br{i}"),
                "rect_px": [r.n(900), r.n(1800), 20 + r.n(100), 20 + r.n(100)]}]);
        }
        if r.b() {
            let (st, mo) = (pick(r, &types_s), pick(r, &types_m));
            // The slot is the same everywhere but now and then, which no one
            // file can match (the result says so).
            let slot = if r.n(8) == 0 { 12 } else { 10 };
            ov["media"] = json!({"static": st, "motion": mo, "slot_s": slot,
                "max_bytes": 500 + r.n(1000), "fps": 25 + r.n(6)});
        }
        if ov != json!({}) {
            s["overrides"] = ov;
        }
        let mut rules = json!({});
        if r.n(3) == 0 {
            rules["max_words"] = json!(2 + r.n(8));
        }
        if r.n(3) == 0 {
            rules["legibility"] =
                json!({"min_cap_height_mm": 5 + r.n(30), "min_legal_cap_height_mm": 3 + r.n(10)});
        }
        if r.n(3) == 0 {
            rules["motion"] = json!({"max_flash_hz": r.n(4), "allowed": r.n(4) != 0});
        }
        if r.n(4) == 0 {
            rules["motion"] = json!({"no_flashing": true});
        }
        if r.n(3) == 0 {
            rules["required_elements"] = json!([{"role": "logo", "min_cap_height_mm": 5 + r.n(20),
                "within_rect_px": [r.n(300), r.n(300), 300 + r.n(700), 300 + r.n(1000)]}]);
        }
        if r.n(4) == 0 {
            rules["required_elements"] =
                json!([{"role": "legal", "min_cap_height_px": 10 + r.n(20)}]);
        }
        if r.n(4) == 0 {
            // Only for creative tagged offer: the result lists what it asks.
            let mut e = json!({"role": "promo", "when": "offer"});
            if r.b() {
                e["min_cap_height_mm"] = json!(5 + r.n(20));
            } else {
                e["min_cap_height_px"] = json!(5 + r.n(30));
            }
            if r.b() {
                e["within_rect_px"] = json!([r.n(300), r.n(300), 300 + r.n(700), 300 + r.n(1000)]);
            }
            let list = rules["required_elements"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            rules["required_elements"] = json!([list, vec![e]].concat());
        }
        if r.n(4) == 0 {
            rules["rule_packs"] = json!([PACK]);
            s["placement"] = json!({"speed_kmh": 40 + r.n(60), "motion_allowed": true});
            if s.get("viewing").is_none() {
                s["viewing"] = json!({"distance_m": [20, 150]});
            }
        }
        if rules != json!({}) {
            s["rules"] = rules;
        }
        screens.push(s);
    }
    let mut fr = json!({"legibility": {"ratio": 100 + r.n(100)}});
    if r.b() {
        fr = json!({});
    }
    let fs = pick(r, &types_s);
    let fm = pick(r, &types_m);
    let mut format = json!({"id": "f", "pixels": [1000, 2000], "physical_mm": [1000, 2000],
        "media": {"static": fs, "motion": fm, "slot_s": 10, "max_bytes": 2000, "fps": 30, "codecs": ["h264"]}});
    if r.b() {
        format["safe_area"] =
            json!({"top": r.n(100), "right": r.n(50), "bottom": r.n(100), "left": r.n(50)});
    }
    let doc = json!({
      "opencrs": "0.1", "publisher": "gen.example", "updated_at": "2026-10-20T09:00:00+11:00",
      "expires_at": "2027-06-30T00:00:00+10:00",
      "networks": [{"id": "n", "name": "n", "rules": fr, "formats": [format], "screens": screens}]});
    (doc, nscreens)
}

fn rect4(v: &Value) -> [i64; 4] {
    let g = |i: usize| v[i].as_i64().unwrap();
    [g(0), g(1), g(2), g(3)]
}

/// A 20 x 20 spot inside the safe area and `within`, off every blocked region.
fn find_spot(out: &Value, within: Option<[i64; 4]>) -> Option<[i64; 4]> {
    let sa = out.get("safe_area");
    let g = |k: &str| sa.and_then(|s| s[k].as_i64()).unwrap_or(0);
    let (mut x0, mut y0, mut x1, mut y1) =
        (g("left"), g("top"), 1000 - g("right"), 2000 - g("bottom"));
    let regs: Vec<[i64; 4]> = out["blocked_regions"]
        .as_array()
        .map(|a| a.iter().map(|r| rect4(&r["rect_px"])).collect())
        .unwrap_or_default();
    if let Some(w) = within {
        x0 = x0.max(w[0]);
        y0 = y0.max(w[1]);
        x1 = x1.min(w[0] + w[2]);
        y1 = y1.min(w[1] + w[3]);
    }
    let (w, h) = (20, 20);
    let mut y = y0;
    while y + h <= y1 {
        let mut x = x0;
        while x + w <= x1 {
            let hit =
                |r: &[i64; 4]| x < r[0] + r[2] && x + w > r[0] && y < r[1] + r[3] && y + h > r[1];
            if !regs.iter().any(hit) {
                return Some([x, y, w, h]);
            }
            x += 10;
        }
        y += 10;
    }
    None
}

fn has_note(out: &Value, text: &str) -> bool {
    out["notes"].as_array().is_some_and(|n| {
        n.iter()
            .any(|n| n.as_str().is_some_and(|n| n.contains(text)))
    })
}

#[test]
fn strictest_file_passes_every_screen() {
    let mut r = Rng(12345);
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z").unwrap();
    let (mut tried, mut built) = (0, 0);
    for case in 0..2000 {
        let (doc, n) = gen_doc(&mut r);
        let raw = serde_json::to_vec(&doc).unwrap();
        let Ok(d) = decode_document(&raw) else {
            continue;
        };
        let Ok(s) = Strictest::from_text(&raw, "gen.example/n/f") else {
            continue;
        };
        let out: Value = serde_json::from_slice(&s.to_json()).unwrap();
        tried += 1;
        for motion in [false, true] {
            let list = if motion {
                &out["media"]["motion"]
            } else {
                &out["media"]["static"]
            };
            let Some(mt) = list
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
            else {
                continue;
            };
            // Slot lengths that differ leave no motion file that passes, and
            // the result says so.
            if motion && has_note(&out, "different slot lengths") {
                continue;
            }
            // Roles with a size, and those with a place to be.
            let required = out["rules"]["required_elements"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut caps: Vec<(String, i64)> = out["min_cap_height_px"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_i64().unwrap()))
                        .collect()
                })
                .unwrap_or_default();
            let mut want: Vec<(String, Option<[i64; 4]>)> = Vec::new();
            for e in &required {
                let role = e["role"].as_str().unwrap().to_string();
                want.push((
                    role.clone(),
                    e["within_rect_px"]
                        .as_array()
                        .map(|_| rect4(&e["within_rect_px"])),
                ));
                if !caps.iter().any(|(k, _)| *k == role) {
                    caps.push((role, 0));
                }
            }
            // What creative tagged for a condition must also meet.
            let mut tags = Vec::new();
            for c in out["conditional_elements_left_out"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                tags.push(c["when"].as_str().unwrap().to_string());
                let role = c["role"].as_str().unwrap().to_string();
                caps.push((role.clone(), c["min_cap_height_px"].as_i64().unwrap_or(0)));
                want.push((
                    role,
                    c["within_rect_px"]
                        .as_array()
                        .map(|_| rect4(&c["within_rect_px"])),
                ));
            }
            let mut els = vec![];
            let mut ok = true;
            for (role, px) in &caps {
                let within = want.iter().find(|(r, _)| r == role).and_then(|(_, w)| *w);
                match find_spot(&out, within) {
                    Some(rect) => els.push(Element {
                        role: role.clone(),
                        text: String::new(),
                        rect_px: Some(Rect(rect)),
                        cap_height_px: Some(*px as f64),
                    }),
                    None => ok = false,
                }
            }
            if !ok {
                continue;
            }
            let words = out["rules"]["max_words"].as_i64();
            let text = |w: i64| vec!["w"; w as usize].join(" ");
            if let Some(h) = els.iter_mut().find(|e| e.role == "headline") {
                if let Some(w) = words {
                    h.text = text(w);
                }
            } else if let Some(w) = words
                && let Some(rect) = find_spot(&out, None)
            {
                els.push(Element {
                    role: "headline".into(),
                    text: text(w),
                    rect_px: Some(Rect(rect)),
                    cap_height_px: Some(1000.0),
                });
            }
            built += 1;
            let f = CreativeFile {
                file: "x".into(),
                media_type: mt.into(),
                pixels: Some([1000, 2000]),
                bytes: Some(out["media"]["max_bytes"].as_i64().unwrap_or(1)),
                duration_s: motion.then(|| out["media"]["slot_s"].as_f64().unwrap_or(10.0)),
                max_flash_hz: motion.then_some(
                    out["rules"]["motion"]["max_flash_hz"]
                        .as_f64()
                        .unwrap_or(0.0),
                ),
                tags: (!tags.is_empty()).then_some(tags),
                targets: vec![],
                elements: els,
                ..Default::default()
            };
            for i in 0..n {
                let target = format!("gen.example/n/screen/S{i}");
                let input = Input {
                    file: &f,
                    campaign: None,
                    probe: None,
                };
                let rep = validate(&d, &input, &target, &Options::new(now)).unwrap();
                let errs: Vec<_> = rep
                    .checks
                    .iter()
                    .filter(|c| c.severity == Severity::Error)
                    .collect();
                assert!(
                    errs.is_empty(),
                    "case {case} motion={motion} screen S{i}\nDOC {doc}\nSTRICTEST {out}\nFILE {f:?}\nERRS {:#?}",
                    errs.iter()
                        .map(|c| (&c.code, &c.message))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    eprintln!("documents {tried}, files {built}");
    assert!(
        tried > 1500 && built > 800,
        "documents {tried}, files {built}"
    );
}
