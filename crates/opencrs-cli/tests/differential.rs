//! Differential test: runs the Go `opencrs` binary (the oracle) and this
//! crate's binary on the same inputs and compares what they print.
//!
//! Runs only when `OPENCRS_ORACLE` names the Go binary; see
//! `tests/differential/README.md`.

#[path = "common/server.rs"]
mod server;
use serde_json::Value;
use server::{Resp, Server};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const NOW: &str = "2026-10-22T12:00:00+11:00";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn oracle() -> Option<PathBuf> {
    match std::env::var_os("OPENCRS_ORACLE") {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ => {
            println!(
                "skipped: set OPENCRS_ORACLE to a Go opencrs binary to run the differential test"
            );
            None
        }
    }
}

/// One invocation to compare.
#[derive(Clone, Default)]
struct Case {
    args: Vec<String>,
    cwd: PathBuf,
    /// If set, an environment variable naming the *other* binary: each
    /// binary is run with the variable pointing at its counterpart.
    cross_var: Option<String>,
}

fn case(cwd: &Path, args: &[&str]) -> Case {
    Case {
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: cwd.to_path_buf(),
        ..Case::default()
    }
}

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(bin: &Path, other: &Path, c: &Case) -> Outcome {
    let mut cmd = Command::new(bin);
    cmd.args(&c.args).current_dir(&c.cwd);
    if let Some(var) = &c.cross_var {
        cmd.env(var, other);
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("{}: {e}", bin.display()));
    Outcome {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Rule (a): lines quoting a schema error are reduced to their instance
/// paths, and the count goes. Go reports a node whose own schema fails in
/// several ways (a `oneOf`) once, as "validation failed"; Rust reports the
/// failures below it, so paths under such a node fold into it.
fn normalise_text(s: &str, go: &str) -> String {
    let collapsed: Vec<String> = go
        .split('\n')
        .filter_map(|l| {
            let rest = l.strip_prefix("  /")?;
            let (path, msg) = rest.split_once(": ")?;
            (msg == "validation failed").then(|| format!("/{path}"))
        })
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut run: BTreeSet<String> = BTreeSet::new();
    let flush = |run: &mut BTreeSet<String>, out: &mut Vec<String>| {
        for p in std::mem::take(run) {
            out.push(format!("  schema error at {p}"));
        }
    };
    for line in s.split('\n') {
        if let Some(path) = schema_error_path(line) {
            let folded = collapsed
                .iter()
                .find(|c| path == **c || path.starts_with(&format!("{c}/")) || **c == "/")
                .cloned()
                .unwrap_or(path);
            run.insert(folded);
            continue;
        }
        flush(&mut run, &mut out);
        out.push(normalise_decode_error(&strip_error_count(line)));
    }
    flush(&mut run, &mut out);
    out.join("\n")
}

/// Rule (c): when a document matches the schema but can't be decoded into
/// the model (a number such as `1e9` for an integer field), the reason is
/// worded by Go's `encoding/json` or by serde. Only the fact is compared.
fn normalise_decode_error(line: &str) -> String {
    if let Some(i) = line.find("output isn't a JSON array of reports: ") {
        return format!("{}output isn't a JSON array of reports", &line[..i]);
    }
    if let Some(i) = line.find("not JSON: ") {
        return format!("{}not JSON", &line[..i]);
    }
    for marker in [
        ": json: cannot unmarshal ",
        ": invalid type: ",
        ": invalid value: ",
    ] {
        if let Some(i) = line.find(marker) {
            return format!("{}: decode error", &line[..i]);
        }
    }
    line.to_string()
}

fn schema_error_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("  /")?;
    let (path, _) = rest.split_once(": ")?;
    if path.contains(' ') {
        return None;
    }
    Some(format!("/{path}"))
}

fn strip_error_count(line: &str) -> String {
    // "<file>: 3 schema error(s) as a screens document"
    if let Some(i) = line.find(" schema error(s) as a ") {
        let head = &line[..i];
        if let Some(j) = head.rfind(": ") {
            return format!("{}: N{}", &head[..j], &line[i..]);
        }
    }
    line.to_string()
}

/// Rule (b): for `-json` output, compare the reports as JSON without
/// `validator` and `checked_at`.
fn normalise_stdout(args: &[String], s: &str, go: &str) -> String {
    if args.iter().any(|a| a == "-json" || a == "--json")
        && let Ok(mut v) = serde_json::from_str::<Value>(s)
    {
        if let Some(reps) = v.as_array_mut() {
            for r in reps {
                if let Some(o) = r.as_object_mut() {
                    o.remove("validator");
                    o.remove("checked_at");
                }
            }
        }
        return normalise_text(&serde_json::to_string_pretty(&v).unwrap(), go);
    }
    normalise_text(s, go)
}

fn compare(oracle: &Path, cases: &[Case]) {
    let rust = PathBuf::from(env!("CARGO_BIN_EXE_opencrs"));
    let mut diffs = Vec::new();
    for c in cases {
        let (g, r) = (run(oracle, &rust, c), run(&rust, oracle, c));
        let mut why = Vec::new();
        if g.code != r.code {
            why.push(format!("exit: go {} rust {}", g.code, r.code));
        }
        let (go_out, rs_out) = (
            normalise_stdout(&c.args, &g.stdout, &g.stdout),
            normalise_stdout(&c.args, &r.stdout, &g.stdout),
        );
        if go_out != rs_out {
            why.push(format!("stdout:\n--- go\n{go_out}\n--- rust\n{rs_out}"));
        }
        let (go_err, rs_err) = (
            normalise_text(&g.stderr, &g.stderr),
            normalise_text(&r.stderr, &g.stderr),
        );
        if go_err != rs_err {
            why.push(format!("stderr:\n--- go\n{go_err}\n--- rust\n{rs_err}"));
        }
        if !why.is_empty() {
            diffs.push(format!(
                "in {}: opencrs {}\n{}",
                c.cwd.display(),
                c.args.join(" "),
                why.join("\n")
            ));
        }
    }
    println!(
        "compared {} invocations, {} differ",
        cases.len(),
        diffs.len()
    );
    if !diffs.is_empty() {
        if let Some(p) = std::env::var_os("OPENCRS_DIFF_LOG") {
            let _ = std::fs::write(p, diffs.join("\n\n"));
        }
        let shown = diffs
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n");
        panic!(
            "{} of {} invocations differ; first ones:\n\n{shown}",
            diffs.len(),
            cases.len()
        );
    }
}

// ---- inputs ----

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            json_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "json") {
            out.push(p);
        }
    }
}

