//! Host file helpers behind the `fs` feature: reading the creative files a
//! manifest names, and the clock. Hosts (the CLI and the language bindings)
//! share these so they all apply one confinement rule. The WebAssembly build
//! never enables this feature.

use crate::batch::check_manifest;
use crate::error::Error;
use crate::load::{decode_document, decode_manifest};
use crate::model::{CreativeFile, Document, Manifest};
use crate::probe::{Probe, Prober, Step};
use crate::report::{Options, Report};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path};

type Time = chrono::DateTime<chrono::FixedOffset>;

/// The local time, with its UTC offset.
pub fn host_now() -> Time {
    chrono::Local::now().fixed_offset()
}

/// `None` is the host's time now; otherwise an RFC 3339 time.
pub fn parse_now(now: Option<&str>) -> Result<Time, String> {
    match now {
        None => Ok(host_now()),
        Some(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map_err(|e| format!("now: {s:?} is not an RFC 3339 time: {e}")),
    }
}

/// A failure while probing one file, with the operation that failed.
#[derive(Debug)]
pub struct ProbeError {
    /// `open`, `stat`, `read` or `seek`.
    pub op: &'static str,
    pub source: std::io::Error,
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

impl std::error::Error for ProbeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Go's `*PathError` text, `op path: reason`, with the reason lower-cased
/// like Go's errno strings.
pub fn go_error(op: &str, path: &str, e: &std::io::Error) -> String {
    let mut why = e.to_string();
    if let Some(i) = why.find(" (os error") {
        why.truncate(i);
    }
    let mut chars = why.chars();
    let why = match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => why,
    };
    format!("{op} {path}: {why}")
}

/// Go's `filepath.Clean`, for showing joined paths as Go does.
pub fn clean_path(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let rooted = p.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else if !rooted {
                    out.push("..");
                }
            }
            x => out.push(x),
        }
    }
    let s = out.join("/");
    match (rooted, s.is_empty()) {
        (true, _) => format!("/{s}"),
        (false, true) => ".".into(),
        _ => s,
    }
}

fn err(op: &'static str) -> impl Fn(std::io::Error) -> ProbeError {
    move |source| ProbeError { op, source }
}

