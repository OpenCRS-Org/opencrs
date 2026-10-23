//! The `opencrs` command line: `schema`, `check` and `resolve`, ported from
//! `cmd/opencrs/main.go`.

pub mod args;
mod cmd_check;
mod cmd_conformance;
mod cmd_fetch;
mod cmd_resolve;
mod cmd_schema;
pub mod conformance;
mod gotime;
pub mod http;
mod io;

use std::io::Write;

pub(crate) const USAGE: &str = "opencrs: the OpenCRS 0.1 reference validator

Usage:
  opencrs schema [-kind K] [-allow-local-http] FILE...
                                            check documents against the JSON Schema; for a
                                            screens.json, also check references and rule conflicts
  opencrs check -spec SCREENS [-spec ...] [flags] CREATIVE
                                            check a creative.json's files against requirements
  opencrs resolve SCREENS REF               print a format's or screen's resolved requirements
  opencrs fetch [flags] DOMAIN|URL          discover a publisher's documents and check which to trust;
                                            given a document URL, check that its publisher vouches for it
  opencrs conformance [-level L] [-cmd CMD] [DIR]
                                            run the conformance suite (default conformance/0.1) against
                                            this validator, or against CMD, which gets each case directory

Kinds: index, screens, creative, report, rule-pack (detected when -kind is omitted).
A JSON array is checked element by element, e.g. the output of check -json.
REF: <publisher>/<network>/<format> or <publisher>/<network>/screen/<id>

Exit status: 0 ok, 1 a document or creative failed, 2 usage or read error.
";

/// Runs the command with `args` (without the program name), returning the
/// exit status: 0 ok, 1 a document or creative failed, 2 usage or read error.
pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let Some(first) = args.first() else {
        let _ = stderr.write_all(USAGE.as_bytes());
        return 2;
    };
    let rest = &args[1..];
    match first.as_str() {
        "schema" => cmd_schema::run(rest, stdout, stderr),
        "check" => cmd_check::run(rest, stdout, stderr),
        "resolve" => cmd_resolve::run(rest, stdout, stderr),
        "fetch" => cmd_fetch::run(rest, stdout, stderr),
        "conformance" => cmd_conformance::run(rest, stdout, stderr),
        "help" | "-h" | "-help" | "--help" => {
            let _ = stdout.write_all(USAGE.as_bytes());
            0
        }
        other => {
            let _ = write!(
                stderr,
                "opencrs: unknown command {}\n\n{USAGE}",
                opencrs::gofmt::quote(other)
            );
            2
        }
    }
}
