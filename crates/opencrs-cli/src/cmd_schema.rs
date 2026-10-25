use crate::cli::SchemaArgs;
use crate::io::read_file;
use opencrs::batch::lint;
use opencrs::check::{Check, Severity};
use opencrs::{Kind, SchemaOptions};
use std::io::Write;

pub fn run(a: &SchemaArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let kind = a.kind.clone().unwrap_or_default();
    let files = &a.files;
    let opt = SchemaOptions {
        list: true,
        allow_local_http: a.allow_local_http,
    };
    let mut status = 0;
    for path in files {
        let raw = match read_file(path) {
            Ok(r) => r,
            Err((_, e)) => {
                let _ = writeln!(stderr, "{path}: {e}");
                status = 2;
                continue;
            }
        };
        let kind = if kind.is_empty() {
            Ok(None)
        } else {
            kind.parse::<Kind>().map(Some)
        };
        let res = kind.and_then(|k| lint(&raw, k, &opt));
        let res = match res {
            Ok(r) => r,
            Err(e) => {
                let _ = writeln!(stderr, "{path}: {e}");
                status = 2;
                continue;
            }
        };
        let line = |c: &Check| {
            let mut line = format!("  {:<18} {}", c.code, c.message);
            if !c.screens.is_empty() {
                line.push_str(&format!("  [{}]", c.screens.join(", ")));
            }
            line
        };
        if res.ok() {
            let warnings = &res.document_checks;
            if warnings.is_empty() {
                let _ = writeln!(stdout, "{path}: valid {} document", res.kind);
            } else {
                let _ = writeln!(
                    stdout,
                    "{path}: valid {} document, with {} warning(s)",
                    res.kind,
                    warnings.len()
                );
                for c in warnings {
                    let _ = writeln!(stdout, "{}", line(c));
                }
            }
            continue;
        }
        if !res.schema_errors.is_empty() {
            let _ = writeln!(
                stdout,
                "{path}: {} schema error(s) as a {} document",
                res.schema_errors.len(),
                res.kind
            );
            for e in &res.schema_errors {
                let _ = writeln!(stdout, "  {e}");
            }
        } else {
            let errors = res
                .document_checks
                .iter()
                .filter(|c| c.severity == Severity::Error)
                .count();
            let _ = writeln!(
                stdout,
                "{path}: matches the schema, but has {errors} document error(s)"
            );
            for c in &res.document_checks {
                let _ = writeln!(stdout, "{}", line(c));
            }
        }
        if status == 0 {
            status = 1;
        }
    }
    status
}
