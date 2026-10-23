use crate::args::FlagSet;
use crate::gotime;
use crate::io::read_file;
use opencrs::batch::{check_manifest_each, manifest_notes};
use opencrs::fs::{host_now, probe_dir};
use opencrs::report::{Options, Outcome};
use opencrs::{decode_document, decode_manifest, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

/// Go's `crs.DefaultWarnMargin`.
const DEFAULT_WARN_MARGIN: f64 = 1.25;

pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let (specs, files_dir, no_files, now, as_json, margin, manifest_path) = {
        let mut fs = FlagSet::new("check", stderr);
        fs.multi("spec", "screens.json to check against (repeatable)");
        fs.string(
            "files",
            "",
            "directory holding the creative files (default: the manifest's directory)",
        );
        fs.boolean(
            "no-files",
            "use only the manifest's declared values; don't read the files",
        );
        fs.boolean("json", "print reports as a JSON array of spec reports");
        fs.string("now", "", "validation time, RFC 3339 (default: now)");
        fs.float(
            "margin",
            DEFAULT_WARN_MARGIN,
            "1.25",
            "warn when text is below this multiple of its minimum size",
        );
        if fs.parse_interleaved(args).is_err() {
            return 2;
        }
        if fs.get_multi("spec").is_empty() || fs.rest().len() != 1 {
            drop(fs);
            let _ = writeln!(
                stderr,
                "usage: opencrs check -spec SCREENS [-spec ...] [flags] CREATIVE"
            );
            return 2;
        }
        (
            fs.get_multi("spec").to_vec(),
            fs.get_string("files").to_string(),
            fs.get_bool("no-files"),
            fs.get_string("now").to_string(),
            fs.get_bool("json"),
            fs.get_float("margin"),
            fs.rest()[0].clone(),
        )
    };
    let now = if now.is_empty() {
        host_now()
    } else {
        match gotime::parse_rfc3339(&now) {
            Ok(t) => t,
            Err(e) => {
                let _ = writeln!(stderr, "opencrs check: -now: {e}");
                return 2;
            }
        }
    };

    let mut docs = Vec::new();
    for path in &specs {
        let d = read_file(path)
            .map_err(|(_, e)| e)
            .and_then(|raw| decode_document(&raw).map_err(|e| e.to_string()));
        match d {
            Ok(d) => docs.push(d),
            Err(e) => {
                let _ = writeln!(stderr, "{path}: {e}");
                return 2;
            }
        }
    }
    let raw = match read_file(&manifest_path) {
        Ok(r) => r,
        Err((_, e)) => {
            let _ = writeln!(stderr, "{e}");
            return 2;
        }
    };
    let m = match decode_manifest(&raw) {
        Ok(m) => m,
        Err(e) => {
            let _ = writeln!(stderr, "{manifest_path}: {e}");
            return 2;
        }
    };
    let dir = if files_dir.is_empty() {
        manifest_dir(&manifest_path)
    } else {
        files_dir
    };

    let mut probes = HashMap::new();
    if !no_files {
        match probe_dir(Path::new(&dir), &m) {
            Ok(found) => {
                for f in &found.missing {
                    let _ = writeln!(
                        stderr,
                        "note: {f} not found in {dir}; using the manifest's declared values"
                    );
                }
                probes = found.probes;
            }
            Err(e) => {
                let _ = writeln!(stderr, "{e}");
                return 2;
            }
        }
    }
    // Go: a zero margin means the default.
    let mut opt = Options::new(now);
    opt.warn_margin = if margin == 0.0 {
        DEFAULT_WARN_MARGIN
    } else {
        margin
    };
    let notes = match manifest_notes(&docs, &m) {
        Ok(n) => n,
        Err(e) => {
            let _ = writeln!(stderr, "{e}");
            return 2;
        }
    };
    for n in &notes {
        let _ = writeln!(stderr, "note: {n}");
    }
    let mut failed = false;
    if as_json {
        let mut reports = Vec::new();
        let r = check_manifest_each(&docs, &m, &probes, &opt, |r| {
            failed |= r.result == Outcome::Fail;
            reports.push(r);
        });
        if let Err(e) = r {
            let _ = writeln!(stderr, "{e}");
            return 2;
        }
        let _ = stdout.write_all(json::to_string_pretty(&reports).as_bytes());
        return i32::from(failed);
    }
    // Text goes out as the reports come, so a network-sized check never holds
    // them all. `manifest_notes` has already ruled out the only errors.
    let mut first = true;
    let r = check_manifest_each(&docs, &m, &probes, &opt, |r| {
        failed |= r.result == Outcome::Fail;
        if !first {
            let _ = writeln!(stdout);
        }
        first = false;
        let _ = writeln!(
            stdout,
            "{} \u{2192} {}: {}",
            r.file,
            r.target,
            r.result.as_str().to_uppercase()
        );
        for c in &r.checks {
            let _ = write!(
                stdout,
                "  {:<11} {:<22} {}",
                c.severity.as_str(),
                c.code,
                c.message
            );
            if !c.screens.is_empty() {
                let _ = write!(stdout, "  [{}]", c.screens.join(", "));
            }
            let _ = writeln!(stdout);
        }
    });
    if let Err(e) = r {
        let _ = writeln!(stderr, "{e}");
        return 2;
    }
    i32::from(failed)
}

/// The directory a manifest's files are relative to: Go's `filepath.Dir`.
/// On Windows that's the path's parent with either separator (and a drive);
/// elsewhere, everything before the last `/`, cleaned.
fn manifest_dir(manifest_path: &str) -> String {
    if cfg!(windows) {
        return match Path::new(manifest_path).parent() {
            Some(d) if !d.as_os_str().is_empty() => d.to_string_lossy().into_owned(),
            _ => ".".to_string(),
        };
    }
    let parent = match manifest_path.rfind('/') {
        Some(i) => &manifest_path[..=i],
        None => "",
    };
    crate::io::clean_path(parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_dir_with_slashes() {
        assert_eq!(manifest_dir("creative.json"), ".");
        assert_eq!(manifest_dir("work/creative.json"), "work");
        assert_eq!(manifest_dir("a/b/creative.json"), "a/b");
    }

    /// The final review's I1: with only `/` treated as a separator, a
    /// Windows path's files were looked for in the current directory.
    #[cfg(windows)]
    #[test]
    fn manifest_dir_on_windows() {
        assert_eq!(manifest_dir(r"C:\work\creative.json"), r"C:\work");
        assert_eq!(manifest_dir(r"work\creative.json"), "work");
        assert_eq!(manifest_dir(r"a/b\creative.json"), "a/b");
        assert_eq!(manifest_dir("creative.json"), ".");
    }
}
