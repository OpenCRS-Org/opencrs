use crate::cli::RulePacksArgs;
use crate::cmd_calc::num;
use opencrs::calc::pack_list;
use opencrs::json;
use opencrs::rulepack::{RulePack, builtin_rule_packs};
use serde_json::json;
use std::io::Write;

fn list(a: &RulePacksArgs, stdout: &mut dyn Write) -> i32 {
    let packs = pack_list();
    if a.json {
        // The shape the WebAssembly module and the bindings give.
        let out = serde_json::to_value(&packs).unwrap_or_default();
        let _ = stdout.write_all(json::to_string_pretty(&json!({ "rule_packs": out })).as_bytes());
        return 0;
    }
    let width = packs.iter().map(|p| p.reference.len()).max().unwrap_or(0);
    let status = packs.iter().map(|p| p.status.len()).max().unwrap_or(0);
    for p in &packs {
        let _ = writeln!(
            stdout,
            "{:<width$}  {:<status$}  {}",
            p.reference, p.status, p.title
        );
    }
    0
}

fn dwell(p: &RulePack) -> String {
    let bands = &p.playback.min_dwell_s;
    if !bands.by_speed() {
        return format!("{} s", num(bands.0[0].value));
    }
    bands
        .0
        .iter()
        .map(|b| format!("{} s {}", num(b.value), b.describe()))
        .collect::<Vec<_>>()
        .join("; ")
}

fn luminance(p: &RulePack, stdout: &mut dyn Write) {
    for (period, zones) in &p.playback.max_luminance_cd_m2 {
        let zones: Vec<String> = zones
            .iter()
            .map(|(z, v)| match v {
                Some(v) => format!("{z} {}", num(*v)),
                None => format!("{z} no limit"),
            })
            .collect();
        let _ = writeln!(stdout, "      {period:<9} {}", zones.join(", "));
    }
}

/// One labelled line of a pack.
fn row(stdout: &mut dyn Write, label: &str, value: String) {
    let _ = writeln!(stdout, "    {label:<15} {value}");
}

fn show(p: &RulePack, stdout: &mut dyn Write) {
    let _ = writeln!(stdout, "{} ({}): {}", p.reference(), p.status, p.title);
    if !p.status_note.is_empty() {
        let _ = writeln!(stdout, "  {}", p.status_note);
    }
    let _ = writeln!(stdout, "  source  {}", p.source_title);
    let _ = writeln!(stdout, "          {}", p.source_url);
    let _ = writeln!(stdout, "  creative");
    let c = &p.creative;
    if let Some(m) = &c.motion {
        let mut parts = Vec::new();
        match m.allowed {
            Some(true) => parts.push("allowed".to_string()),
            Some(false) => parts.push("not allowed".to_string()),
            None => {}
        }
        if m.no_flashing == Some(true) {
            parts.push("no flashing".into());
        }
        if let Some(hz) = m.max_flash_hz {
            parts.push(format!("at most {} flashes a second", num(hz)));
        }
        if let Some(s) = m.min_frame_hold_s {
            parts.push(format!("each frame held {} s or more", num(s)));
        }
        row(stdout, "motion", parts.join(", "));
    }
    for (on, what) in [
        (c.no_traffic_device_imagery, "no traffic-device imagery"),
        (
            c.max_words_at_glance,
            "words limited to what a glance reads",
        ),
        (c.no_sequencing, "no sequencing"),
        (c.no_device_interaction, "no device interaction"),
    ] {
        if on {
            row(stdout, "rule", what.into());
        }
    }
    let pb = &p.playback;
    let _ = writeln!(stdout, "  playback");
    if !pb.min_dwell_s.is_empty() {
        row(stdout, "min dwell", dwell(p));
    }
    if let Some(s) = pb.max_transition_s {
        row(stdout, "max transition", format!("{} s", num(s)));
    }
    if !pb.failure_state.is_empty() {
        row(stdout, "failure state", pb.failure_state.clone());
    }
    if !pb.school_zone.is_empty() {
        row(stdout, "school zone", pb.school_zone.clone());
    }
    if !pb.max_luminance_cd_m2.is_empty() {
        row(
            stdout,
            "max luminance",
            "cd/m2, by lighting period and zone".into(),
        );
        luminance(p, stdout);
    }
    for n in &p.notes {
        let _ = writeln!(stdout, "  note: {n}");
    }
}

pub fn run(a: &RulePacksArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let Some(reference) = &a.reference else {
        return list(a, stdout);
    };
    let packs = builtin_rule_packs();
    let Some(p) = packs.get(reference) else {
        let known: Vec<&str> = packs.keys().map(String::as_str).collect();
        let _ = writeln!(
            stderr,
            "opencrs rule-packs: no built-in rule pack {reference}; known: {}",
            known.join(", ")
        );
        return 1;
    };
    if a.json {
        let _ = stdout.write_all(json::to_string_pretty(p).as_bytes());
    } else {
        show(p, stdout);
    }
    0
}
