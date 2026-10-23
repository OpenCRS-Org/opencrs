//! Reading files, with errors worded as Go's `os` package words them.

use std::fs::File;
use std::io::Read;
use std::path::Path;

pub use opencrs::fs::{clean_path, go_error};

/// Go's `os.ReadFile`.
pub fn read_file(path: &str) -> Result<Vec<u8>, (std::io::ErrorKind, String)> {
    let mut f = File::open(path).map_err(|e| (e.kind(), go_error("open", path, &e)))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)
        .map_err(|e| (e.kind(), go_error("read", path, &e)))?;
    Ok(buf)
}

/// Go's `crs.ProbeFile`: the file's hash and what its header says (see
/// `opencrs::fs::probe_path`), with errors worded like Go's.
pub fn probe_file(path: &Path) -> Result<opencrs::probe::Probe, (std::io::ErrorKind, String)> {
    opencrs::fs::probe_path(path).map_err(|e| {
        (
            e.source.kind(),
            go_error(e.op, &path.to_string_lossy(), &e.source),
        )
    })
}
