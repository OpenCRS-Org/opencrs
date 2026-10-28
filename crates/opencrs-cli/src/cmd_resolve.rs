use crate::cli::ResolveArgs;
use crate::io::read_file;
use opencrs::resolve::parse_ref;
use opencrs::{decode_document, json};
use std::io::Write;

pub fn run(a: &ResolveArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let args = [&a.screens, &a.reference];
    let doc = read_file(args[0])
        .map_err(|(_, e)| e)
        .and_then(|raw| decode_document(&raw).map_err(|e| e.to_string()));
    let doc = match doc {
        Ok(d) => d,
        Err(e) => {
            let _ = writeln!(stderr, "{}: {e}", args[0]);
            return 2;
        }
    };
    let r = match parse_ref(args[1]) {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(stderr, "{e}");
            return 2;
        }
    };
    let out = match opencrs::budget::ensure(doc.size, || {
        // Views are built as the screens are resolved, and no more once the
        // budget is spent: resolve_each then says why.
        let mut out = Vec::new();
        doc.resolve_each(&r, |q| {
            if !opencrs::budget::tripped() {
                out.push(q.view());
            }
        })?;
        match opencrs::budget::explain(Some(&doc)) {
            Some(why) => Err(opencrs::resolve::ResolveError::TooComplex(why)),
            None => Ok(out),
        }
    }) {
        Ok(x) => x,
        Err(e) => {
            let _ = writeln!(stderr, "{e}");
            return 1;
        }
    };
    let _ = stdout.write_all(json::to_string_pretty(&out).as_bytes());
    0
}