/// Every reference a screens.json defines: its formats and its screens.
fn refs(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let publisher = doc["publisher"].as_str().unwrap_or("");
    for n in doc["networks"].as_array().into_iter().flatten() {
        let net = n["id"].as_str().unwrap_or("");
        for f in n["formats"].as_array().into_iter().flatten() {
            out.push(format!(
                "{publisher}/{net}/{}",
                f["id"].as_str().unwrap_or("")
            ));
        }
        for sc in n["screens"].as_array().into_iter().flatten() {
            out.push(format!(
                "{publisher}/{net}/screen/{}",
                sc["id"].as_str().unwrap_or("")
            ));
        }
    }
    out
}

fn is_screens(v: &Value) -> bool {
    v.get("networks").is_some() && v.get("publisher").is_some_and(Value::is_string)
}

fn is_creative(v: &Value) -> bool {
    v.get("files").is_some_and(Value::is_array)
}

/// `schema` on a file, and `resolve` for each of `ref_limit` references.
fn doc_cases(root: &Path, file: &Path, ref_limit: usize, out: &mut Vec<Case>) {
    out.push(case(root, &["schema", &s(file)]));
    let Ok(raw) = std::fs::read(file) else { return };
    let Ok(v) = serde_json::from_slice::<Value>(&raw) else {
        return;
    };
    if is_screens(&v) {
        let all = refs(&v);
        let step = (all.len() / ref_limit.max(1)).max(1);
        for r in all.iter().step_by(step) {
            out.push(case(root, &["resolve", &s(file), r]));
        }
        out.push(case(
            root,
            &["resolve", &s(file), "nobody.example/nothing/none"],
        ));
    }
}

