use crate::cli::ProbeArgs;
use crate::io::probe_file;
use opencrs::json;
use opencrs::probe::Probe;
use serde::Serialize;
use std::io::Write;
use std::path::Path;

/// A probe with the file it is of, for `--json`.
#[derive(Serialize)]
struct Probed<'a> {
    file: &'a str,
    #[serde(flatten)]
    probe: &'a Probe,
}

pub fn run(a: &ProbeArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let mut status = 0;
    let mut probes = Vec::new();
    for path in &a.files {
        match probe_file(Path::new(path)) {
            Ok(p) => probes.push((path, p)),
            Err((_, e)) => {
                let _ = writeln!(stderr, "{path}: {e}");
                status = 2;
            }
        }
    }
    if a.json {
        let out: Vec<Probed> = probes
            .iter()
            .map(|(file, probe)| Probed { file, probe })
            .collect();
        let _ = stdout.write_all(json::to_string_pretty(&out).as_bytes());
        return status;
    }
    for (path, p) in &probes {
        let _ = writeln!(stdout, "{path}");
        let media = if p.media_type.is_empty() {
            "unknown"
        } else {
            &p.media_type
        };
        let _ = writeln!(stdout, "  type      {media}");
        if let Some([w, h]) = p.pixels {
            let _ = writeln!(stdout, "  pixels    {w} x {h}");
        }
        let _ = writeln!(stdout, "  bytes     {}", p.bytes);
        if let Some(d) = p.duration_s {
            let _ = writeln!(stdout, "  duration  {d} s");
        }
        let _ = writeln!(stdout, "  sha256    {}", p.sha256);
    }
    status
}
