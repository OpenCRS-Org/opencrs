//! Holds the command line to `parity/operations.json`, which lists every
//! operation of the core and the name each caller gives it. Every name listed
//! for the CLI must be a subcommand (with its flag, if one is written), every
//! subcommand must be listed (as an operation or a helper), and an operation
//! the CLI has no name for needs an entry in `exceptions`. An operation added
//! to the file fails here until the CLI has it.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn manifest() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../parity/operations.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn every_operation_in_the_manifest_is_a_subcommand_and_nothing_else_is() {
    let m = manifest();
    let root = opencrs_cli::command();
    let mut listed: BTreeSet<String> = m["helpers"]["cli"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    for (op, entry) in m["operations"].as_object().unwrap() {
        for b in ["cli", "go", "js", "npm", "python"] {
            assert!(
                entry.get(b).is_some(),
                "{op}: the manifest says nothing for {b}"
            );
        }
        let excepted = m["exceptions"].get(format!("cli.{op}")).is_some();
        let Some(names) = entry["cli"].as_array() else {
            assert!(excepted, "{op}: the CLI has no name and no exception");
            continue;
        };
        assert!(!names.is_empty(), "{op}: cli needs a list of names or null");
        assert!(!excepted, "{op}: the CLI has names and an exception");
        for name in names {
            let name = name.as_str().unwrap();
            let mut words = name.split_whitespace();
            let sub = words.next().unwrap();
            let cmd = root
                .find_subcommand(sub)
                .unwrap_or_else(|| panic!("{op}: the CLI has no subcommand {sub}"));
            for flag in words {
                let long = flag
                    .strip_prefix("--")
                    .expect("a flag is written --like-this");
                assert!(
                    cmd.get_arguments().any(|a| a.get_long() == Some(long)),
                    "{op}: {sub} has no flag {flag}"
                );
            }
            listed.insert(sub.to_string());
        }
    }
    let have: BTreeSet<String> = root
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    assert_eq!(
        have, listed,
        "a subcommand is missing from parity/operations.json"
    );
}
