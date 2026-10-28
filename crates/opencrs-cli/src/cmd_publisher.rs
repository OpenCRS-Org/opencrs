use crate::cli::PublisherArgs;
use crate::cmd_fetch::print_discovery;
use opencrs::batch::LintResult;
use opencrs::json;
use opencrs_fetch::{FetchOptions, fetch_publisher};
use std::io::Write;

/// One document's own problems, under its URL.
fn lint(stdout: &mut dyn Write, url: &str, l: &LintResult) {
    for e in &l.schema_errors {
        let _ = writeln!(stdout, "  error     {url}\n            {e}");
    }
    for c in &l.document_checks {
        let _ = writeln!(
            stdout,
            "  {:<9} {url}\n            {}  {}",
            c.severity.as_str(),
            c.code,
            c.message
        );
    }
}

pub fn run(a: &PublisherArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let now = match opencrs::fs::parse_now(a.now.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(stderr, "opencrs publisher: {e}");
            return 2;
        }
    };
    let f = fetch_publisher(&a.domain, now, &FetchOptions::default());
    let res = &f.check;
    let rounds = (f.rounds_exhausted).then(|| {
        format!(
            "the check still wanted URLs after its last round ({} rounds)",
            f.rounds
        )
    });
    // A domain that can't publish, or an index that can't be used, is an
    // error in what was asked for, not a finding about the publisher's files.
    let status = if !res.error.is_empty() || rounds.is_some() {
        2
    } else if res.errors > 0 {
        1
    } else {
        0
    };
    if a.json {
        let _ = stdout.write_all(json::to_string_pretty(res).as_bytes());
        if let Some(why) = rounds {
            let _ = writeln!(stderr, "opencrs publisher: {why}");
        }
        return status;
    }
    if !res.error.is_empty() {
        let _ = writeln!(stderr, "opencrs publisher: {}", res.error);
        return status;
    }
    if let Some(d) = &res.discovery {
        print_discovery(stdout, d);
    }
    if let Some(l) = &res.index_lint {
        lint(stdout, &res.index_url, l);
    }
    for (url, l) in &res.lints {
        lint(stdout, url, l);
    }
    for adv in &res.advice {
        let _ = writeln!(
            stdout,
            "  {:<9} {}\n            {}",
            adv.severity.as_str(),
            adv.url,
            adv.message
        );
    }
    let _ = writeln!(
        stdout,
        "\n{}: {} error(s), {} warning(s)",
        res.domain, res.errors, res.warnings
    );
    if let Some(why) = rounds {
        let _ = writeln!(stderr, "opencrs publisher: {why}");
    }
    status
}
