use crate::cli::ConformanceArgs;
use crate::conformance::{command, load, reference, run as run_cases};
use opencrs::conformance::LEVELS;
use std::io::Write;

fn title(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn run(a: &ConformanceArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let level = a.level.clone();
    let cmd = a.cmd.clone().unwrap_or_default();
    let verbose = a.verbose;
    if !LEVELS.contains(&level.as_str()) {
        let _ = writeln!(
            stderr,
            "opencrs conformance: --level must be one of {}",
            LEVELS.join(", ")
        );
        return 2;
    }
    let dir = a.dir.to_string_lossy().into_owned();
    let cases = match load(&dir) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(stderr, "opencrs conformance: {e}");
            return 2;
        }
    };
    let external = command(&cmd);
    let (v, name): (&dyn Fn(&_) -> _, String) = if cmd.is_empty() {
        (&reference, "the reference validator".into())
    } else {
        (&external, cmd.clone())
    };
    let cases: Vec<_> = cases
        .into_iter()
        .filter(|c| !(a.no_strictest && c.is_optional()))
        .collect();
    let results = run_cases(&cases, &level, v);
    let show = |stdout: &mut dyn Write, r: &crate::conformance::CaseResult| {
        if r.passed() {
            if verbose {
                let _ = writeln!(stdout, "pass  {:<34} {}", r.case.id, r.case.description);
            }
            return;
        }
        let _ = writeln!(stdout, "FAIL  {:<34} {}", r.case.id, r.case.description);
        let _ = writeln!(stdout, "      spec: {}", r.case.spec);
        if let Some(e) = &r.err {
            let _ = writeln!(stdout, "      error: {e}");
        }
        for p in &r.problems {
            let _ = writeln!(stdout, "      {p}");
        }
    };
    for r in &results {
        show(stdout, r);
    }
    let (optional, required): (Vec<_>, Vec<_>) = results.iter().partition(|r| r.case.is_optional());
    let passed = required.iter().filter(|r| r.passed()).count();
    let verdict = if passed < required.len() {
        "does not conform"
    } else {
        "conforms"
    };
    let _ = writeln!(
        stdout,
        "{name} {verdict} at the {} level: {passed} of {} cases pass",
        title(&level),
        required.len()
    );
    let mut failed = passed < required.len();
    if a.no_strictest {
        let _ = writeln!(stdout, "Strictest requirements (optional) were not tested");
    } else if !optional.is_empty() {
        let ok = optional.iter().filter(|r| r.passed()).count();
        let verdict = if ok < optional.len() {
            "does not support"
        } else {
            "supports"
        };
        let _ = writeln!(
            stdout,
            "{name} {verdict} strictest requirements (optional): {ok} of {} cases pass",
            optional.len()
        );
        failed |= ok < optional.len();
    }
    i32::from(failed)
}