/// `check` of a creative.json against the screens.json beside it, as text and
/// as JSON.
fn check_cases(cwd: &Path, specs: &[String], creative: &str, extra: &[&str], out: &mut Vec<Case>) {
    for json in [false, true] {
        let mut args = vec!["check".to_string()];
        for sp in specs {
            args.push("-spec".into());
            args.push(sp.clone());
        }
        args.extend(["-now".into(), NOW.into()]);
        args.extend(extra.iter().map(|e| e.to_string()));
        if json {
            args.push("-json".into());
        }
        args.push(creative.into());
        out.push(Case {
            args,
            cwd: cwd.to_path_buf(),
            ..Case::default()
        });
    }
}

fn example_cases(check: bool) -> Vec<Case> {
    let root = root();
    let mut files = Vec::new();
    json_files(&root.join("examples"), &mut files);
    let mut cases = Vec::new();
    let mut screens = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap().to_path_buf();
        if !check {
            doc_cases(&root, &rel, 12, &mut cases);
        }
        if let Ok(v) = serde_json::from_slice::<Value>(&std::fs::read(f).unwrap())
            && is_screens(&v)
        {
            screens.push(s(&rel));
        }
    }
    if check {
        for f in &files {
            let rel = f.strip_prefix(&root).unwrap();
            if let Ok(v) = serde_json::from_slice::<Value>(&std::fs::read(f).unwrap())
                && is_creative(&v)
            {
                check_cases(&root, &screens, &s(rel), &[], &mut cases);
                check_cases(&root, &screens, &s(rel), &["-no-files"], &mut cases);
            }
        }
    }
    cases
}

fn conformance_cases(check: bool) -> Vec<Case> {
    let root = root();
    let base = root.join("conformance/0.1/cases");
    let mut dirs: Vec<_> = std::fs::read_dir(&base)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    dirs.sort();
    let mut cases = Vec::new();
    for dir in dirs.iter().filter(|d| d.join("case.json").exists()) {
        let spec: Value =
            serde_json::from_slice(&std::fs::read(dir.join("case.json")).unwrap()).unwrap();
        if check {
            // As the adapter in conformance/README.md does.
            let now = spec["now"].as_str().unwrap();
            let specs: Vec<String> = spec["specs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
            let mut args = vec![
                "check".to_string(),
                "-json".into(),
                "-now".into(),
                now.into(),
            ];
            for sp in &specs {
                args.push("-spec".into());
                args.push(sp.clone());
            }
            args.push(spec["creative"].as_str().unwrap().into());
            cases.push(Case {
                args,
                cwd: dir.clone(),
                ..Case::default()
            });
            check_cases(
                dir,
                &specs,
                spec["creative"].as_str().unwrap(),
                &[],
                &mut cases,
            );
        } else {
            let mut files = Vec::new();
            json_files(dir, &mut files);
            for f in files
                .iter()
                .filter(|f| f.file_name().is_some_and(|n| n != "case.json"))
            {
                doc_cases(&root, f, 12, &mut cases);
            }
        }
    }
    cases
}

fn core_data_cases() -> Vec<Case> {
    let root = root();
    let mut files = Vec::new();
    json_files(&root.join("crates/opencrs/tests/data"), &mut files);
    let mut cases = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap();
        doc_cases(&root, rel, 12, &mut cases);
    }
    cases
}

