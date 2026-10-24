//! `cargo xtask generate` rewrites the man page and completions.
//! `cargo xtask check` fails if the committed ones are out of date.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn main() -> ExitCode {
    let mode = std::env::args().nth(1);
    let files = match opencrs_xtask::generate() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("xtask: {e}");
            return ExitCode::FAILURE;
        }
    };
    match mode.as_deref() {
        Some("generate") => {
            for (path, text) in files {
                let full = root().join(&path);
                if let Some(dir) = full.parent() {
                    std::fs::create_dir_all(dir).expect("create the directory");
                }
                std::fs::write(&full, text).expect("write the file");
                println!("wrote {}", path.display());
            }
            ExitCode::SUCCESS
        }
        Some("check") => {
            let mut stale = false;
            for (path, text) in files {
                if std::fs::read(root().join(&path)).ok().as_deref() != Some(&text[..]) {
                    eprintln!("{} is out of date", path.display());
                    stale = true;
                }
            }
            if stale {
                eprintln!("run: cargo xtask generate");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: cargo xtask generate|check");
            ExitCode::from(2)
        }
    }
}
