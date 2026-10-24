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
    let results = run_cases(&cases, &level, v);
    let mut passed = 0;
    for r in &results {
        if r.passed() {
            passed += 1;
            if verbose {
                let _ = writeln!(stdout, "pass  {:<34} {}", r.case.id, r.case.description);
            }
            continue;
        }
        let _ = writeln!(stdout, "FAIL  {:<34} {}", r.case.id, r.case.description);
        let _ = writeln!(stdout, "      spec: {}", r.case.spec);
        if let Some(e) = &r.err {
            let _ = writeln!(stdout, "      error: {e}");
        }
        for p in &r.problems {
            let _ = writeln!(stdout, "      {p}");
        }
    }
    let verdict = if passed < results.len() {
        "does not conform"
    } else {
        "conforms"
    };
    let _ = writeln!(
        stdout,
        "{name} {verdict} at the {} level: {passed} of {} cases pass",
        title(&level),
        results.len()
    );
    i32::from(passed < results.len())
}