/// The bench network: 20000 screens in 400 formats, one creative file per
/// format. Returns `None` if `uv` isn't available.
fn bench_cases(tmp: &Path, check: bool) -> Option<Vec<Case>> {
    let root = root();
    let dir = tmp.join("bench");
    std::fs::create_dir_all(&dir).unwrap();
    let status = Command::new("uv")
        .current_dir(&root)
        .args(["run", "bench/gen-network.py"])
        .arg(&dir)
        .args(["20000", "400", "examples/harbourline/lunch-portrait.png"])
        .status();
    match status {
        Ok(st) if st.success() => {}
        other => {
            // In CI a skipped 20k-screen comparison would go unnoticed.
            assert!(
                std::env::var_os("CI").is_none(),
                "uv run bench/gen-network.py failed in CI: {other:?}"
            );
            println!("skipped the bench network: uv run bench/gen-network.py failed ({other:?})");
            return None;
        }
    }
    let mut cases = Vec::new();
    for name in ["screens.json", "creative.json"] {
        let f = dir.join(name);
        if !check {
            doc_cases(&root, &f, 6, &mut cases);
        }
    }
    if check {
        let specs = vec![s(&dir.join("screens.json"))];
        for creative in ["creative.json", "creative-screens.json"] {
            if dir.join(creative).exists() {
                check_cases(&root, &specs, &s(&dir.join(creative)), &[], &mut cases);
            }
        }
    }
    Some(cases)
}

// ---- mutations ----

#[derive(Clone, Debug)]
enum Seg {
    Key(String),
    Idx(usize),
}

fn walk(v: &Value, path: &mut Vec<Seg>, out: &mut Vec<(Vec<Seg>, Value)>) {
    out.push((path.clone(), v.clone()));
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                path.push(Seg::Key(k.clone()));
                walk(x, path, out);
                path.pop();
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                path.push(Seg::Idx(i));
                walk(x, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

fn at_mut<'a>(v: &'a mut Value, path: &[Seg]) -> &'a mut Value {
    let mut cur = v;
    for seg in path {
        cur = match seg {
            Seg::Key(k) => cur.get_mut(k).unwrap(),
            Seg::Idx(i) => cur.get_mut(*i).unwrap(),
        };
    }
    cur
}

const RAW_1E9: &str = "@@RAW-1E9@@";

/// Every mutation of a document, in a fixed order, as JSON text with a label.
fn mutations(doc: &Value) -> Vec<(String, String)> {
    let mut nodes = Vec::new();
    walk(doc, &mut Vec::new(), &mut nodes);
    let mut out = Vec::new();
    let render = |v: &Value| {
        serde_json::to_string_pretty(v)
            .unwrap()
            .replace(&format!("\"{RAW_1E9}\""), "1e9")
    };
    for (path, val) in nodes.iter().filter(|(p, _)| !p.is_empty()) {
        let name = path
            .iter()
            .map(|s| match s {
                Seg::Key(k) => k.clone(),
                Seg::Idx(i) => i.to_string(),
            })
            .collect::<Vec<_>>()
            .join(".");
        let last_key = match path.last() {
            Some(Seg::Key(k)) => Some(k.as_str()),
            _ => None,
        };
        let parent_is_targets =
            matches!(path.iter().rev().nth(1), Some(Seg::Key(k)) if k == "targets");
        // A field removed.
        if let Some(Seg::Key(k)) = path.last() {
            let mut d = doc.clone();
            at_mut(&mut d, &path[..path.len() - 1])
                .as_object_mut()
                .unwrap()
                .remove(k);
            out.push((format!("remove {name}"), render(&d)));
        }
        // A number replaced.
        if val.is_number() {
            for (label, repl) in [
                ("-1", Value::from(-1)),
                ("0", Value::from(0)),
                ("1e9", Value::from(RAW_1E9)),
                ("string", Value::from("seven")),
            ] {
                let mut d = doc.clone();
                *at_mut(&mut d, path) = repl;
                out.push((format!("{name} = {label}"), render(&d)));
            }
        }
        // An id reference pointed nowhere.
        if val.is_string() && (matches!(last_key, Some("format" | "venue")) || parent_is_targets) {
            let mut d = doc.clone();
            *at_mut(&mut d, path) = Value::from("missing-id");
            out.push((format!("{name} = missing-id"), render(&d)));
        }
        // Text outside ASCII.
        if val.is_string() && matches!(last_key, Some("text" | "id" | "name")) {
            let mut d = doc.clone();
            *at_mut(&mut d, path) = Value::from("Ünïcödé text");
            out.push((format!("{name} = unicode"), render(&d)));
        }
    }
    out
}

