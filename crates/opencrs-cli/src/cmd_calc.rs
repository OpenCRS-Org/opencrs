//! `legibility` and `dwell`: the core's calculators, one line of flags each.

use crate::cli::{DwellArgs, LegibilityArgs};
use opencrs::calc::{self, PackLimits, ScreenWidth, TextSize};
use opencrs::json;
use std::io::Write;

/// Formats a number without trailing zeros, as the reports do.
pub(crate) fn num(f: f64) -> String {
    if f == f.trunc() {
        return format!("{}", f as i64);
    }
    let s = format!("{f:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The screen flags are given together or not at all (clap enforces it).
fn screen(mm: Option<f64>, px: Option<i64>) -> Option<ScreenWidth> {
    Some(ScreenWidth { mm: mm?, px: px? })
}

fn size(t: &TextSize) -> String {
    match t.px {
        Some(px) => format!("{} mm, {px} px  ({})", num(t.mm), t.basis),
        None => format!("{} mm  ({})", num(t.mm), t.basis),
    }
}

pub fn legibility(a: &LegibilityArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let s = screen(a.screen_width_mm, a.screen_width_px);
    let l = match calc::legibility([a.near, a.far], a.ratio, s, a.dwell, a.rate) {
        Ok(l) => l,
        Err(e) => {
            let _ = writeln!(stderr, "opencrs legibility: {e}");
            return 2;
        }
    };
    if a.json {
        let _ = stdout.write_all(json::to_string_pretty(&l).as_bytes());
        return 0;
    }
    let _ = writeln!(
        stdout,
        "Text seen from {} m to {} m, legible at {} times its height",
        num(a.near),
        num(a.far),
        num(l.ratio)
    );
    let _ = writeln!(stdout, "  key text    {}", size(&l.key));
    let _ = writeln!(stdout, "  legal text  {}", size(&l.legal));
    if let Some(w) = &l.words {
        let _ = writeln!(stdout, "  word limit  {} words  ({})", w.value, w.basis);
    }
    0
}

/// One pack's limits at the speed asked about.
fn pack(stdout: &mut dyn Write, p: &PackLimits) {
    let _ = writeln!(stdout, "\n{} ({}): {}", p.reference, p.status, p.title);
    let mut say = |label: &str, value: String| {
        let _ = writeln!(stdout, "  {label:<15} {value}");
    };
    match (p.min_dwell_s, p.dwell_basis.as_str()) {
        (Some(s), "") => say("min dwell", format!("{} s", num(s))),
        (Some(s), basis) => say("min dwell", format!("{} s ({basis})", num(s))),
        (None, _) if !p.dwell_unknown.is_empty() => say("min dwell", p.dwell_unknown.clone()),
        (None, _) => {}
    }
    if let Some(s) = p.max_transition_s {
        say("max transition", format!("{} s", num(s)));
    }
    if let Some(allowed) = p.motion_allowed {
        say(
            "motion",
            if allowed { "allowed" } else { "not allowed" }.into(),
        );
    }
    let rules = [
        (p.no_flashing, "no flashing"),
        (p.max_words_at_glance, "words limited to a glance"),
        (p.no_sequencing, "no sequencing"),
        (p.no_traffic_device_imagery, "no traffic-device imagery"),
        (p.no_device_interaction, "no device interaction"),
    ];
    let on: Vec<&str> = rules
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, s)| *s)
        .collect();
    if !on.is_empty() {
        say("creative", on.join(", "));
    }
    if !p.failure_state.is_empty() {
        say("failure state", p.failure_state.clone());
    }
    if !p.school_zone.is_empty() {
        say("school zone", p.school_zone.clone());
    }
    if !p.status_note.is_empty() {
        say("status note", p.status_note.clone());
    }
}

pub fn dwell(a: &DwellArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let s = screen(a.screen_width_mm, a.screen_width_px);
    let d = match calc::dwell(a.speed, [a.near, a.far], a.ratio, a.rate, s, &a.pack) {
        Ok(d) => d,
        Err(e) => {
            let _ = writeln!(stderr, "opencrs dwell: {e}");
            return 2;
        }
    };
    if a.json {
        let _ = stdout.write_all(json::to_string_pretty(&d).as_bytes());
        return 0;
    }
    let g = &d.glance;
    let _ = writeln!(
        stdout,
        "At {} km/h ({} m/s), seen from {} m to {} m: {} s to cross",
        num(d.speed_kmh),
        num(g.speed_m_s),
        num(a.near),
        num(a.far),
        num(g.crossing_s)
    );
    let _ = writeln!(
        stdout,
        "  glance      {} s, {} words, reading from {} m",
        num(g.seconds),
        g.words,
        num(g.read_from_m)
    );
    let _ = writeln!(stdout, "  key text    {}", size(&d.key));
    for p in &d.packs {
        pack(stdout, p);
    }
    0
}
