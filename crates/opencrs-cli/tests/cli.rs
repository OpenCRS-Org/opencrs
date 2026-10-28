//! Port of the `schema`, `check` and `resolve` tests in `cmd/opencrs/main_test.go`.

use std::path::PathBuf;

#[path = "common/server.rs"]
mod server;
use server::{Resp, Server};

fn ex(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/harbourline")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn run_cli(args: &[&str]) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = opencrs_cli::run(&args, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn schema_command() {
    let (code, out, _) = run_cli(&[
        "schema",
        &ex("retail.screens.json"),
        &ex("creative.json"),
        &ex(".well-known/opencrs.json"),
        &ex("reports.json"),
    ]);
    assert!(
        code == 0 && out.matches("valid").count() == 4,
        "code {code}\n{out}"
    );
}

#[test]
fn schema_command_reports_rule_conflicts() {
    let doc = r#"{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-10-20T09:00:00Z","networks":[{"id":"n","name":"N",
	  "rules":{"required_elements":[{"role":"legal","within_rect_px":[0,1700,1080,220]}]},
	  "venues":[{"id":"v","name":"V","rules":{"required_elements":[{"role":"legal","within_rect_px":[0,0,1080,200]}]}}],
	  "formats":[{"id":"f","pixels":[1080,1920],"media":{"static":["image/png"]}}],
	  "screens":[{"id":"s1","format":"f","venue":"v"},{"id":"s2","format":"f","venue":"v"},{"id":"s3","format":"f"},{"id":"s4","format":"nope"}]}]}"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.screens.json");
    std::fs::write(&path, doc).unwrap();
    let (code, out, _) = run_cli(&["schema", path.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    for want in [
        "2 document error(s)",
        "CRS-RULE-CONFLICT  venue v restates the legal element within [0 0 1080 200], which doesn't overlap the inherited [0 1700 1080 220]; no creative can comply  (network n, venue v)",
        "CRS-REFERENCE      screen \"s4\" uses format \"nope\"",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
}

#[test]
fn schema_command_reports_schema_errors_and_unreadable_files() {
    let (code, out, _) = run_cli(&["schema", "-kind", "index", &ex("creative.json")]);
    assert_eq!(code, 1);
    assert!(out.contains("schema error(s) as a index document"), "{out}");
    let (code, _, err) = run_cli(&["schema", "-kind", "bogus", &ex("creative.json")]);
    assert_eq!(code, 2);
    assert!(
        err.ends_with(": unknown document kind \"bogus\"\n"),
        "{err}"
    );
    let (code, _, err) = run_cli(&["schema", "/no/such/file.json"]);
    assert_eq!(code, 2);
    // The OS supplies the reason, as it does for Go.
    let reason = if cfg!(windows) {
        "the system cannot find the path specified."
    } else {
        "no such file or directory"
    };
    assert_eq!(
        err,
        format!("/no/such/file.json: open /no/such/file.json: {reason}\n")
    );
}

#[test]
fn check_command() {
    let (code, out, stderr) = run_cli(&[
        "check",
        "-spec",
        &ex("retail.screens.json"),
        "-now",
        "2026-10-22T12:00:00Z",
        &ex("creative.json"),
    ]);
    assert_eq!(code, 1, "a creative fails");
    for want in [
        "lunch-portrait.png → harbourline.example/retail/portrait-totem-75: FAIL",
        "Legal text is 18 px; 23 px minimum at 3 m, ratio 150  [HC-L1-002]",
        "Logo overlaps the network-logo area",
    ] {
        assert!(out.contains(want), "output missing {want:?}:\n{out}");
    }
    assert_eq!(stderr, "");
}

#[test]
fn check_json() {
    let (_, out, _) = run_cli(&[
        "check",
        &ex("creative.json"),
        "-spec",
        &ex("retail.screens.json"),
        "-json",
        "-now",
        "2026-10-22T12:00:00Z",
    ]);
    let reps: Vec<serde_json::Value> =
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        reps.len() == 2
            && reps[0]["file_sha256"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
        "got {} reports",
        reps.len()
    );
}

#[test]
fn resolve_command() {
    let (code, out, stderr) = run_cli(&[
        "resolve",
        &ex("retail.screens.json"),
        "harbourline.example/retail/screen/HC-L1-002",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let got: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(got.len(), 1, "{out}");
    assert_eq!(
        got[0]["blocked_regions"].as_array().unwrap().len(),
        2,
        "{out}"
    );
    let derived = &got[0]["derived"];
    assert_eq!(derived["min_key_cap_height"]["px"], 77.0);
    assert_eq!(derived["max_words"]["value"], 6.0);
    // Go prints whole numbers without a fraction.
    assert!(
        out.contains("\"physical_mm\": [\n      940,\n      1670\n    ]"),
        "{out}"
    );
}

#[test]
fn resolve_errors() {
    let screens = ex("retail.screens.json");
    let (code, _, err) = run_cli(&[
        "resolve",
        &screens,
        "harbourline.example/retail/screen/nope",
    ]);
    assert_eq!(code, 1);
    assert_eq!(err, "screen \"nope\" not found in network \"retail\"\n");
    let (code, _, err) = run_cli(&["resolve", &screens, "x"]);
    assert_eq!(code, 2);
    assert_eq!(err, "\"x\" is not a format or screen reference\n");
}

#[test]
fn resolve_strictest_command() {
    let screens = ex("retail.screens.json");
    let (code, out, err) = run_cli(&[
        "resolve",
        "--strictest",
        &screens,
        "harbourline.example/retail/portrait-totem-75",
    ]);
    assert_eq!(code, 0, "{err}");
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(got["screens"], 2, "{out}");
    assert_eq!(got["min_cap_height_px"]["headline"], 77, "{out}");
    assert_eq!(
        got["sources"]["min_cap_height_px.headline"], "HC-L1-002",
        "{out}"
    );
    let (code, _, err) = run_cli(&[
        "resolve",
        "--strictest",
        &screens,
        "harbourline.example/retail/nope",
    ]);
    assert_eq!(code, 1);
    assert_eq!(err, "format \"nope\" not found in network \"retail\"\n");
    let (code, _, err) = run_cli(&["resolve", "--strictest", &screens, "x"]);
    assert_eq!(code, 2);
    assert_eq!(err, "\"x\" is not a format or screen reference\n");
    let (code, _, _) = run_cli(&["resolve", "--strictest", "missing.json", "a.example/n/f"]);
    assert_eq!(code, 2);
}

#[test]
fn usage_errors() {
    for args in [
        vec![],
        vec!["nope"],
        vec!["check", "creative.json"],
        vec!["resolve", "screens.json"],
        vec!["schema"],
        vec!["schema", "-nope"],
        vec!["check", "-spec", "x", "-now", "yesterday", "c.json"],
    ] {
        let (code, _, _) = run_cli(&args);
        assert_eq!(code, 2, "{args:?}");
    }
    let (code, out, err) = run_cli(&["help"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.starts_with("Check digital-signage creative against the screens it's made for\n"));
    let (_, _, err) = run_cli(&["nope"]);
    assert!(
        err.starts_with("error: unrecognized subcommand 'nope'\n"),
        "{err}"
    );
    let (_, _, err) = run_cli(&["check", "-spec", "x", "-now", "yesterday", "c.json"]);
    assert!(
        err.starts_with("opencrs check: --now: parsing time \"yesterday\" as "),
        "{err}"
    );
}

/// No command prints the top-level help to stderr and exits 2; asking for
/// help prints the same text to stdout and exits 0.
#[test]
fn help_and_version() {
    let (code, out, err) = run_cli(&[]);
    assert_eq!((code, out.as_str()), (2, ""));
    for args in [&["help"][..], &["-h"], &["--help"], &["-help"]] {
        let (c, o, e) = run_cli(args);
        assert_eq!((c, e.as_str()), (0, ""), "{args:?}");
        assert_eq!(o, err, "{args:?}");
    }
    assert!(err.contains("Usage: opencrs <COMMAND>\n"), "{err}");
    assert!(!err.contains('\x1b'), "plain without a terminal");

    for args in [
        &["check", "-h"][..],
        &["check", "-help"],
        &["help", "check"],
    ] {
        let (c, o, e) = run_cli(args);
        assert_eq!((c, e.as_str()), (0, ""), "{args:?}");
        assert!(o.contains("Usage: opencrs check "), "{o}");
        assert!(
            o.ends_with("Exit status: 0 passed, 1 a file failed, 2 usage or read error\n"),
            "{o}"
        );
    }
    // --help is the long form.
    let (_, short, _) = run_cli(&["check", "-h"]);
    let (_, long, _) = run_cli(&["check", "--help"]);
    assert!(
        long.len() > short.len() && long.contains("Examples:"),
        "{long}"
    );

    let (code, out, _) = run_cli(&["-V"]);
    assert_eq!(
        (code, out),
        (0, format!("opencrs {}\n", env!("CARGO_PKG_VERSION")))
    );
    let (code, out, _) = run_cli(&["version"]);
    assert_eq!(code, 0);
    assert!(out.starts_with(&format!(
        "opencrs {}\nOpenCRS spec 0.1\n",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(out.contains("au-nsw-transport-corridor@2017-11  "), "{out}");
}

/// The old single-dash spellings and the new `--long` ones do the same.
#[test]
fn old_and_new_spellings_agree() {
    let screens = ex("retail.screens.json");
    let creative = ex("creative.json");
    let old = run_cli(&[
        "check",
        "-spec",
        &screens,
        "-json",
        "-now",
        "2026-10-22T12:00:00Z",
        &creative,
    ]);
    let new = run_cli(&[
        "check",
        "--spec",
        &screens,
        "--json",
        "--now",
        "2026-10-22T12:00:00Z",
        &creative,
    ]);
    let short = run_cli(&[
        "check",
        &creative,
        "-s",
        &screens,
        "--json=true",
        "--now=2026-10-22T12:00:00Z",
    ]);
    assert_eq!(old, new);
    assert_eq!(old.0, 1);
    assert_eq!(old, short);
    // A value that looks like a flag is still the flag's value; `=false`
    // turns a boolean off, last one wins.
    let (code, out, _) = run_cli(&[
        "check",
        "-spec",
        &screens,
        "-json",
        "-json=false",
        "-now",
        "2026-10-22T12:00:00Z",
        &creative,
    ]);
    assert_eq!(code, 1);
    assert!(out.starts_with("lunch-portrait.png"), "{out}");
    let (code, _, err) = run_cli(&["check", "-spec", "-json", &creative]);
    assert_eq!(code, 2);
    assert!(err.starts_with("-json: open -json: "), "{err}");
    // Flags after `--` are positionals.
    let (code, _, _) = run_cli(&["schema", "--", "-kind"]);
    assert_eq!(code, 2);
    // Booleans with junk values and unknown flags are usage errors.
    for args in [
        vec!["schema", "-allow-local-http=maybe", "x"],
        vec!["schema", "-nope", "x"],
        vec!["schema", "---x"],
        vec!["schema", "-=x"],
        vec!["schema", "-kind"],
        vec!["check", "-margin", "1e999", "-spec", "x", "c.json"],
        vec!["conformance", "-level", "core", "a", "b"],
    ] {
        let (code, out, _) = run_cli(&args);
        assert_eq!((code, out.as_str()), (2, ""), "{args:?}");
    }
    let (code, out, _) = run_cli(&["schema", "-kind=screens", "--allow-local-http", &screens]);
    assert_eq!(code, 0, "{out}");
    // `-o` and `--o` both name the output directory.
    for flag in ["-o", "--o", "--output"] {
        let (code, _, err) = run_cli(&["fetch", flag, "x", "a b"]);
        assert_eq!(code, 2, "{flag}");
        assert!(err.contains("opencrs fetch:"), "{flag}: {err}");
    }
}

/// A publisher testing its index locally before deploying it.
#[test]
fn fetch_command() {
    let doc = std::fs::read(ex("retail.screens.json")).unwrap();
    let served = doc.clone();
    let srv = Server::start(move |r, addr| match r.path.as_str() {
        "/.well-known/opencrs.json" => Resp::ok(format!(
            r#"{{"opencrs":"0.1","publisher":{{"id":"harbourline.example","name":"Harbourline"}},
		  "documents":[{{"url":"http://{addr}/retail.screens.json","network":"retail"}},{{"url":"http://{addr}/locations.screens.json","visibility":"partner"}}]}}"#
        )),
        "/retail.screens.json" => Resp::ok(served.clone()).header("Cache-Control", "max-age=3600"),
        _ => Resp::status(404),
    });
    let host = srv.addr.clone();
    let dir = tempfile::tempdir().unwrap();
    let (code, out, stderr) = run_cli(&[
        "fetch",
        "-allow-local-http",
        "-o",
        dir.path().to_str().unwrap(),
        &host,
    ]);
    assert_eq!(code, 0, "{out}{stderr}");
    for want in [
        format!("trusted   public   http://{host}/retail.screens.json"),
        "networks retail, 3 screens; updated 2026-10-20T09:00:00+11:00; expires 2027-01-20T00:00:00+11:00; Cache-Control: max-age=3600".to_string(),
        format!("skipped   partner  http://{host}/locations.screens.json"),
        format!("needs a token issued by {host}"),
    ] {
        assert!(out.contains(&want), "missing {want:?} in:\n{out}");
    }
    let saved = std::fs::read(dir.path().join("harbourline.example/retail.screens.json")).unwrap();
    assert_eq!(
        saved, doc,
        "the saved document must be byte-for-byte what was served"
    );

    let (code, _, stderr) = run_cli(&["fetch", &host]);
    assert!(
        code == 2 && stderr.contains("isn't a publisher domain"),
        "without -allow-local-http: exit {code}, {stderr}"
    );
}

#[test]
fn fetch_usage_errors() {
    for args in [
        vec!["fetch"],
        vec!["fetch", "a", "b"],
        vec!["fetch", "-token", "nope", "a.example"],
        vec!["fetch", "-token", "=x", "a.example"],
    ] {
        assert_eq!(run_cli(&args).0, 2, "{args:?}");
    }
    let (_, _, err) = run_cli(&["fetch", "-token", "nope", "a.example"]);
    assert_eq!(
        err,
        "opencrs fetch: --token wants DOMAIN=TOKEN, got \"nope\"\n"
    );
}

#[test]
fn conformance_passes_full_level() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("conformance/0.1");
    let (code, out, err) = run_cli(&["conformance", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        out,
        "the reference validator conforms at the Full level: 48 of 48 cases pass\n\
         the reference validator supports strictest requirements (optional): 9 of 9 cases pass\n"
    );
    let (code, out, _) = run_cli(&["conformance", "--no-strictest", dir.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "the reference validator conforms at the Full level: 48 of 48 cases pass\n\
         Strictest requirements (optional) were not tested\n"
    );
    let (code, out, _) = run_cli(&["conformance", "-level", "core", "-v", dir.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains("pass  core-pass"), "{out}");
    let (code, _, err) = run_cli(&["conformance", "-level", "nope"]);
    assert_eq!(code, 2);
    assert_eq!(
        err,
        "opencrs conformance: --level must be one of core, layout, full\n"
    );
}

#[test]
fn check_confines_manifest_files_to_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ex("creative.json")).unwrap()).unwrap();
    m["files"][0]["file"] = "../outside.png".into();
    let path = dir.path().join("creative.json");
    std::fs::write(&path, m.to_string()).unwrap();
    let (code, _, err) = run_cli(&[
        "check",
        "-spec",
        &ex("retail.screens.json"),
        "-now",
        "2026-10-22T12:00:00Z",
        &path.to_string_lossy(),
    ]);
    assert_eq!(code, 2);
    assert_eq!(
        err,
        "../outside.png: file name must be relative and stay inside the directory\n"
    );
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
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

#[test]
fn conformance_reports_failing_cases() {
    let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/0.1");
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&suite, tmp.path());
    let p = tmp.path().join("cases/core-pass/case.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["expect"][0]["result"] = "fail".into();
    std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();

    let (code, out, _) = run_cli(&["conformance", tmp.path().to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("FAIL  core-pass"), "{out}");
    assert!(out.contains("\n      spec: "), "{out}");
    assert!(out.contains("\n      "), "{out}");
    assert!(
        out.contains(
            "the reference validator does not conform at the Full level: 47 of 48 cases pass\n"
        ),
        "{out}"
    );

    // An external validator that prints nothing is an error on every case.
    // (`-cmd` runs through `sh`, so this needs a Unix shell.)
    if !cfg!(unix) {
        return;
    }
    let (code, out, _) = run_cli(&[
        "conformance",
        "-level",
        "core",
        "-cmd",
        "exit 3; :",
        tmp.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 1);
    assert!(out.contains("      error: exit status 3: \n"), "{out}");
    assert!(
        out.contains("exit 3; : does not conform at the Core level"),
        "{out}"
    );
}

#[test]
fn fetch_saves_the_merged_document() {
    let public = std::fs::read_to_string(ex("retail.screens.json")).unwrap();
    let mut other: serde_json::Value = serde_json::from_str(&public).unwrap();
    other["networks"][0]["id"] = "private".into();
    other["networks"][0]["name"] = "Private".into();
    let other = other.to_string();
    let srv = Server::start(move |r, addr| match r.path.as_str() {
        "/.well-known/opencrs.json" => Resp::ok(format!(
            r#"{{"opencrs":"0.1","publisher":{{"id":"harbourline.example","name":"Harbourline"}},
 "documents":[{{"url":"http://{addr}/retail.screens.json"}},{{"url":"http://{addr}/private.screens.json","visibility":"partner"}}]}}"#
        )),
        "/retail.screens.json" => Resp::ok(public.clone()),
        "/private.screens.json" if r.authorization == "Bearer s3cret" => Resp::ok(other.clone()),
        _ => Resp::status(403),
    });
    let host = srv.addr.clone();
    let dir = tempfile::tempdir().unwrap();
    let token = format!("{host}=s3cret");
    let (code, out, err) = run_cli(&[
        "fetch",
        "-allow-local-http",
        "-token",
        &token,
        "-o",
        dir.path().to_str().unwrap(),
        &host,
    ]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("merged.screens.json (public and partner documents merged)"),
        "{out}"
    );
    let merged: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("harbourline.example/merged.screens.json")).unwrap(),
    )
    .unwrap();
    let ids: Vec<&str> = merged["networks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["retail", "private"], "{merged}");
    assert_eq!(merged["publisher"], "harbourline.example");
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_is_a_usage_error() {
    use std::os::unix::ffi::OsStrExt;
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_opencrs"))
        .arg("schema")
        .arg(std::ffi::OsStr::from_bytes(b"\xff.json"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "opencrs: argument 2 is not valid UTF-8\n"
    );
}

#[cfg(unix)]
#[test]
fn unreadable_creative_file_keeps_gos_wording() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    for f in [
        "creative.json",
        "lunch-portrait.png",
        "lunch-portrait-logo-right.png",
    ] {
        std::fs::copy(ex(f), dir.path().join(f)).unwrap();
    }
    let png = dir.path().join("lunch-portrait.png");
    std::fs::set_permissions(&png, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&png).is_ok() {
        return; // running as root: permissions don't apply
    }
    let creative = dir.path().join("creative.json");
    let (code, _, err) = run_cli(&[
        "check",
        "-spec",
        &ex("retail.screens.json"),
        creative.to_str().unwrap(),
    ]);
    assert_eq!(code, 2);
    assert_eq!(
        err,
        format!(
            "lunch-portrait.png: open {}/lunch-portrait.png: permission denied\n",
            dir.path().display()
        )
    );
}

#[test]
fn probe_command() {
    let a = ex("lunch-portrait.png");
    let b = ex("lunch-portrait-logo-right.png");
    let (code, out, err) = run_cli(&["probe", &a, &b]);
    assert_eq!(code, 0, "{err}");
    let want = opencrs::fs::probe_path(std::path::Path::new(&a)).unwrap();
    for line in [
        a.clone(),
        "  type      image/png".to_string(),
        "  pixels    1080 x 1920".to_string(),
        format!("  bytes     {}", want.bytes),
        format!("  sha256    {}", want.sha256),
        b.clone(),
    ] {
        assert!(out.contains(&line), "missing {line:?} in:\n{out}");
    }
    assert!(err.is_empty());

    let (code, out, _) = run_cli(&["probe", "--json", &a, &b]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(v[0]["file"], a.as_str());
    assert_eq!(v[0]["sha256"], want.sha256.as_str());
    assert_eq!(v[0]["bytes"], want.bytes);
    assert_eq!(v[0]["media_type"], "image/png");
    assert_eq!(v[0]["pixels"], serde_json::json!([1080, 1920]));
}

#[test]
fn probe_reports_unreadable_files_and_unknown_types() {
    let dir = tempfile::tempdir().unwrap();
    let blob = dir.path().join("blob.bin");
    std::fs::write(&blob, b"not a known type").unwrap();
    let blob = blob.to_str().unwrap();
    let missing = dir.path().join("nope.png");
    let missing = missing.to_str().unwrap();

    let (code, out, err) = run_cli(&["probe", missing, blob]);
    assert_eq!(code, 2);
    assert!(err.starts_with(&format!("{missing}: ")), "{err}");
    assert!(
        out.contains("  type      unknown") && out.contains("  bytes     16"),
        "{out}"
    );
    assert!(!out.contains("pixels"), "{out}");
    // The file that could be read is still in the JSON.
    let (code, out, _) = run_cli(&["probe", "--json", missing, blob]);
    assert_eq!(code, 2);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["file"], blob);
    let (code, _, err) = run_cli(&["probe"]);
    assert_eq!(code, 2, "{err}");
}

#[test]
fn legibility_command() {
    let (code, out, err) = run_cli(&[
        "legibility",
        "--near",
        "2",
        "--far",
        "6",
        "--screen-width-mm",
        "940",
        "--screen-width-px",
        "1080",
        "--dwell",
        "4",
    ]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "Text seen from 2 m to 6 m, legible at 150 times its height\n\
         \x20 key text    40 mm, 46 px  (at 6 m, ratio 150)\n\
         \x20 legal text  13.3 mm, 16 px  (at 2 m, ratio 150)\n\
         \x20 word limit  8 words  (4 s dwell at 2 words/s)\n"
    );
    let (code, out, _) = run_cli(&[
        "legibility",
        "--near",
        "2",
        "--far",
        "6",
        "--screen-width-mm",
        "940",
        "--screen-width-px",
        "1080",
        "--dwell",
        "4",
        "--json",
    ]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["ratio"], 150);
    assert_eq!(v["key"]["px"], 46);
    assert_eq!(v["key"]["px_exact"], 45.96);
    assert_eq!(v["legal"]["px"], 16);
    assert_eq!(v["words"]["value"], 8);
    // Without a screen or a dwell: millimetres only.
    let (_, out, _) = run_cli(&[
        "legibility",
        "--near",
        "3",
        "--far",
        "10",
        "--ratio",
        "250",
        "--json",
    ]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["key"]["mm"], 40);
    assert!(v["key"].get("px").is_none() && v.get("words").is_none());
    // The Go-style single dash still works.
    let (code, _, _) = run_cli(&["legibility", "-near", "2", "-far", "6"]);
    assert_eq!(code, 0);
}

#[test]
fn legibility_and_dwell_inputs_that_make_no_sense_are_usage_errors() {
    let (code, _, err) = run_cli(&["legibility", "--near", "6", "--far", "2"]);
    assert_eq!(code, 2);
    assert_eq!(
        err,
        "opencrs legibility: the near distance (6 m) is past the far one (2 m)\n"
    );
    for args in [
        vec!["legibility", "--near", "2"],
        vec![
            "legibility",
            "--near",
            "2",
            "--far",
            "6",
            "--screen-width-mm",
            "940",
        ],
        vec!["legibility", "--near", "2", "--far", "6", "--ratio", "x"],
        vec!["dwell", "--near", "20", "--far", "150"],
    ] {
        let (code, _, err) = run_cli(&args);
        assert_eq!(code, 2, "{args:?}: {err}");
    }
    let (code, _, err) = run_cli(&["legibility", "--near", "2", "--far", "6", "--ratio", "0"]);
    assert!(
        code == 2 && err.contains("ratio must be a number above zero"),
        "{err}"
    );
    let (code, _, err) = run_cli(&["dwell", "--speed", "-5", "--near", "20", "--far", "150"]);
    assert!(code == 2 && err.contains("the speed must be"), "{err}");
    let (code, _, err) = run_cli(&[
        "dwell", "--speed", "80", "--near", "20", "--far", "150", "--pack", "x@1",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err, "opencrs dwell: unknown rule pack x@1\n");
}

#[test]
fn dwell_command() {
    let pack = "au-nsw-transport-corridor@2017-11";
    let (code, out, err) = run_cli(&[
        "dwell", "--speed", "80", "--near", "20", "--far", "150", "--pack", pack,
    ]);
    assert_eq!(code, 0, "{err}");
    for want in [
        "At 80 km/h (22.22 m/s), seen from 20 m to 150 m: 5.85 s to cross",
        "  glance      2 s, 4 words, reading from 64.44 m",
        "  key text    429.6 mm  (to read from 64 m, the start of a 2 s glance at 80 km/h (ratio 150))",
        "au-nsw-transport-corridor@2017-11 (draft): NSW digital signs in transport corridors",
        "  min dwell       25 s (at 80 km/h and over)",
        "  max transition  0.1 s",
        "  motion          not allowed",
        "  failure state   black",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    let (code, out, _) = run_cli(&[
        "dwell", "--speed", "60", "--near", "20", "--far", "150", "--pack", pack, "--json",
    ]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["glance"]["words"], 4);
    assert_eq!(v["packs"][0]["ref"], pack);
    assert_eq!(v["packs"][0]["min_dwell_s"], 10);
    assert_eq!(v["packs"][0]["dwell_basis"], "below 80 km/h");
}

#[test]
fn rule_packs_command() {
    let (code, out, _) = run_cli(&["rule-packs"]);
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 6, "{out}");
    let line = out
        .lines()
        .find(|l| l.starts_with("au-nsw-transport-corridor@2017-11"))
        .unwrap();
    assert!(line.contains("  draft  "), "{line}");
    assert!(
        line.ends_with("NSW digital signs in transport corridors"),
        "{line}"
    );
    assert!(out.contains("au-tas-spp-signs-code@2024-06"), "{out}");

    let (code, out, _) = run_cli(&["rule-packs", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let packs = v["rule_packs"].as_array().unwrap();
    assert_eq!(packs.len(), 6);
    assert!(
        packs
            .iter()
            .all(|p| p["ref"].is_string() && p["status"].is_string())
    );

    let pack = "au-nsw-transport-corridor@2017-11";
    let (code, out, _) = run_cli(&["rule-packs", pack]);
    assert_eq!(code, 0);
    for want in [
        "min dwell       10 s below 80 km/h; 25 s at 80 km/h and over",
        "max transition  0.1 s",
        "failure state   black",
        "day       zone_1 no limit, zone_2 6000, zone_3 6000, zone_4 6000",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    // The pack as the document it is.
    let (code, out, _) = run_cli(&["rule-packs", "--json", pack]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../rulepacks")
        .join(format!("{pack}.json"));
    let want: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    for key in [
        "rule_pack",
        "version",
        "title",
        "source_url",
        "playback",
        "creative",
    ] {
        assert_eq!(v[key], want[key], "{key}");
    }

    let (code, _, err) = run_cli(&["rule-packs", "x@1"]);
    assert_eq!(code, 1);
    assert!(
        err.starts_with("opencrs rule-packs: no built-in rule pack x@1; known: au-nsw"),
        "{err}"
    );
}

/// The publisher check's fetching is tested in opencrs-fetch, which has the
/// loop and the refusals; here, what the command does with its answers.
#[test]
fn publisher_command_without_a_publisher() {
    let (code, out, err) = run_cli(&["publisher", "not a domain", "--now", "2026-10-25T12:00:00Z"]);
    assert_eq!(code, 2);
    assert_eq!(out, "");
    assert!(err.contains("isn't a publisher domain"), "{err}");
    let (code, out, _) = run_cli(&["publisher", "--json", "localhost"]);
    assert_eq!(code, 2);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["errors"], 1);
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("isn't a publisher domain")
    );
    assert_eq!(v["wanted"], serde_json::json!([]));
    let (code, _, err) = run_cli(&["publisher", "x.example", "--now", "later"]);
    assert!(code == 2 && err.contains("not an RFC 3339 time"), "{err}");
    let (code, _, _) = run_cli(&["publisher"]);
    assert_eq!(code, 2);
}