fn mutation_cases(tmp: &Path, check: bool) -> Vec<Case> {
    let root = root();
    let ex = root.join("examples/harbourline");
    let screens_path = ex.join("retail.screens.json");
    let creative_path = ex.join("creative.json");
    let screens: Value = serde_json::from_slice(&std::fs::read(&screens_path).unwrap()).unwrap();
    let creative: Value = serde_json::from_slice(&std::fs::read(&creative_path).unwrap()).unwrap();
    let dir = tmp.join("mutants");
    std::fs::create_dir_all(&dir).unwrap();
    let all_refs = refs(&screens);
    let mut cases = Vec::new();

    for (i, (label, text)) in mutations(&screens).iter().enumerate() {
        let f = dir.join(format!("s{i:04}.screens.json"));
        std::fs::write(&f, text).unwrap();
        println!("mutant s{i:04}: {label}");
        if check {
            check_cases(&root, &[s(&f)], &s(&creative_path), &[], &mut cases);
        } else {
            cases.push(case(&root, &["schema", &s(&f)]));
            for r in &all_refs {
                cases.push(case(&root, &["resolve", &s(&f), r]));
            }
        }
    }
    for (i, (label, text)) in mutations(&creative).iter().enumerate() {
        let f = dir.join(format!("c{i:04}.creative.json"));
        std::fs::write(&f, text).unwrap();
        println!("mutant c{i:04}: {label}");
        if check {
            let files = s(&ex);
            check_cases(
                &root,
                &[s(&screens_path)],
                &s(&f),
                &["-files", &files],
                &mut cases,
            );
        } else {
            cases.push(case(&root, &["schema", &s(&f)]));
        }
    }
    cases
}

// ---- tests ----

#[test]
fn usage_and_flags() {
    let Some(o) = oracle() else { return };
    let r = root();
    let e = "examples/harbourline";
    let screens = format!("{e}/retail.screens.json");
    let creative = format!("{e}/creative.json");
    let cases: Vec<Case> = [
        vec![],
        vec!["help"],
        vec!["-h"],
        vec!["--help"],
        vec!["-help"],
        vec!["nope"],
        vec!["schema"],
        vec!["schema", "-h"],
        vec!["schema", "-nope", &screens],
        vec!["schema", "-kind"],
        vec!["schema", "-kind", "bogus", &screens],
        vec!["schema", "-kind=screens", "--allow-local-http", &screens],
        vec!["schema", "-allow-local-http=maybe", &screens],
        vec!["schema", "---x"],
        vec!["schema", "-=x"],
        vec!["schema", "no/such/file.json", &screens],
        vec!["schema", "crates"],
        vec!["resolve"],
        vec!["resolve", &screens],
        vec!["resolve", "no/such/file.json", "a/b/c"],
        vec!["resolve", &screens, "not-a-ref"],
        vec![
            "resolve",
            &screens,
            "other.example/retail/portrait-totem-75",
        ],
        vec![
            "resolve",
            &screens,
            "harbourline.example/nope/portrait-totem-75",
        ],
        vec!["resolve", &screens, "harbourline.example/retail/nope"],
        vec!["resolve", &creative, "harbourline.example/retail/nope"],
        vec!["resolve", "resolve", "x"],
        vec!["check"],
        vec!["check", "-h"],
        vec!["check", "-nope"],
        vec!["check", "-margin", "abc", "-spec", &screens, &creative],
        vec!["check", "-margin", "1e999", "-spec", &screens, &creative],
        vec!["check", "-json=maybe", "-spec", &screens, &creative],
        vec!["check", "-now"],
        vec!["check", &creative],
        vec!["check", "-spec", &screens],
        vec!["check", "-spec", &screens, &creative, "extra"],
        vec!["check", "-spec", "no/such/spec.json", &creative],
        vec!["check", "-spec", &creative, &creative],
        vec!["check", "-spec", &screens, "no/such/creative.json"],
        vec!["check", "-spec", &screens, &screens],
        vec!["check", "-spec", &screens, "-now", "yesterday", &creative],
        vec![
            "check",
            "-spec",
            &screens,
            "-now",
            "2026-13-01T00:00:00Z",
            &creative,
        ],
        vec![
            "check",
            "-spec",
            &screens,
            "-now",
            "2026-02-30T00:00:00Z",
            &creative,
        ],
        vec![
            "check",
            "-spec",
            &screens,
            "-now",
            "2026-10-22T12:00:00",
            &creative,
        ],
        vec![
            "check",
            "-spec",
            &screens,
            "-now",
            "2026-10-22 12:00:00Z",
            &creative,
        ],
        vec![
            "check",
            "-spec",
            &screens,
            "-now",
            "2026-10-22T25:00:00Z",
            &creative,
        ],
        vec![
            "check",
            "-spec",
            &screens,
            "-now",
            "2026-10-22T12:00:00+25:00",
            &creative,
        ],
        vec![
            "check",
            "-spec",
            &screens,
            "-now",
            "2026-10-22T12:00:00Zjunk",
            &creative,
        ],
        vec![
            "check",
            "-spec",
            &screens,
            "-now",
            "2026-10-22T12:00:00.123456789012Z",
            "-h",
        ],
    ]
    .iter()
    .map(|a| case(&r, a))
    .collect();
    compare(&o, &cases);
}

