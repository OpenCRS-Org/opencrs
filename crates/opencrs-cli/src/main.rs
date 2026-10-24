//! The `opencrs` command: see [`opencrs_cli::run_styled`].

use std::io::Write;

fn main() {
    let mut args: Vec<String> = Vec::new();
    for (i, a) in std::env::args_os().skip(1).enumerate() {
        match a.into_string() {
            Ok(a) => args.push(a),
            Err(_) => {
                eprintln!("opencrs: argument {} is not valid UTF-8", i + 1);
                std::process::exit(2);
            }
        }
    }
    // Buffered: a network-sized check prints tens of thousands of lines, and
    // stdout is line-buffered by default.
    let mut stdout = std::io::BufWriter::with_capacity(64 * 1024, std::io::stdout().lock());
    let mut stderr = std::io::stderr().lock();
    // Colour only on a terminal, and never with NO_COLOR (anstream also
    // honours CLICOLOR, CLICOLOR_FORCE and TERM=dumb).
    let color = opencrs_cli::Color {
        stdout: anstream::AutoStream::choice(&std::io::stdout()) != anstream::ColorChoice::Never,
        stderr: anstream::AutoStream::choice(&std::io::stderr()) != anstream::ColorChoice::Never,
    };
    let code = opencrs_cli::run_styled(&args, &mut stdout, &mut stderr, color);
    let _ = stdout.flush();
    std::process::exit(code);
}
