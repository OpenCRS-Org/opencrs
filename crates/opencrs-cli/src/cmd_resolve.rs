use crate::io::read_file;
use opencrs::resolve::parse_ref;
use opencrs::{decode_document, json};
use std::io::Write;

pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    if args.len() != 2 {
        let _ = writeln!(stderr, "usage: opencrs resolve SCREENS REF");
        return 2;
    }
    let doc = read_file(&args[0])
        .map_err(|(_, e)| e)
        .and_then(|raw| decode_document(&raw).map_err(|e| e.to_string()));
    let doc = match doc {
        Ok(d) => d,
        Err(e) => {
            let _ = writeln!(stderr, "{}: {e}", args[0]);
            return 2;
        }
    };
    let r = match parse_ref(&args[1]) {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(stderr, "{e}");
            return 2;
        }
    };
    let reqs = match doc.resolve(&r) {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(stderr, "{e}");
            return 1;
        }
    };
    let out: Vec<_> = reqs.iter().map(|r| r.view()).collect();
    let _ = stdout.write_all(json::to_string_pretty(&out).as_bytes());
    0
}