#[test]
fn examples() {
    let Some(o) = oracle() else { return };
    compare(&o, &example_cases(false));
}

#[test]
fn conformance_documents() {
    let Some(o) = oracle() else { return };
    compare(&o, &conformance_cases(false));
}

#[test]
fn core_test_data() {
    let Some(o) = oracle() else { return };
    compare(&o, &core_data_cases());
}

#[test]
fn bench_network() {
    let Some(o) = oracle() else { return };
    let tmp = tempfile::tempdir().unwrap();
    if let Some(cases) = bench_cases(tmp.path(), false) {
        compare(&o, &cases);
    }
}

#[test]
fn mutated_documents() {
    let Some(o) = oracle() else { return };
    let tmp = tempfile::tempdir().unwrap();
    compare(&o, &mutation_cases(tmp.path(), false));
}

// The `check` command, over the same inputs.

#[test]
fn check_examples() {
    let Some(o) = oracle() else { return };
    compare(&o, &example_cases(true));
}

#[test]
fn check_conformance_cases() {
    let Some(o) = oracle() else { return };
    compare(&o, &conformance_cases(true));
}

#[test]
fn check_bench_network() {
    let Some(o) = oracle() else { return };
    let tmp = tempfile::tempdir().unwrap();
    if let Some(cases) = bench_cases(tmp.path(), true) {
        compare(&o, &cases);
    }
}

#[test]
fn check_mutated_documents() {
    let Some(o) = oracle() else { return };
    let tmp = tempfile::tempdir().unwrap();
    compare(&o, &mutation_cases(tmp.path(), true));
}

// ---- fetch ----

fn harbourline() -> Value {
    serde_json::from_slice(
        &std::fs::read(root().join("examples/harbourline/retail.screens.json")).unwrap(),
    )
    .unwrap()
}

/// A copy of the retail document with its one network renamed, so the two
/// can be merged.
fn partner_doc(network: &str) -> Value {
    let mut v = harbourline();
    v["networks"][0]["id"] = network.into();
    v["networks"][0]["name"] = format!("Partner {network}").into();
    v
}

