//! The `opencrs` command line: `check`, `schema`, `resolve`, `fetch`,
//! `conformance`, `completion` and `version`, ported from `cmd/opencrs/main.go`
//! and given a clap front end, and `probe`, `legibility`, `dwell`, `rule-packs`
//! and `publisher`, which put the rest of the core on the command line.

mod cli;
mod cmd_calc;
mod cmd_check;
mod cmd_conformance;
mod cmd_fetch;
mod cmd_probe;
mod cmd_publisher;
mod cmd_resolve;
mod cmd_rulepacks;
mod cmd_schema;
pub mod conformance;
mod escape;
mod gotime;
pub use opencrs_fetch::http;
mod io;

pub use cli::{DOCS_URL, Shell, command, write_completions};
pub use cli::{
    EXIT_CALC, EXIT_CHECK, EXIT_CONFORMANCE, EXIT_FETCH, EXIT_PROBE, EXIT_PUBLISHER, EXIT_RESOLVE,
    EXIT_RULE_PACKS, EXIT_SCHEMA,
};

use clap::FromArgMatches;
use clap::error::ErrorKind;
use cli::{Cli, Cmd};
use std::io::Write;

/// Whether to colour what goes to each stream. The binary decides from the
/// terminal and the environment (`NO_COLOR` and the like); [`run`] never does.
#[derive(Clone, Copy, Default)]
pub struct Color {
    pub stdout: bool,
    pub stderr: bool,
}

/// Runs the command with `args` (without the program name), returning the
/// exit status: 0 ok, 1 a document or creative failed, 2 usage or read error.
/// Help and errors are plain text.
pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    run_styled(args, stdout, stderr, Color::default())
}

/// [`run`], with colour in help and usage errors where `color` allows.
pub fn run_styled(
    args: &[String],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    color: Color,
) -> i32 {
    let root = command();
    let argv = std::iter::once("opencrs".to_string()).chain(cli::normalize(&root, args));
    let parsed = root
        .try_get_matches_from(argv)
        .and_then(|m| Cli::from_arg_matches(&m));
    let cli = match parsed {
        Ok(c) => c,
        Err(e) => {
            let to_stderr = e.use_stderr();
            let styled = if to_stderr {
                color.stderr
            } else {
                color.stdout
            };
            let rendered = e.render();
            let text = if styled {
                rendered.ansi().to_string()
            } else {
                rendered.to_string()
            };
            let w: &mut dyn Write = if to_stderr { stderr } else { stdout };
            let _ = w.write_all(text.as_bytes());
            if e.kind() == ErrorKind::DisplayHelp || e.kind() == ErrorKind::DisplayVersion {
                return 0;
            }
            return 2;
        }
    };
    match &cli.command {
        Cmd::Check(a) => cmd_check::run(a, stdout, stderr),
        Cmd::Schema(a) => cmd_schema::run(a, stdout, stderr),
        Cmd::Resolve(a) => cmd_resolve::run(a, stdout, stderr),
        // These print what a publisher wrote, so a terminal gets it with the
        // control characters shown, not obeyed.
        Cmd::Fetch(a) => cmd_fetch::run(
            a,
            &mut escape::Escaped(stdout),
            &mut escape::Escaped(stderr),
        ),
        Cmd::Publisher(a) => cmd_publisher::run(
            a,
            &mut escape::Escaped(stdout),
            &mut escape::Escaped(stderr),
        ),
        Cmd::Probe(a) => cmd_probe::run(a, stdout, stderr),
        Cmd::Legibility(a) => cmd_calc::legibility(a, stdout, stderr),
        Cmd::Dwell(a) => cmd_calc::dwell(a, stdout, stderr),
        Cmd::RulePacks(a) => cmd_rulepacks::run(a, stdout, stderr),
        Cmd::Conformance(a) => cmd_conformance::run(a, stdout, stderr),
        Cmd::Completion(a) => {
            write_completions(a.shell, stdout);
            0
        }
        Cmd::Version => {
            let _ = stdout.write_all(version_text().as_bytes());
            0
        }
    }
}

/// What `opencrs version` prints: the tool's version, the spec version, and
/// the rule packs built in, with their status.
pub fn version_text() -> String {
    let mut s = format!(
        "opencrs {}\nOpenCRS spec {}\n\nRule packs:\n",
        env!("CARGO_PKG_VERSION"),
        opencrs::SPEC_VERSION
    );
    let width = opencrs::rulepack::builtin_rule_packs()
        .keys()
        .map(String::len)
        .max()
        .unwrap_or(0);
    for (reference, pack) in opencrs::rulepack::builtin_rule_packs() {
        s.push_str(&format!("  {reference:<width$}  {}\n", pack.status));
    }
    s
}
