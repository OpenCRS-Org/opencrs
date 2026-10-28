//! Writes the files generated from the `opencrs` command definition:
//! `man/opencrs.1` and the shell completions in `completions/`.
//!
//! `cargo xtask generate` rewrites them in the repository; the
//! `committed_files_are_current` test regenerates into a temporary directory
//! and fails if the committed files differ. Nothing generated mentions the
//! version, so releases don't change them.

use clap::Command;
use clap_mangen::{Man, roff::Roff, roff::bold, roff::roman};
use opencrs_cli::{
    DOCS_URL, EXIT_CALC, EXIT_CHECK, EXIT_CONFORMANCE, EXIT_FETCH, EXIT_PROBE, EXIT_PUBLISHER,
    EXIT_RESOLVE, EXIT_RULE_PACKS, EXIT_SCHEMA, Shell, command,
};
use std::io;
use std::path::{Path, PathBuf};

/// The files, as (path relative to the repository root, contents).
pub fn generate() -> io::Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut files = vec![(PathBuf::from("man/opencrs.1"), man_page()?)];
    for shell in Shell::ALL {
        let mut text = Vec::new();
        opencrs_cli::write_completions(shell, &mut text);
        files.push((Path::new("completions").join(shell.file_name()), text));
    }
    Ok(files)
}

/// The whole manual in one page: the top-level sections from clap_mangen,
/// each command as a subsection of COMMANDS, then the hand-written sections.
pub fn man_page() -> io::Result<Vec<u8>> {
    let root = command();
    let top = root
        .clone()
        .override_usage("opencrs <COMMAND>\nopencrs [-h|--help] [-V|--version]")
        .long_about(
            "Check digital-signage creative against the screens it's made for\n\n\
             opencrs reads a publisher's screens.json files, which say what \
             each screen needs, and a creative.json, which lists the files of \
             one piece of creative, and checks the files against the screens \
             they target. It can also find a publisher's files on the web and \
             check which of them to trust, check any OpenCRS document against \
             its schema, and run the conformance suite against a validator.",
        );
    let man = Man::new(top)
        .title("OPENCRS")
        .section("1")
        .source("OpenCRS")
        .manual("OpenCRS Manual");
    let mut out = Vec::new();
    man.render_title(&mut out)?;
    man.render_name_section(&mut out)?;
    man.render_synopsis_section(&mut out)?;
    man.render_description_section(&mut out)?;
    out.extend(description_extra());
    man.render_options_section(&mut out)?;

    let mut roff = Roff::default();
    roff.control("SH", ["COMMANDS"]);
    roff.to_writer(&mut out)?;
    for sub in root.get_subcommands() {
        command_section(&mut out, sub)?;
    }
    out.extend(extra_sections());
    Ok(out)
}

fn description_extra() -> Vec<u8> {
    let mut r = Roff::default();
    r.control("PP", []);
    r.text([roman(
        "Each command takes its own options; see COMMANDS. Every flag may also \
         be written with a single dash and its full name (for example \
         -spec for --spec), as in earlier versions.",
    )]);
    to_bytes(r)
}

/// One command as a subsection: what it does, its usage, its options.
fn command_section(out: &mut Vec<u8>, sub: &Command) -> io::Result<()> {
    let name = format!("opencrs {}", sub.get_name());
    let mut cmd = sub.clone().bin_name(&name).display_name(&name);
    cmd.build();
    let mut r = Roff::default();
    r.control("SS", [name.as_str()]);
    let about = cmd.get_about().map(|a| a.to_string()).unwrap_or_default();
    r.text([roman(format!("{about}."))]);
    // The paragraphs after the long description's first line.
    let long = cmd.get_long_about().map(|a| a.to_string());
    for p in long.iter().flat_map(|t| t.split("\n\n").skip(1)) {
        r.control("PP", []);
        r.text([roman(p)]);
    }
    r.control("PP", []);
    let usage = cmd.clone().render_usage().to_string();
    let mut first = true;
    for line in usage.lines() {
        let line = line.trim().trim_start_matches("Usage:").trim();
        if !first {
            r.control("br", []);
        }
        first = false;
        match line.strip_prefix(name.as_str()) {
            Some(rest) => r.text([bold(&name), roman(rest)]),
            None => r.text([roman(line)]),
        };
    }
    out.extend(to_bytes(r));
    if cmd.get_arguments().any(|a| !a.is_hide_set()) {
        let mut opts = Vec::new();
        Man::new(cmd).render_options_section(&mut opts)?;
        out.extend(without_headings(&opts));
    }
    Ok(())
}

/// A rendered section without its `.SH` line, to sit under a `.SS`.
fn without_headings(rendered: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(rendered)
        .lines()
        .filter(|l| !l.starts_with(".SH"))
        .map(|l| format!("{l}\n"))
        .collect::<String>()
        .into_bytes()
}

fn to_bytes(r: Roff) -> Vec<u8> {
    let mut v = Vec::new();
    r.to_writer(&mut v).expect("writing to a Vec");
    v
}

/// A block of lines shown as written: `.nf` ... `.fi`.
fn example(r: &mut Roff, lines: &[&str]) {
    r.control("PP", []);
    r.control("RS", ["4"]);
    r.control("nf", []);
    for l in lines {
        r.text([roman(*l)]);
    }
    r.control("fi", []);
    r.control("RE", []);
}

fn para(r: &mut Roff, text: &str) {
    r.control("PP", []);
    r.text([roman(text)]);
}

fn tagged(r: &mut Roff, tag: &str, text: &str) {
    r.control("TP", []);
    r.text([bold(tag)]);
    r.text([roman(text)]);
}