#[test]
fn fetch() {
    let Some(o) = oracle() else { return };
    let r = root();
    let public = harbourline().to_string();
    let partner = partner_doc("private").to_string();
    let mut expired = harbourline();
    expired["expires_at"] = "2020-01-01T00:00:00Z".into();
    let expired = expired.to_string();
    let mut foreign = harbourline();
    foreign["publisher"] = "other.example".into();
    let foreign = foreign.to_string();

    let main = Server::start(move |req, addr| match req.path.as_str() {
        "/.well-known/opencrs.json" => Resp::ok(format!(
            r#"{{"opencrs":"0.1","publisher":{{"id":"harbourline.example","name":"Harbourline"}},
 "documents":[
  {{"url":"http://{addr}/retail.screens.json","network":"retail"}},
  {{"url":"http://{addr}/private.screens.json","visibility":"partner"}},
  {{"url":"http://{addr}/expired.screens.json"}},
  {{"url":"http://{addr}/foreign.screens.json"}},
  {{"url":"http://{addr}/missing.screens.json"}},
  {{"url":"http://{addr}/broken.json"}},
  {{"url":"http://{addr}/redirected.json"}},
  {{"url":"http://{addr}/forbidden.json","visibility":"partner"}}],
 "delegates":[{{"domain":"nowhere.example","scope":"bogus"}}]}}"#
        )),
        "/retail.screens.json" => Resp::ok(public.clone())
            .header("ETag", "\"v1\"")
            .header("Cache-Control", "max-age=3600")
            .header("Last-Modified", "Tue, 20 Oct 2026 09:00:00 GMT"),
        "/private.screens.json" if req.authorization == "Bearer s3cret" => {
            Resp::ok(partner.clone())
        }
        "/private.screens.json" | "/forbidden.json" => Resp::status(403),
        "/expired.screens.json" => Resp::ok(expired.clone()),
        "/foreign.screens.json" => Resp::ok(foreign.clone()),
        "/broken.json" => Resp::ok("{\"publisher\":\"harbourline.example\",\"networks\":3}"),
        "/redirected.json" => Resp::redirect("/retail.screens.json"),
        _ => Resp::status(404),
    });
    let host = main.addr.clone();
    let out = tempfile::tempdir().unwrap();
    let o_dir = s(out.path());
    let token = format!("{host}=s3cret");
    let bad_token = format!("{host}=wrong");
    let empty = Server::start(|_, _| Resp::status(404));

    let target = empty.url();
    let hop = Server::start(move |_, _| Resp::redirect(&format!("{target}/x")));
    let loopy = Server::start(|_, _| Resp::redirect("/.well-known/opencrs.json"));
    let big = Server::start(|_, _| Resp::ok(vec![b'x'; (50 << 20) + 1]));
    let cases: Vec<Case> = [
        vec!["fetch", "-allow-local-http", &hop.addr],
        vec!["fetch", "-allow-local-http", &loopy.addr],
        vec!["fetch", "-allow-local-http", &big.addr],
        vec!["fetch", "-allow-local-http", &host],
        vec!["fetch", "-allow-local-http", "-json", &host],
        vec!["fetch", "-allow-local-http", "-token", &token, &host],
        vec![
            "fetch",
            "-allow-local-http",
            "-token",
            &token,
            "-json",
            &host,
        ],
        vec!["fetch", "-allow-local-http", "-token", &bad_token, &host],
        vec![
            "fetch",
            "-allow-local-http",
            "-token",
            &token,
            "-o",
            &o_dir,
            &host,
        ],
        vec!["fetch", "-allow-local-http", &host, "-o", &o_dir],
        vec!["fetch", &format!("http://{host}/retail.screens.json")],
        vec!["fetch", "-allow-local-http", &empty.addr],
        vec!["fetch", "-allow-local-http", "-h"],
        vec!["fetch", "-allow-local-http", "-token", "nope", &host],
        vec!["fetch", "-allow-local-http", "-token", "=x", &host],
        vec!["fetch"],
        vec!["fetch", "-allow-local-http", &host, "extra"],
        vec![
            "fetch",
            "-allow-local-http",
            &format!("http://{host}/nothing.json"),
        ],
    ]
    .iter()
    .map(|a| case(&r, a))
    .collect();
    compare(&o, &cases);
}

// ---- conformance ----

/// The README's adapter, run through whichever binary `$OPENCRS_UNDER_TEST`
/// names. Needs `jq`.
const ADAPTER: &str = r#"#!/bin/sh
dir="$1"
now=$(jq -r .now "$dir/case.json")
specs=$(jq -r '.specs[] | "-spec " + .' "$dir/case.json")
cd "$dir" && exec "$OPENCRS_UNDER_TEST" check -json -now "$now" $specs "$(jq -r .creative case.json)"
"#;

