use crate::args::FlagSet;
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

pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let (level, cmd, verbose, rest) = {
        let mut fs = FlagSet::new("conformance", stderr);
        fs.string(
            "level",
            "full",
            "conformance level to test: core, layout or full",
        );
        fs.string(
            "cmd",
            "",
            "external validator to test: run with each case directory as its last argument, it must print the reports as a JSON array",
        );
        fs.boolean("v", "list passing cases too");
        if fs.parse_interleaved(args).is_err() {
            return 2;
        }
        (
            fs.get_string("level").to_string(),
            fs.get_string("cmd").to_string(),
            fs.get_bool("v"),
            fs.rest().to_vec(),
        )
    };
    if !LEVELS.contains(&level.as_str()) {
        let _ = writeln!(
            stderr,
            "opencrs conformance: -level must be one of {}",
            LEVELS.join(", ")
        );
        return 2;
    }
    let mut dir = "conformance/0.1".to_string();
    if rest.len() > 1 {
        let _ = writeln!(
            stderr,
            "usage: opencrs conformance [-level L] [-cmd CMD] [DIR]"
        );
        return 2;
    } else if rest.len() == 1 {
        dir = rest[0].clone();
    }
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
