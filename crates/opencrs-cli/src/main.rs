//! The `opencrs` command: see [`opencrs_cli::run`].

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
    let code = opencrs_cli::run(&args, &mut stdout, &mut stderr);
    let _ = stdout.flush();
    std::process::exit(code);
}