#[test]
fn conformance() {
    let Some(o) = oracle() else { return };
    let r = root();
    let tmp = tempfile::tempdir().unwrap();

    // A suite where some cases fail: expectations changed.
    let broken = tmp.path().join("broken");
    copy_dir(&r.join("conformance/0.1"), &broken);
    for (id, edit) in [
        ("core-pass", "result"),
        ("core-pixels", "code"),
        ("full-max-words", "drop"),
    ] {
        let p = broken.join("cases").join(id).join("case.json");
        let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        match edit {
            "result" => v["expect"][0]["result"] = "fail".into(),
            "code" => v["expect"][0]["checks"][0]["code"] = "CRS-NOPE".into(),
            _ => v["expect"][0]["checks"] = Value::Array(vec![]),
        }
        std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }
    // An unknown level and an empty directory.
    let bad_level = tmp.path().join("bad-level");
    copy_dir(&r.join("conformance/0.1"), &bad_level);
    let p = bad_level.join("cases/core-pass/case.json");
    let t = std::fs::read_to_string(&p)
        .unwrap()
        .replace("\"level\": \"core\"", "\"level\": \"bogus\"");
    std::fs::write(&p, t).unwrap();
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();

    let adapter = tmp.path().join("adapter.sh");
    std::fs::write(&adapter, ADAPTER).unwrap();
    let adapter_cmd = format!("sh {}", s(&adapter));
    let (broken, bad_level, empty) = (s(&broken), s(&bad_level), s(&empty));

    let mut cases: Vec<Case> = [
        vec!["conformance"],
        vec!["conformance", "-v"],
        vec!["conformance", "-level", "core"],
        vec!["conformance", "-level=layout", "-v"],
        vec!["conformance", "-level", "bogus"],
        vec!["conformance", "-h"],
        vec!["conformance", "-nope"],
        vec!["conformance", "conformance/0.1"],
        vec!["conformance", "./conformance/0.1/"],
        vec!["conformance", "a", "b"],
        vec!["conformance", "no/such/dir"],
        vec!["conformance", &empty],
        vec!["conformance", &bad_level],
        vec!["conformance", &broken],
        vec!["conformance", "-v", &broken],
        vec!["conformance", "-level", "core", &broken],
        vec!["conformance", "-cmd", "echo '[]' #"],
        vec!["conformance", "-cmd", "exit 3"],
        vec!["conformance", "-cmd", "echo oops >&2; exit 1"],
        vec!["conformance", "-cmd", "echo 'not json'"],
    ]
    .iter()
    .map(|a| case(&r, a))
    .collect();
    // Each binary runs the other through the adapter.
    for level in ["core", "layout", "full"] {
        let mut c = case(&r, &["conformance", "-level", level, "-cmd", &adapter_cmd]);
        c.cross_var = Some("OPENCRS_UNDER_TEST".into());
        cases.push(c);
    }
    let mut c = case(&r, &["conformance", "-v", "-cmd", &adapter_cmd, &broken]);
    c.cross_var = Some("OPENCRS_UNDER_TEST".into());
    cases.push(c);
    compare(&o, &cases);
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

#[cfg(unix)]
#[test]
fn unreadable_creative_file() {
    use std::os::unix::fs::PermissionsExt;
    let Some(o) = oracle() else { return };
    let r = root();
    let dir = tempfile::tempdir().unwrap();
    let ex = r.join("examples/harbourline");
    for f in [
        "creative.json",
        "lunch-portrait.png",
        "lunch-portrait-logo-right.png",
    ] {
        std::fs::copy(ex.join(f), dir.path().join(f)).unwrap();
    }
    let png = dir.path().join("lunch-portrait.png");
    std::fs::set_permissions(&png, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&png).is_ok() {
        return; // root
    }
    let creative = s(&dir.path().join("creative.json"));
    let screens = s(&ex.join("retail.screens.json"));
    // A manifest path with doubled slashes, too: Go cleans the directory.
    let odd = creative.replacen("/creative.json", "//creative.json", 1);
    let cases = vec![
        case(&r, &["check", "-spec", &screens, &creative]),
        case(&r, &["check", "-spec", &screens, &odd]),
    ];
    compare(&o, &cases);
}
