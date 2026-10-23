//! Running the conformance suite: loading cases, the reference validator
//! (in-process) and external command validators. Port of the file I/O half
//! of `conformance/conformance.go`.

use crate::gotime;
use crate::io::{go_error, probe_file, read_file};
use opencrs::batch::check_manifest;
use opencrs::conformance::{Case, LEVELS, compare};
use opencrs::report::Options;
use opencrs::{decode_document, decode_manifest, json};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Produces reports for a case as a JSON array, or says why it can't.
pub type Validator<'a> = &'a dyn Fn(&Case) -> Result<Vec<u8>, String>;

/// The outcome of one case.
pub struct CaseResult {
    pub case: Case,
    pub problems: Vec<String>,
    pub err: Option<String>,
}

impl CaseResult {
    pub fn passed(&self) -> bool {
        self.err.is_none() && self.problems.is_empty()
    }
}

/// Reads every case under `dir/cases`, sorted by id.
pub fn load(dir: &str) -> Result<Vec<Case>, String> {
    let cases_dir = Path::new(dir).join("cases");
    let shown = |p: &Path| crate::io::clean_path(&p.to_string_lossy());
    let mut paths = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&cases_dir) {
        for e in entries.flatten() {
            let p = e.path().join("case.json");
            if p.exists() {
                paths.push(p);
            }
        }
    }
    if paths.is_empty() {
        return Err(format!("no cases under {}", shown(&cases_dir)));
    }
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let name = shown(&p);
        let raw = read_file(&name).map_err(|(_, e)| e)?;
        let mut c: Case = serde_json::from_slice(&raw).map_err(|e| format!("{name}: {e}"))?;
        if !LEVELS.contains(&c.level.as_str()) {
            return Err(format!(
                "{name}: unknown level {}",
                opencrs::gofmt::quote(&c.level)
            ));
        }
        c.dir = std::path::PathBuf::from(crate::io::clean_path(
            &p.parent().unwrap().to_string_lossy(),
        ));
        out.push(c);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn in_dir(c: &Case, name: &str) -> String {
    c.dir.join(name).to_string_lossy().into_owned()
}

/// Runs the reference validator in-process. Creative files present in the
/// case directory are read; others are checked on the manifest's declared
/// values.
pub fn reference(c: &Case) -> Result<Vec<u8>, String> {
    let mut docs = Vec::new();
    for s in &c.specs {
        let raw = read_file(&in_dir(c, s)).map_err(|(_, e)| e)?;
        docs.push(decode_document(&raw).map_err(|e| format!("{s}: {e}"))?);
    }
    let raw = read_file(&in_dir(c, &c.creative)).map_err(|(_, e)| e)?;
    let m = decode_manifest(&raw).map_err(|e| format!("{}: {e}", c.creative))?;
    let mut probes = HashMap::new();
    for f in &m.files {
        match probe_file(&c.dir.join(&f.file)) {
            Ok(p) => {
                probes.insert(f.file.clone(), p);
            }
            Err((std::io::ErrorKind::NotFound, _)) => {}
            Err((_, e)) => return Err(e),
        }
    }
    let now = gotime::parse_rfc3339(&c.now).map_err(|e| format!("case now: {e}"))?;
    let (reports, _) =
        check_manifest(&docs, &m, &probes, &Options::new(now)).map_err(|e| e.to_string())?;
    Ok(json::to_string(&reports).into_bytes())
}

/// Runs an external validator: `cmd` is run by the shell with the case
/// directory as its last argument, and must print the reports as a JSON
/// array on stdout. A validator may exit non-zero when creative fails; only
/// its output counts.
pub fn command(cmd: &str) -> impl Fn(&Case) -> Result<Vec<u8>, String> {
    let cmd = cmd.to_string();
    move |c: &Case| {
        let abs = std::path::absolute(&c.dir).map_err(|e| go_error("getwd", "", &e))?;
        let out = Command::new("sh")
            .arg("-c")
            .arg(format!("{cmd} \"$1\""))
            .arg("sh")
            .arg(&abs)
            .output();
        let (run_err, stdout, stderr) = match out {
            Ok(o) => (
                exit_error(&o.status),
                o.stdout,
                String::from_utf8_lossy(&o.stderr).into_owned(),
            ),
            Err(e) => (
                Some(format!("fork/exec /bin/sh: {e}")),
                Vec::new(),
                String::new(),
            ),
        };
        if stdout.is_empty() {
            let why = run_err.unwrap_or_else(|| "no output".into());
            return Err(format!("{why}: {}", stderr.trim()));
        }
        Ok(stdout)
    }
}

fn exit_error(st: &std::process::ExitStatus) -> Option<String> {
    if st.success() {
        None
    } else if let Some(code) = st.code() {
        Some(format!("exit status {code}"))
    } else {
        Some(format!("signal: {}", signal(st)))
    }
}

#[cfg(unix)]
fn signal(st: &std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    st.signal().map_or("unknown".into(), |s| s.to_string())
}

// Windows has no signals: a process without an exit code is unknown.
#[cfg(not(unix))]
fn signal(_: &std::process::ExitStatus) -> String {
    "unknown".into()
}

/// Runs every case at or below `level` through `v`.
pub fn run(cases: &[Case], level: &str, v: Validator<'_>) -> Vec<CaseResult> {
    let mut out = Vec::new();
    for c in cases {
        if !c.in_level(level) {
            continue;
        }
        let (problems, err) = match v(c) {
            Ok(output) => (compare(c, &output), None),
            Err(e) => (Vec::new(), Some(e)),
        };
        out.push(CaseResult {
            case: c.clone(),
            problems,
            err,
        });
    }
    out
}
