//! The JSON examples in the specification and in `examples/` must pass the
//! published schemas. Ported from `TestSpecExamples` and
//! `TestExamplesMatchSchemas` in the retired Go validator.

use std::path::PathBuf;

use opencrs::{Kind, SchemaOptions, detect_kind, validate_schema};

fn read(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// The spec's "A complete example" is copied verbatim into the examples
// directory; it must pass the schema exactly as published.
#[test]
fn examples_match_schemas() {
    let cases = [
        ("retail.screens.json", Kind::Screens),
        (".well-known/opencrs.json", Kind::Index),
        ("creative.json", Kind::Creative),
        ("reports.json", Kind::Report), // what "opencrs check -json" printed
    ];
    for (name, want) in cases {
        let raw = read(&format!("examples/harbourline/{name}"));
        assert_eq!(detect_kind(&raw).unwrap(), want, "{name}");
        let opt = SchemaOptions {
            list: true,
            ..Default::default()
        };
        let errs = validate_schema(want, &raw, &opt).unwrap();
        assert!(errs.is_empty(), "{name}: {errs:?}");
    }
}

// Every JSON example in the spec must validate, and the complete example
// must be identical to the copy in examples/.
#[test]
fn spec_examples() {
    let spec = String::from_utf8(read("OpenCRS 0.1 Specification.md")).unwrap();
    let blocks: Vec<&str> = spec.split("```json\n").skip(1).collect();
    assert!(
        blocks.len() >= 5,
        "found {} JSON blocks in the spec",
        blocks.len()
    );
    for (i, b) in blocks.iter().enumerate() {
        let raw = &b[..b.find("```").unwrap()];
        let kind = detect_kind(raw.as_bytes()).unwrap_or_else(|e| panic!("block {i}: {e}"));
        let errs = validate_schema(kind, raw.as_bytes(), &SchemaOptions::default()).unwrap();
        assert!(
            errs.is_empty(),
            "spec {} example (block {i}): {errs:?}",
            kind.as_str()
        );
    }
    let complete = &spec[spec.find("## A complete example").unwrap()..];
    let complete = complete
        .split_once("```json\n")
        .unwrap()
        .1
        .split("```")
        .next()
        .unwrap();
    assert_eq!(
        complete.as_bytes(),
        read("examples/harbourline/retail.screens.json"),
        "the spec's complete example and examples/harbourline/retail.screens.json differ"
    );
}

// Examples name fictional organisations on .example domains, never real
// companies (see CONTRIBUTING.md). This covers the spec's JSON examples,
// examples/ and the conformance cases.
#[test]
fn examples_use_example_domains() {
    let mut sources: Vec<(String, Vec<u8>)> = Vec::new();
    let spec = String::from_utf8(read("OpenCRS 0.1 Specification.md")).unwrap();
    for (i, b) in spec.split("```json\n").skip(1).enumerate() {
        sources.push((
            format!("spec block {i}"),
            b[..b.find("```").unwrap()].into(),
        ));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in ["examples", "conformance"] {
        json_files(&root.join(dir), &mut sources);
    }
    assert!(
        sources.len() >= 10,
        "found only {} example sources",
        sources.len()
    );
    let mut bad = Vec::new();
    for (name, raw) in &sources {
        let v: serde_json::Value =
            serde_json::from_slice(raw).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut hosts = Vec::new();
        example_hosts(&v, "", &mut hosts);
        for h in hosts.into_iter().filter(|h| !fictional_host(h)) {
            bad.push(format!("{name}: {h:?} is not a .example domain"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    for h in ["example.com", "specs.acme.com", "harbourline.example.com"] {
        assert!(!fictional_host(h), "{h}");
    }
}

fn json_files(dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            json_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push((path.display().to_string(), std::fs::read(&path).unwrap()));
        }
    }
}

// The domains an example names: URL hosts, email domains, ext keys, and the
// values of fields that hold a domain. Rule packs' source URLs are skipped:
// they cite real regulators by design.
fn example_hosts(v: &serde_json::Value, key: &str, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                if k == "source_url" {
                    continue;
                }
                if key == "ext" {
                    out.push(k.clone());
                }
                let k = if key == "publisher" && k == "id" {
                    "domain"
                } else {
                    k
                };
                example_hosts(x, k, out);
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| example_hosts(x, key, out)),
        serde_json::Value::String(s) => {
            if s.starts_with("http://") || s.starts_with("https://") {
                if let Some(h) = url::Url::parse(s)
                    .ok()
                    .and_then(|u| u.host_str().map(String::from))
                {
                    out.push(h.trim_matches(['[', ']']).to_string());
                }
            } else if let Some((_, domain)) = s
                .rsplit_once('@')
                .filter(|(_, d)| d.contains('.') && !s.contains(' '))
            {
                // An email address (a rule pack pin such as "pack@2024-06" has no dot).
                out.push(domain.to_string());
            } else if matches!(key, "publisher" | "domain" | "target") {
                out.push(s.split('/').next().unwrap_or_default().to_string());
            }
        }
        _ => {}
    }
}

fn fictional_host(h: &str) -> bool {
    let h = h.to_ascii_lowercase();
    matches!(
        h.as_str(),
        "localhost" | "127.0.0.1" | "::1" | "opencrs.dev" | "json-schema.org"
    ) || h == "example"
        || h.ends_with(".example")
}