/// Hashes the file by streaming, then reads only the ranges the prober asks
/// for. Anything but a regular file is an error (checked before opening:
/// opening a FIFO blocks, and a device never ends).
pub fn probe_path(path: &Path) -> Result<Probe, ProbeError> {
    if !std::fs::metadata(path).map_err(err("stat"))?.is_file() {
        return Err(ProbeError {
            op: "stat",
            source: std::io::Error::other("not a regular file"),
        });
    }
    let mut f = File::open(path).map_err(err("open"))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(err("read"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut prober = Prober::new(size, hex);
    loop {
        match prober.next() {
            Step::Done(p) => return Ok(p),
            Step::Need { offset, len } => {
                f.seek(SeekFrom::Start(offset)).map_err(err("seek"))?;
                let mut chunk = Vec::with_capacity(len as usize);
                // A short read just means the file ended.
                (&mut f)
                    .take(u64::from(len))
                    .read_to_end(&mut chunk)
                    .map_err(err("read"))?;
                prober.feed(&chunk);
            }
        }
    }
}

/// A file error worded as Go's `ProbeFile` words it: the path is the
/// directory joined with the name, as given.
fn go_io(dir: &Path, name: &str, op: &str, e: &std::io::Error) -> String {
    let joined = clean_path(&format!("{}/{name}", dir.display()));
    go_error(op, &joined, e)
}

/// The probes of the files a manifest names under a directory.
#[derive(Debug, Default)]
pub struct DirProbes {
    /// Probe of each file that was read, keyed by the manifest's file name.
    pub probes: HashMap<String, Probe>,
    /// Names of files that aren't there; the manifest's declared values are
    /// used for them.
    pub missing: Vec<String>,
}

/// Probes each file the manifest names under `dir`. The manifest is
/// untrusted: a name that is empty, absolute or climbs out with `..`, a
/// symlink whose target leaves `dir`, and anything but a regular file are
/// errors. A file that isn't there is listed in `missing` (as is every file
/// when `dir` itself isn't there).
pub fn probe_dir(dir: &Path, m: &Manifest) -> Result<DirProbes, Error> {
    let mut out = DirProbes::default();
    let root = match std::fs::canonicalize(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            out.missing = m.files.iter().map(|f| f.file.clone()).collect();
            return Ok(out);
        }
        Err(e) => return Err(Error::msg(format!("{}: {e}", dir.display()))),
    };
    let one = |f: &CreativeFile| -> Result<Option<Probe>, Error> {
        let rel = Path::new(&f.file);
        let plain = rel
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
        if f.file.is_empty() || !plain {
            return Err(Error::msg(format!(
                "{}: file name must be relative and stay inside the directory",
                f.file
            )));
        }
        let path = match std::fs::canonicalize(root.join(rel)) {
            Ok(p) => p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(e) => {
                return Err(Error::msg(format!(
                    "{}: {}",
                    f.file,
                    go_io(dir, &f.file, "open", &e)
                )));
            }
        };
        if !path.starts_with(&root) {
            return Err(Error::msg(format!(
                "{}: resolves outside the directory",
                f.file
            )));
        }
        let meta = std::fs::metadata(&path)
            .map_err(|e| Error::msg(format!("{}: {}", f.file, go_io(dir, &f.file, "stat", &e))))?;
        if !meta.is_file() {
            return Err(Error::msg(format!("{}: not a regular file", f.file)));
        }
        let p = probe_path(&path).map_err(|e| {
            Error::msg(format!(
                "{}: {}",
                f.file,
                go_io(dir, &f.file, e.op, &e.source)
            ))
        })?;
        Ok(Some(p))
    };
    #[cfg(feature = "parallel")]
    let results: Vec<Result<Option<Probe>, Error>> = {
        use rayon::prelude::*;
        m.files.par_iter().map(one).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let results: Vec<Result<Option<Probe>, Error>> = m.files.iter().map(one).collect();
    // In manifest order, so the first error and the `missing` list are the
    // same however the work was scheduled.
    for (f, r) in m.files.iter().zip(results) {
        match r? {
            Some(p) => {
                out.probes.insert(f.file.clone(), p);
            }
            None => out.missing.push(f.file.clone()),
        }
    }
    Ok(out)
}

/// Probes the manifest's files under `dir` (see [`probe_dir`]), then runs
/// `check_manifest`.
pub fn check_dir(
    dir: &Path,
    docs: &[Document],
    m: &Manifest,
    opt: &Options,
) -> Result<(Vec<Report>, Vec<String>), Error> {
    let found = probe_dir(dir, m)?;
    check_manifest(docs, m, &found.probes, opt)
}

/// Decodes the specs and manifest, then runs `check_manifest`. The result is
/// `{"reports": [...], "notes": [...]}`; errors are the core's message.
pub fn run_check(
    specs: &[String],
    manifest: &str,
    probes: &HashMap<String, Probe>,
    now: Option<&str>,
) -> Result<serde_json::Value, String> {
    let docs = specs
        .iter()
        .map(|s| decode_document(s.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let m = decode_manifest(manifest.as_bytes()).map_err(|e| e.to_string())?;
    let opt = Options::new(parse_now(now)?);
    let (reports, notes) = check_manifest(&docs, &m, probes, &opt).map_err(|e| e.to_string())?;
    let reports = reports
        .iter()
        .map(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({"reports": reports, "notes": notes}))
}

/// [`probe_dir`] on a manifest given as text, as the bindings receive it.
pub fn probe_dir_text(dir: &str, manifest: &str) -> Result<DirProbes, String> {
    let m = decode_manifest(manifest.as_bytes()).map_err(|e| e.to_string())?;
    probe_dir(Path::new(dir), &m).map_err(|e| e.to_string())
}

/// [`run_check`] with the manifest's files probed under `dir`, for the
/// bindings' `checkDir`/`check_dir`. The result adds `"missing"`: the
/// manifest's file names that weren't found under `dir` and so were judged
/// on the manifest's own declared values (every one of them, if `dir`
/// doesn't exist). The CLI prints the same as notes.
pub fn check_dir_text(
    dir: &str,
    specs: &[String],
    manifest: &str,
    now: Option<&str>,
) -> Result<serde_json::Value, String> {
    let found = probe_dir_text(dir, manifest)?;
    let mut res = run_check(specs, manifest, &found.probes, now)?;
    res["missing"] = serde_json::json!(found.missing);
    Ok(res)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::model::CreativeFile;

    fn manifest(names: &[&str]) -> Manifest {
        Manifest {
            opencrs: "0.1".into(),
            campaign: None,
            files: names
                .iter()
                .map(|n| CreativeFile {
                    file: (*n).into(),
                    media_type: "image/png".into(),
                    ..CreativeFile::default()
                })
                .collect(),
            source: "".into(),
        }
    }

    fn run(dir: &Path, names: &[&str]) -> Result<DirProbes, String> {
        probe_dir(dir, &manifest(names)).map_err(|e| e.to_string())
    }

    #[test]
    fn reads_a_normal_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.bin"), b"hello").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub/b.bin"), b"hi").unwrap();
        let got = run(d.path(), &["a.bin", "./sub/b.bin"]).unwrap();
        assert_eq!(got.probes["a.bin"].bytes, 5);
        assert_eq!(got.probes["./sub/b.bin"].bytes, 2);
        assert!(got.missing.is_empty());
    }

    #[test]
    fn check_dir_text_lists_missing_files() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/harbourline");
        let spec = std::fs::read_to_string(root.join("retail.screens.json")).unwrap();
        let manifest = std::fs::read_to_string(root.join("creative.json")).unwrap();
        let now = Some("2026-10-22T12:00:00+11:00");
        let all = check_dir_text(
            root.to_str().unwrap(),
            std::slice::from_ref(&spec),
            &manifest,
            now,
        )
        .unwrap();
        assert_eq!(all["missing"], serde_json::json!([]));
        let d = tempfile::tempdir().unwrap();
        let none = check_dir_text(d.path().to_str().unwrap(), &[spec], &manifest, now).unwrap();
        assert_eq!(
            none["missing"],
            serde_json::json!(["lunch-portrait.png", "lunch-portrait-logo-right.png"])
        );
    }

    #[test]
    fn skips_a_missing_file() {
        let d = tempfile::tempdir().unwrap();
        let got = run(d.path(), &["gone.png"]).unwrap();
        assert!(got.probes.is_empty());
        assert_eq!(got.missing, ["gone.png"]);
        let got = run(&d.path().join("nodir"), &["gone.png"]).unwrap();
        assert_eq!(got.missing, ["gone.png"]);
    }

    #[test]
    fn rejects_absolute_and_parent_paths() {
        let d = tempfile::tempdir().unwrap();
        for name in ["/etc/hosts", "../x", "sub/../../x", "a/../b", ""] {
            let e = run(d.path(), &[name]).unwrap_err();
            assert!(e.contains("inside the directory"), "{name}: {e}");
        }
    }

    #[test]
    fn rejects_a_symlink_out_of_the_directory() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"x").unwrap();
        let d = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), d.path().join("out")).unwrap();
        let e = run(d.path(), &["out"]).unwrap_err();
        assert!(e.contains("outside the directory"), "{e}");
    }

    #[test]
    fn allows_a_symlink_within_the_directory() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("real"), b"abc").unwrap();
        std::os::unix::fs::symlink("real", d.path().join("link")).unwrap();
        assert_eq!(run(d.path(), &["link"]).unwrap().probes["link"].bytes, 3);
    }

    #[test]
    fn rejects_a_directory_instead_of_a_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        let e = run(d.path(), &["sub"]).unwrap_err();
        assert!(e.contains("not a regular file"), "{e}");
    }

    #[test]
    fn probe_path_rejects_non_regular_files() {
        let e = probe_path(Path::new("/dev/zero")).unwrap_err();
        assert!(e.to_string().contains("not a regular file"));
    }
}