fn extra_sections() -> Vec<u8> {
    let mut r = Roff::default();

    r.control("SH", ["EXAMPLES"]);
    para(&mut r, "Check a creative against a publisher's screens:");
    example(
        &mut r,
        &["opencrs check --spec retail.screens.json creative.json"],
    );
    para(
        &mut r,
        "Check against two publishers and save the reports as JSON:",
    );
    example(
        &mut r,
        &[
            "opencrs check -s retail.screens.json -s airports.screens.json \\",
            "    --json creative.json > reports.json",
        ],
    );
    para(
        &mut r,
        "Check a screens.json and a creative.json against the schemas:",
    );
    example(&mut r, &["opencrs schema screens.json creative.json"]);
    para(&mut r, "Show what a screen requires:");
    example(
        &mut r,
        &["opencrs resolve retail.screens.json harbourline.example/retail/screen/HC-L1-002"],
    );
    para(
        &mut r,
        "Find a publisher's specs and save the ones to trust:",
    );
    example(
        &mut r,
        &["opencrs fetch --output specs harbourline.example"],
    );
    para(&mut r, "Check how a publisher serves its specs:");
    example(&mut r, &["opencrs publisher harbourline.example"]);
    para(&mut r, "See what the validator reads from a creative file:");
    example(&mut r, &["opencrs probe lunch-portrait.png"]);
    para(
        &mut r,
        "Work out the smallest text readable from 2 to 6 metres:",
    );
    example(
        &mut r,
        &["opencrs legibility --near 2 --far 6 --screen-width-mm 940 --screen-width-px 1080"],
    );
    para(
        &mut r,
        "Work out what a driver at 80 km/h can read, and a rule pack's limits:",
    );
    example(
        &mut r,
        &[
            "opencrs dwell --speed 80 --near 20 --far 150 \\",
            "    --pack au-nsw-transport-corridor@2017-11",
        ],
    );
    para(&mut r, "List the built-in rule packs, then show one:");
    example(
        &mut r,
        &[
            "opencrs rule-packs",
            "opencrs rule-packs au-nsw-transport-corridor@2017-11",
        ],
    );
    para(&mut r, "Install the zsh completions:");
    example(&mut r, &["opencrs completion zsh > ~/.zfunc/_opencrs"]);

    r.control("SH", ["EXIT STATUS"]);
    para(
        &mut r,
        "All commands exit 0 on success, 1 when what they checked failed, and \
         2 for a usage error or a file that could not be read. Asking for help \
         or the version exits 0.",
    );
    for (cmd, text) in [
        ("check", EXIT_CHECK),
        ("schema", EXIT_SCHEMA),
        ("resolve", EXIT_RESOLVE),
        ("fetch", EXIT_FETCH),
        ("publisher", EXIT_PUBLISHER),
        ("probe", EXIT_PROBE),
        ("legibility, dwell", EXIT_CALC),
        ("rule-packs", EXIT_RULE_PACKS),
        ("conformance", EXIT_CONFORMANCE),
    ] {
        tagged(&mut r, cmd, text);
    }

    r.control("SH", ["ENVIRONMENT"]);
    tagged(
        &mut r,
        "NO_COLOR",
        "When set to a non-empty value, help and error messages are never \
         coloured. They are also plain when output is not a terminal, or \
         when TERM is dumb.",
    );
    tagged(
        &mut r,
        "CLICOLOR_FORCE",
        "When set to a value other than 0, colour is used even when output \
         is not a terminal (NO_COLOR still wins).",
    );
    tagged(
        &mut r,
        "HTTP_PROXY, HTTPS_PROXY",
        "Ignored by fetch and publisher. A proxy would resolve host names where the \
         addresses can't be checked, so they always connect directly.",
    );

    r.control("SH", ["NETWORK"]);
    para(
        &mut r,
        "Only fetch and publisher use the network, and they share one HTTP \
         client. It is built so that an index cannot point it at a service on \
         the machine or its network.",
    );
    tagged(
        &mut r,
        "Scheme",
        "Only https:// URLs are fetched. With fetch --allow-local-http, \
         http:// is allowed for localhost, 127.0.0.1 and [::1] only.",
    );
    tagged(
        &mut r,
        "Addresses",
        "Each host, redirects included, is resolved once and refused if any of \
         its addresses is loopback, private, link-local, shared (CGNAT), \
         multicast, broadcast, unspecified, reserved or for documentation. The \
         connection goes to the addresses that were checked. With \
         --allow-local-http, loopback is allowed for localhost, 127.0.0.1 and \
         [::1] only; other internal addresses stay refused.",
    );
    tagged(
        &mut r,
        "Redirects",
        "At most four, and only to https://. A redirect to another host is \
         refused for a document that must stay on its publisher's host and for \
         any request that carries a token.",
    );
    tagged(
        &mut r,
        "Time",
        "Each request, redirects included, must finish within 30 seconds.",
    );
    tagged(
        &mut r,
        "Size",
        "A document larger than 50 MiB (52428800 bytes) is refused. One \
         discovery reads at most 512 MiB in total, indexes and documents \
         together.",
    );
    tagged(
        &mut r,
        "Publisher check",
        "publisher fetches one URL at a time and allows each request 2 \
         minutes, redirects included. It reads at most 4000 URLs, 50 MiB for \
         one page and 180 MiB in all; a URL past a limit is reported as not \
         fetched.",
    );
    tagged(
        &mut r,
        "Index entries",
        "An index may list at most 1000 documents, and at most 1000 delegates.",
    );
    tagged(
        &mut r,
        "Tokens",
        "A --token is sent only with requests for that publisher's partner \
         documents.",
    );

    r.control("SH", ["SEE ALSO"]);
    r.text([roman(format!("The OpenCRS specification: {DOCS_URL}"))]);
    to_bytes(r)
}
