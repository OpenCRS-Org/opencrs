//! Port of `conformance/conformance_test.go` (the file-reading half; the
//! comparison tests live in the core).

use opencrs::{Kind, SchemaOptions, batch::lint};
use opencrs_cli::conformance::{command, load, reference, run};
use std::path::PathBuf;

fn suite() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/0.1")
        .to_string_lossy()
        .into_owned()
}

/// The reference validator must pass the whole suite at the Full level.
#[test]
fn reference_validator_conforms() {
    let cases = load(&suite()).unwrap();
    assert!(cases.len() >= 25, "only {} cases", cases.len());
    for r in run(&cases, "full", &reference) {
        assert!(r.err.is_none(), "{}: {:?}", r.case.id, r.err);
        assert!(r.problems.is_empty(), "{}: {:?}", r.case.id, r.problems);
    }
}

/// Every case's inputs must be valid documents, its level must match its
/// id, and it must cite the spec.
#[test]
fn cases_are_well_formed() {
    for c in load(&suite()).unwrap() {
        assert!(
            c.id.starts_with(&format!("{}-", c.level)),
            "{}: id should start with its level, {}",
            c.id,
            c.level
        );
        assert!(
            !c.spec.is_empty() && !c.description.is_empty(),
            "{}: needs spec and description",
            c.id
        );
        assert_eq!(
            c.is_optional(),
            !c.strictest.is_empty(),
            "{}: strictest cases, and only they, are optional",
            c.id
        );
        for name in c
            .specs
            .iter()
            .chain([&c.creative])
            .filter(|n| !n.is_empty())
        {
            let raw = std::fs::read(c.dir.join(name)).unwrap_or_else(|e| panic!("{}: {e}", c.id));
            let res = lint(&raw, None::<Kind>, &SchemaOptions::default())
                .unwrap_or_else(|e| panic!("{}/{name}: {e}", c.id));
            assert!(res.schema_errors.is_empty(), "{}/{name}: {res:?}", c.id);
        }
    }
}

#[test]
fn load_errors() {
    let dir = tempfile::tempdir().unwrap();
    let e = load(dir.path().to_str().unwrap()).unwrap_err();
    assert!(
        e.starts_with("no cases under ") && e.ends_with("/cases"),
        "{e}"
    );
    let case = dir.path().join("cases/x");
    std::fs::create_dir_all(&case).unwrap();
    std::fs::write(case.join("case.json"), r#"{"id":"x-1","level":"bogus"}"#).unwrap();
    let e = load(dir.path().to_str().unwrap()).unwrap_err();
    assert!(e.ends_with(": unknown level \"bogus\""), "{e}");
}

/// An external validator is run with the case directory as its argument.
/// `-cmd` runs through `sh`, so this needs a Unix shell.
#[cfg(unix)]
#[test]
fn command_validator() {
    let cases = load(&suite()).unwrap();
    let target: Vec<_> = cases
        .into_iter()
        .filter(|c| c.id == "core-other-publisher")
        .collect();
    // This stand-in validator prints no reports, which is right for this case.
    let ok = command(r#"test -f "$1/case.json" && echo "[]"; :"#);
    let rs = run(&target, "full", &ok);
    assert!(rs[0].passed(), "{:?} {:?}", rs[0].err, rs[0].problems);
    let silent = command("exit 3; :");
    let rs = run(&target, "full", &silent);
    assert!(
        rs[0].err.is_some(),
        "a validator that prints nothing should be an error"
    );
}
