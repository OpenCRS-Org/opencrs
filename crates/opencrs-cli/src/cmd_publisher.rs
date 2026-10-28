use crate::cli::PublisherArgs;
use crate::cmd_fetch::print_discovery;
use chrono::{DateTime, FixedOffset};
use opencrs::batch::LintResult;
use opencrs::json;
use opencrs_fetch::{FetchOptions, Fetched, fetch_publisher};
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
    run_with(a, stdout, stderr, &|domain, now| {
        fetch_publisher(domain, now, &FetchOptions::default())
    })
}

type Fetch<'a> = &'a dyn Fn(&str, DateTime<FixedOffset>) -> Fetched;

pub fn run_with(
    a: &PublisherArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    fetch: Fetch<'_>,
) -> i32 {
    let now = match opencrs::fs::parse_now(a.now.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(stderr, "opencrs publisher: {e}");
            return 2;
        }
    };
    let f = fetch(&a.domain, now);
    let res = &f.check;
    let exhausted = f.rounds_exhausted.then(|| {
        format!(
            "the check still wanted URLs after its last round ({} rounds)",
            f.rounds
        )
    });
    // A domain that can't publish, or an index that couldn't be fetched, is
    // an error in what was asked for. An index that arrived but is invalid
    // is a finding about the publisher's files, like any other.
    let status = if exhausted.is_some() || (!res.error.is_empty() && res.index_lint.is_none()) {
        2
    } else if res.errors > 0 {
        1
    } else {
        0
    };
    if a.json {
        let _ = stdout.write_all(json::to_string_pretty(res).as_bytes());
    } else {
        if !res.error.is_empty() {
            let _ = writeln!(stderr, "opencrs publisher: {}", res.error);
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
        if res.error.is_empty() {
            let _ = writeln!(
                stdout,
                "\n{}: {} error(s), {} warning(s)",
                res.domain, res.errors, res.warnings
            );
        }
    }
    if let Some(why) = exhausted {
        let _ = writeln!(stderr, "opencrs publisher: {why}");
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencrs::publisher::{Page, check_publisher};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const INDEX_URL: &str = "https://harbourline.example/.well-known/opencrs.json";
    const DOC_URL: &str = "https://harbourline.example/opencrs/retail.screens.json";

    fn example(name: &str) -> Vec<u8> {
        std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/harbourline")
                .join(name),
        )
        .unwrap()
    }

    struct Served {
        status: u16,
        body: Vec<u8>,
        headers: bool,
    }

    /// Runs the command over pages held in memory, in place of the network.
    fn run_over(args: &[&str], pages: &[(&str, Served)]) -> (i32, String, String) {
        let a = PublisherArgs {
            domain: "harbourline.example".into(),
            json: args.contains(&"--json"),
            now: Some("2026-10-25T12:00:00Z".into()),
        };
        let fetch = |domain: &str, now| {
            let pages: BTreeMap<String, Page<'_>> = pages
                .iter()
                .map(|(url, s)| {
                    let mut p = Page {
                        status: s.status,
                        body: &s.body,
                        ..Page::default()
                    };
                    if s.headers {
                        p.cache_control = "max-age=60".into();
                        p.etag = "\"v1\"".into();
                        p.content_type = "application/json".into();
                        p.allow_origin = "*".into();
                    }
                    (url.to_string(), p)
                })
                .collect();
            Fetched {
                check: check_publisher(domain, &pages, "not fetched", now),
                rounds_exhausted: false,
                rounds: 5,
                cancelled: false,
            }
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_with(&a, &mut out, &mut err, &fetch);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    fn served(name: &str, headers: bool) -> Served {
        Served {
            status: 200,
            body: example(name),
            headers,
        }
    }

    #[test]
    fn a_clean_publisher_passes() {
        let (code, out, err) = run_over(
            &[],
            &[
                (INDEX_URL, served(".well-known/opencrs.json", true)),
                (DOC_URL, served("retail.screens.json", true)),
            ],
        );
        assert_eq!(code, 0, "{out}{err}");
        assert!(
            out.contains(
                "trusted   public   https://harbourline.example/opencrs/retail.screens.json"
            ),
            "{out}"
        );
        assert!(
            out.ends_with("harbourline.example: 0 error(s), 0 warning(s)\n"),
            "{out}"
        );
        assert_eq!(err, "");
    }

    #[test]
    fn how_the_files_are_served_is_advice() {
        let (code, out, _) = run_over(
            &[],
            &[
                (INDEX_URL, served(".well-known/opencrs.json", false)),
                (DOC_URL, served("retail.screens.json", false)),
            ],
        );
        // Warnings are not errors.
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("  warning   https://harbourline.example/opencrs/retail.screens.json\n            retail.screens.json is served without a Cache-Control header."), "{out}");
        assert!(out.contains("served without an ETag"), "{out}");
        assert!(
            out.ends_with("harbourline.example: 0 error(s), 4 warning(s)\n"),
            "{out}"
        );
        let (code, out, _) = run_over(
            &["--json"],
            &[
                (INDEX_URL, served(".well-known/opencrs.json", false)),
                (DOC_URL, served("retail.screens.json", false)),
            ],
        );
        assert_eq!(code, 0);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["warnings"], 4);
        assert_eq!(v["wanted"], serde_json::json!([]));
    }

    #[test]
    fn an_index_that_arrives_but_is_invalid_is_a_finding() {
        let bad = Served {
            status: 200,
            body: br#"{"opencrs":"0.1"}"#.to_vec(),
            headers: true,
        };
        let (code, out, err) = run_over(&[], &[(INDEX_URL, bad)]);
        assert_eq!(code, 1, "{out}{err}");
        assert!(err.starts_with("opencrs publisher: "), "{err}");
        // What is wrong with the index is listed, on stdout.
        assert!(
            out.contains(&format!("  error     {INDEX_URL}\n            /")),
            "{out}"
        );
        assert!(out.contains("publisher"), "{out}");
    }

    #[test]
    fn an_index_that_cannot_be_fetched_is_a_usage_error() {
        let gone = Served {
            status: 404,
            body: Vec::new(),
            headers: false,
        };
        let (code, out, err) = run_over(&[], &[(INDEX_URL, gone)]);
        assert_eq!(code, 2);
        assert_eq!(out, "");
        assert!(err.contains("HTTP 404"), "{err}");
    }

    #[test]
    fn a_document_with_problems_is_listed_with_them() {
        let broken = Served {
            status: 200,
            body: br#"{"opencrs":"0.1","publisher":"harbourline.example","updated_at":"2026-10-20T09:00:00Z","networks":[{"id":"n","name":"N","formats":[{"id":"f","pixels":[1,1],"media":{"static":["image/png"]}}],"screens":[{"id":"s","format":"g"}]}]}"#.to_vec(),
            headers: true,
        };
        let (code, out, _) = run_over(
            &[],
            &[
                (INDEX_URL, served(".well-known/opencrs.json", true)),
                (DOC_URL, broken),
            ],
        );
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("CRS-REFERENCE"), "{out}");
    }
}
