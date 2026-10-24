//! The man page and completions in the repository are what the command
//! definition generates (like the committed wasm module, which CI rebuilds
//! and compares). Fix with: cargo xtask generate

use std::path::Path;

#[test]
fn committed_files_are_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Regenerate into a temporary directory, then compare file by file.
    let tmp = tempfile::tempdir().unwrap();
    let files = opencrs_xtask::generate().unwrap();
    assert!(files.len() >= 5, "man page and four completions");
    let mut stale = Vec::new();
    for (path, text) in &files {
        let out = tmp.path().join(path);
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        std::fs::write(&out, text).unwrap();
        let committed = std::fs::read(root.join(path)).unwrap_or_default();
        if std::fs::read(&out).unwrap() != committed {
            stale.push(path.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "out of date: {stale:?}; run `cargo xtask generate` and commit the result"
    );
}
