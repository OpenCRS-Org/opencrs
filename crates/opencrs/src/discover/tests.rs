use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::json;

use super::*;

fn now() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z").unwrap()
}

/// Serves canned bodies by exact URL and records what was asked for.
#[derive(Default)]
struct Fake {
    bodies: HashMap<String, String>,
    status: HashMap<String, u16>,
    /// The token each URL was fetched with.
    tokens: RefCell<HashMap<String, String>>,
}

impl Fetcher for Fake {
    fn fetch(&self, r: &Request) -> Result<Response, FetchError> {
        self.tokens
            .borrow_mut()
            .insert(r.url.clone(), r.token.clone().unwrap_or_default());
        if let Some(&code) = self.status.get(&r.url) {
            return Err(FetchError::Status {
                url: r.url.clone(),
                code,
            });
        }
        match self.bodies.get(&r.url) {
            Some(b) => Ok(Response {
                body: b.clone().into_bytes(),
                etag: "\"v1\"".into(),
                cache_control: "max-age=3600".into(),
                last_modified: String::new(),
            }),
            None => Err(FetchError::Status {
                url: r.url.clone(),
                code: 404,
            }),
        }
    }
}

impl Fake {
    fn put(&mut self, url: impl Into<String>, body: String) {
        self.bodies.insert(url.into(), body);
    }
}

fn doc(url: &str, visibility: &str) -> IndexDocument {
    IndexDocument {
        url: url.into(),
        network: String::new(),
        visibility: visibility.into(),
    }
}

fn delegate(domain: &str, scope: &str) -> Delegate {
    Delegate {
        domain: domain.into(),
        scope: scope.into(),
    }
}

fn index(publisher: &str, docs: &[IndexDocument], delegates: &[Delegate]) -> String {
    let mut idx =
        json!({"opencrs":"0.1","publisher":{"id":publisher,"name":publisher},"documents":docs});
    if !delegates.is_empty() {
        idx["delegates"] = serde_json::to_value(delegates).unwrap();
    }
    idx.to_string()
}

/// A minimal valid screens document.
fn screens(publisher: &str, network: &str, ids: &[&str]) -> String {
    let ss: Vec<String> = ids
        .iter()
        .map(|id| format!(r#"{{"id":"{id}","format":"f"}}"#))
        .collect();
    format!(
        r#"{{"opencrs":"0.1","publisher":"{publisher}","updated_at":"2026-10-20T09:00:00+11:00","networks":[{{"id":"{network}","name":"N",
	  "formats":[{{"id":"f","pixels":[1080,1920],"media":{{"static":["image/png"]}}}}],"screens":[{}]}}]}}"#,
        ss.join(",")
    )
}

fn opts() -> Options {
    Options {
        tokens: HashMap::new(),
        allow_local_http: false,
        now: now(),
        max_index_entries: 0,
        max_run_bytes: 0,
        max_fetches: 0,
        keep_raw: true,
    }
}

fn by_url<'a>(r: &'a DiscoveryResult, url: &str) -> &'a DiscoveredDocument {
    r.documents
        .iter()
        .find(|d| d.url == url)
        .unwrap_or_else(|| panic!("{url} not in results"))
}

fn idx_url(d: &str) -> String {
    index_url(d, false)
}

// The spec's index example: Harbourline hosts one document itself and one
// on a hosting company. Both are trusted because harbourline.example's index lists
// their exact URLs.
#[test]
fn own_documents_trusted_wherever_hosted() {
    let mut f = Fake::default();
    let own = "https://harbourline.example/opencrs/retail.screens.json";
    let hosted = "https://specs.cms-a.example/harbourline/airports.screens.json";
    let mut a = doc(own, "");
    a.network = "retail".into();
    let mut b = doc(hosted, "");
    b.network = "airports".into();
    f.put(
        idx_url("harbourline.example"),
        index("harbourline.example", &[a, b], &[]),
    );
    f.put(own, screens("harbourline.example", "retail", &["S1", "S2"]));
    f.put(hosted, screens("harbourline.example", "airports", &["A1"]));

    let res = discover("HARBOURLINE.example", &opts(), &f).unwrap();
    for u in [own, hosted] {
        let d = by_url(&res, u);
        assert_eq!(d.status, Status::Trusted, "{u}: {}", d.reason);
        assert_eq!(d.etag, "\"v1\"");
        assert_eq!(d.cache_control, "max-age=3600");
    }
    let d = by_url(&res, own);
    assert_eq!((d.screens, d.networks[0].as_str()), (2, "retail"));
    assert_eq!(res.trusted().len(), 2);
}

#[test]
fn verify_needs_the_exact_url() {
    let mut f = Fake::default();
    let listed = "https://specs.cms-a.example/harbourline/airports.screens.json";
    f.put(
        idx_url("harbourline.example"),
        index("harbourline.example", &[doc(listed, "")], &[]),
    );
    let body = screens("harbourline.example", "airports", &["A1"]);
    // Hosts are case-insensitive, so a real server answers either spelling.
    for u in [
        listed.to_string(),
        "https://SPECS.cms-a.example/harbourline/airports.screens.json".into(),
        format!("{listed}?v=2"),
        format!("{listed}/"),
        "https://evil.example/airports.screens.json".into(),
    ] {
        f.put(u, body.clone());
    }
    let (_, d) = verify(
        "https://SPECS.cms-a.example/harbourline/airports.screens.json",
        &opts(),
        &f,
    )
    .unwrap();
    assert_eq!(d.status, Status::Trusted, "host case differs: {}", d.reason);
    for u in [
        format!("{listed}?v=2"),
        format!("{listed}/"),
        "https://evil.example/airports.screens.json".to_string(),
    ] {
        let (_, d) = verify(&u, &opts(), &f).unwrap();
        assert_eq!(d.status, Status::Rejected, "{u}");
        assert!(
            d.reason
                .contains("not listed by harbourline.example's index")
        );
    }
}

#[test]
fn verify_needs_a_publisher() {
    let mut f = Fake::default();
    f.put("https://x.example/a.json", "{}".into());
    let e = verify("https://x.example/a.json", &opts(), &f).unwrap_err();
    assert_eq!(
        e.to_string(),
        "https://x.example/a.json doesn't declare a publisher"
    );
}

#[test]
fn verify_needs_an_object_header() {
    let mut f = Fake::default();
    f.put(idx_url("evil.example"), index("evil.example", &[], &[]));
    f.put("https://x.example/a.json", r#"["evil.example"]"#.into());
    let e = verify("https://x.example/a.json", &opts(), &f).unwrap_err();
    assert_eq!(
        e.to_string(),
        "https://x.example/a.json doesn't declare a publisher"
    );
}

#[test]
fn index_cant_vouch_for_another_publisher() {
    let mut f = Fake::default();
    let u = "https://landlord.example/opencrs/x.screens.json";
    f.put(
        idx_url("landlord.example"),
        index("landlord.example", &[doc(u, "")], &[]),
    );
    f.put(u, screens("harbourline.example", "retail", &["S1"]));
    let res = discover("landlord.example", &opts(), &f).unwrap();
    let d = by_url(&res, u);
    assert_eq!(d.status, Status::Rejected);
    assert!(
        d.reason.contains("declares publisher harbourline.example"),
        "{}",
        d.reason
    );
}

#[test]
fn index_must_name_its_own_domain() {
    let mut f = Fake::default();
    f.put(
        idx_url("evil.example"),
        index("harbourline.example", &[], &[]),
    );
    let e = discover("evil.example", &opts(), &f).unwrap_err();
    assert!(
        e.to_string().contains("can only speak for its own domain"),
        "{e}"
    );
}

#[test]
fn missing_index_error_is_wrapped() {
    let f = Fake::default();
    let e = discover("nowhere.example", &opts(), &f).unwrap_err();
    assert_eq!(
        e.to_string(),
        "fetching the index: https://nowhere.example/.well-known/opencrs.json: HTTP 404"
    );
}

// harbourline.example delegates its retail network to baycrest.example.
// Baycrest's index lists documents written for Harbourline's screens.
#[test]
fn delegation() {
    let mut f = Fake::default();
    let in_scope = "https://baycrest.example/opencrs/hl-retail.screens.json";
    let out_of_scope = "https://baycrest.example/opencrs/hl-airports.screens.json";
    let baycrest_own = "https://baycrest.example/opencrs/own.screens.json";
    let second_hop = "https://third.example/hl-retail.screens.json";
    f.put(
        idx_url("harbourline.example"),
        index(
            "harbourline.example",
            &[],
            &[
                delegate("baycrest.example", "network:retail"),
                delegate("weird.example", "venue:abc"),
            ],
        ),
    );
    f.put(
        idx_url("baycrest.example"),
        index(
            "baycrest.example",
            &[
                doc(in_scope, ""),
                doc(out_of_scope, ""),
                doc(baycrest_own, ""),
            ],
            &[delegate("third.example", "")],
        ),
    );
    f.put(
        in_scope,
        screens("harbourline.example", "retail", &["HC-L2-014"]),
    );
    f.put(
        out_of_scope,
        screens("harbourline.example", "airports", &["SYD-1"]),
    );
    f.put(
        baycrest_own,
        screens("baycrest.example", "centres", &["V1"]),
    );
    f.put(
        idx_url("third.example"),
        index("third.example", &[doc(second_hop, "")], &[]),
    );
    f.put(second_hop, screens("harbourline.example", "retail", &["X"]));

    let res = discover("harbourline.example", &opts(), &f).unwrap();
    let d = by_url(&res, in_scope);
    assert_eq!(d.status, Status::Trusted, "{}", d.reason);
    assert_eq!(
        (d.delegated_by.as_str(), d.listed_by.as_str()),
        ("harbourline.example", "baycrest.example")
    );
    assert_eq!(
        d.reason,
        "harbourline.example names baycrest.example as a delegate, and baycrest.example's index lists this URL"
    );
    let d = by_url(&res, out_of_scope);
    assert_eq!(d.status, Status::Rejected);
    assert!(
        d.reason.contains("outside the scope network:retail"),
        "{}",
        d.reason
    );
    for d in &res.documents {
        assert!(
            d.url != baycrest_own && d.url != second_hop,
            "{} shouldn't be in harbourline's results",
            d.url
        );
    }
    assert!(
        !f.tokens.borrow().contains_key(&idx_url("third.example")),
        "delegates are followed one hop only"
    );
    assert!(
        res.delegates[1].error.contains("grants nothing"),
        "{:?}",
        res.delegates[1]
    );
    assert_eq!(res.delegates[0].index_url, idx_url("baycrest.example"));
}

#[test]
fn partner_documents_and_tokens() {
    let mut f = Fake::default();
    let public = "https://harbourline.example/opencrs/retail.screens.json";
    let partner = "https://harbourline.example/opencrs/retail-locations.screens.json";
    let delegated = "https://baycrest.example/opencrs/hl.screens.json";
    f.put(
        idx_url("harbourline.example"),
        index(
            "harbourline.example",
            &[doc(public, ""), doc(partner, "partner")],
            &[delegate("baycrest.example", "")],
        ),
    );
    f.put(
        idx_url("baycrest.example"),
        index("baycrest.example", &[doc(delegated, "")], &[]),
    );
    f.put(public, screens("harbourline.example", "retail", &["S1"]));
    f.put(partner, screens("harbourline.example", "retail", &["S1"]));
    f.put(delegated, screens("harbourline.example", "west", &["W1"]));

    let res = discover("harbourline.example", &opts(), &f).unwrap();
    let d = by_url(&res, partner);
    assert_eq!(d.status, Status::Skipped);
    assert!(
        d.reason
            .contains("needs a token issued by harbourline.example"),
        "{}",
        d.reason
    );

    let mut o = opts();
    o.tokens
        .insert("harbourline.example".into(), "s3cret".into());
    let res = discover("harbourline.example", &o, &f).unwrap();
    assert_eq!(by_url(&res, partner).status, Status::Trusted);
    for (u, tok) in f.tokens.borrow().iter() {
        assert!(tok.is_empty() || u == partner, "token sent to {u}");
    }

    f.status.insert(partner.into(), 403);
    let res = discover("harbourline.example", &o, &f).unwrap();
    let d = by_url(&res, partner);
    assert_eq!(d.status, Status::Skipped);
    assert!(d.reason.contains("refused the credentials"), "{}", d.reason);
}

#[test]
fn expired_and_invalid_documents() {
    let mut f = Fake::default();
    let expired = "https://p.example/old.screens.json";
    let invalid = "https://p.example/bad.screens.json";
    let missing = "https://p.example/gone.screens.json";
    f.put(
        idx_url("p.example"),
        index(
            "p.example",
            &[doc(expired, ""), doc(invalid, ""), doc(missing, "")],
            &[],
        ),
    );
    f.put(
        expired,
        screens("p.example", "n", &["S"]).replacen(
            "\"updated_at\"",
            "\"expires_at\":\"2026-01-01T00:00:00Z\",\"updated_at\"",
            1,
        ),
    );
    f.put(
        invalid,
        r#"{"opencrs":"0.1","publisher":"p.example","networks":[]}"#.into(),
    );
    let res = discover("p.example", &opts(), &f).unwrap();
    for (u, s) in [
        (expired, Status::Expired),
        (invalid, Status::Rejected),
        (missing, Status::Failed),
    ] {
        let d = by_url(&res, u);
        assert_eq!(d.status, s, "{u}: {}", d.reason);
    }
    assert!(
        by_url(&res, invalid)
            .reason
            .starts_with("not a valid screens document (")
    );
    assert!(res.trusted().is_empty(), "none of these may be used");
}

#[test]
fn expiry_compares_instants_across_offsets() {
    let mut f = Fake::default();
    let u = "https://p.example/a.screens.json";
    f.put(idx_url("p.example"), index("p.example", &[doc(u, "")], &[]));
    // 2026-10-22T12:30+11:00 is 01:30Z, before now (12:00Z): expired,
    // although the wall-clock time is after 12:00.
    f.put(
        u,
        screens("p.example", "n", &["S"]).replacen(
            "\"updated_at\"",
            "\"expires_at\":\"2026-10-22T12:30:00+11:00\",\"updated_at\"",
            1,
        ),
    );
    let res = discover("p.example", &opts(), &f).unwrap();
    assert_eq!(by_url(&res, u).status, Status::Expired);
}

#[test]
fn not_json_is_rejected() {
    let mut f = Fake::default();
    let u = "https://p.example/a.screens.json";
    f.put(idx_url("p.example"), index("p.example", &[doc(u, "")], &[]));
    f.put(u, "<html>".into());
    let res = discover("p.example", &opts(), &f).unwrap();
    let d = by_url(&res, u);
    assert_eq!(d.status, Status::Rejected);
    assert!(d.reason.starts_with("not JSON: "), "{}", d.reason);
}

// An index on localhost (for local testing) can stand in for any publisher.
#[test]
fn local_testing() {
    let raw = include_str!("../../../../examples/harbourline/retail.screens.json");
    let mut f = Fake::default();
    let u = "http://localhost:8080/retail.screens.json";
    f.put(
        "http://localhost:8080/.well-known/opencrs.json",
        index("harbourline.example", &[doc(u, "")], &[]),
    );
    f.put(u, raw.to_string());
    let mut o = opts();
    assert!(
        discover("localhost:8080", &o, &f).is_err(),
        "http://localhost needs allow_local_http"
    );
    o.allow_local_http = true;
    let res = discover("localhost:8080", &o, &f).unwrap();
    let d = by_url(&res, u);
    assert_eq!((d.status, d.screens), (Status::Trusted, 3), "{}", d.reason);
}

#[test]
fn parse_scope_cases() {
    assert_eq!(parse_scope(""), Some(vec![]));
    assert_eq!(
        parse_scope("network:retail"),
        Some(vec!["retail".to_string()])
    );
    assert_eq!(parse_scope("network:"), None);
    assert_eq!(parse_scope("venue:x"), None);
}

#[test]
fn result_serialises_like_go() {
    let mut f = Fake::default();
    let u = "https://p.example/a.screens.json";
    f.put(idx_url("p.example"), index("p.example", &[doc(u, "")], &[]));
    f.put(u, screens("p.example", "n", &["S"]));
    let res = discover("p.example", &opts(), &f).unwrap();
    let v = serde_json::to_value(&res).unwrap();
    let d = &v["documents"][0];
    assert_eq!(d["status"], "trusted");
    assert_eq!(d["screens"], 1);
    assert!(d.get("raw").is_none() && d.get("delegated_by").is_none());
    assert!(v.get("delegates").is_none());
    assert_eq!(v["index"]["publisher"]["id"], "p.example");
}

/// Serves canned bodies by the host and path the URL parser sees, as a real
/// client would connect.
#[derive(Default)]
struct ByHost {
    bodies: HashMap<String, String>,
    asked: RefCell<Vec<String>>,
}

impl Fetcher for ByHost {
    fn fetch(&self, r: &Request) -> Result<Response, FetchError> {
        let u = url::Url::parse(&r.url).map_err(|e| FetchError::Other(e.to_string()))?;
        let key = format!("{}{}", u.host_str().unwrap_or(""), u.path());
        self.asked.borrow_mut().push(key.clone());
        match self.bodies.get(&key) {
            Some(b) => Ok(Response {
                body: b.clone().into_bytes(),
                ..Response::default()
            }),
            None => Err(FetchError::Status {
                url: r.url.clone(),
                code: 404,
            }),
        }
    }
}

/// The review's H1: a publisher with URL authority syntax in it sent the
/// index request to another host, whose index then spoke for the domain.
#[test]
fn publisher_must_be_a_bare_domain() {
    let mut f = ByHost::default();
    let lure = "https://attacker.example/lure.screens.json";
    let forged = "https://attacker.example/forged.screens.json";
    f.bodies.insert(
        "attacker.example/.well-known/opencrs.json".into(),
        index("victim.example", &[doc(forged, "")], &[]),
    );
    f.bodies.insert(
        "attacker.example/lure.screens.json".into(),
        screens("victim.example:@attacker.example", "n", &["S1"]),
    );
    f.bodies.insert(
        "attacker.example/forged.screens.json".into(),
        screens("victim.example", "n", &["S1"]),
    );
    let e = verify(lure, &opts(), &f).unwrap_err().to_string();
    assert!(e.contains("isn't a domain name"), "{e}");
    assert!(
        !f.asked.borrow().iter().any(|k| k.contains("well-known")),
        "no index was fetched: {:?}",
        f.asked.borrow()
    );

    for bad in [
        "victim.example:@attacker.example",
        "victim.example:8443",
        "user@victim.example",
        "victim.example/path",
        "0x7f.1",
        "localhost:8080",
        "victim",
    ] {
        let e = discover(bad, &opts(), &f).unwrap_err().to_string();
        assert!(e.contains("isn't a publisher domain"), "{bad}: {e}");
    }
    assert!(!f.asked.borrow().iter().any(|k| k.contains("well-known")));
}

/// Every publisher value the schema allows is a domain here, and the
/// other way round.
#[test]
fn is_domain_matches_the_schema() {
    use crate::schema::{Kind, SchemaOptions, validate_schema};
    for p in [
        "harbourline.example",
        "a.b",
        "a-.b",
        "-a.b-",
        "1.2.3.4",
        "0x7f.1",
        "xn--bcher-kva.example",
        "a..b",
        ".a.b",
        "a.b.",
        "A.example",
        "a_b.example",
        "localhost",
        "a.example:443",
        "a.example/x",
        "u@a.example",
        "a.ex ample",
        "ä.example",
        "",
    ] {
        let doc = screens(p, "n", &["S1"]);
        let errs =
            validate_schema(Kind::Screens, doc.as_bytes(), &SchemaOptions::default()).unwrap();
        let schema_ok = !errs.iter().any(|e| e.path == "/publisher");
        assert_eq!(is_domain(p), schema_ok, "{p:?}");
    }
}

/// The review's L6: a listed URL on another host may be an internal
/// service; its values must not come back in the reasons.
#[test]
fn reasons_for_other_hosts_quote_no_values() {
    let mut f = Fake::default();
    let schema_bad = "https://internal.example/a.json";
    let type_bad = "https://internal.example/b.json";
    let own_bad = "https://p.example/c.json";
    f.put(
        idx_url("p.example"),
        index(
            "p.example",
            &[doc(schema_bad, ""), doc(type_bad, ""), doc(own_bad, "")],
            &[],
        ),
    );
    let leaky =
        r#"{"opencrs":"0.1","publisher":"p.example","updated_at":"SECRET-1","networks":[]}"#;
    f.put(schema_bad, leaky.into());
    f.put(type_bad, r#"{"networks":"SECRET-2"}"#.into());
    f.put(own_bad, leaky.into());
    let res = discover("p.example", &opts(), &f).unwrap();
    let a = by_url(&res, schema_bad);
    assert_eq!(a.status, Status::Rejected);
    assert_eq!(
        a.reason,
        "not a valid screens document (1 schema errors, first at /updated_at)"
    );
    let b = by_url(&res, type_bad);
    assert_eq!(b.status, Status::Rejected);
    assert!(!b.reason.contains("SECRET"), "{}", b.reason);
    // The publisher's own host keeps the full text.
    assert!(by_url(&res, own_bad).reason.contains("SECRET-1"));
}

/// Counts every fetch, repeats included.
struct Counting<'a>(&'a Fake, std::cell::Cell<usize>);

impl Fetcher for Counting<'_> {
    fn fetch(&self, r: &Request) -> Result<Response, FetchError> {
        self.1.set(self.1.get() + 1);
        self.0.fetch(r)
    }
}

/// The review's M6: one index listing a large document many times kept
/// every copy in memory.
#[test]
fn a_run_reads_a_bounded_number_of_bytes() {
    let mut f = Fake::default();
    let u = "https://p.example/a.screens.json";
    let body = screens("p.example", "n", &["S1", "S2"]);
    let idx = index("p.example", &vec![doc(u, ""); 50], &[]);
    let (idx_len, doc_len) = (idx.len(), body.len());
    f.put(idx_url("p.example"), idx);
    f.put(u, body);
    let opt = Options {
        max_run_bytes: idx_len + 3 * doc_len,
        ..opts()
    };
    let counting = Counting(&f, std::cell::Cell::new(0));
    let res = discover("p.example", &opt, &counting).unwrap();
    let trusted = res.trusted();
    assert_eq!(trusted.len(), 3);
    let failed: Vec<_> = res
        .documents
        .iter()
        .filter(|d| d.status == Status::Failed)
        .collect();
    assert_eq!(failed.len(), 47);
    assert!(
        failed[0].reason.contains("-byte limit"),
        "{}",
        failed[0].reason
    );
    assert!(failed.iter().all(|d| d.raw.is_empty()));
    // The index and three documents use the allowance up exactly; nothing
    // more is fetched.
    assert_eq!(counting.1.get(), 4);
    let opt = Options {
        max_run_bytes: idx_len + 3 * doc_len - 1,
        ..opts()
    };
    let counting = Counting(&f, std::cell::Cell::new(0));
    let res = discover("p.example", &opt, &counting).unwrap();
    assert_eq!(res.trusted().len(), 2);
    // The third went over, so it's dropped and nothing more is fetched.
    assert_eq!(counting.1.get(), 4);
}

#[test]
fn an_index_lists_a_bounded_number_of_documents() {
    let mut f = Fake::default();
    let docs: Vec<_> = (0..5)
        .map(|i| doc(&format!("https://p.example/{i}.json"), ""))
        .collect();
    f.put(idx_url("p.example"), index("p.example", &docs, &[]));
    let opt = Options {
        max_index_entries: 4,
        ..opts()
    };
    let e = discover("p.example", &opt, &f).unwrap_err().to_string();
    assert!(e.ends_with("lists 5 documents; at most 4 are read"), "{e}");
    let dels: Vec<_> = (0..5)
        .map(|i| delegate(&format!("d{i}.example"), ""))
        .collect();
    f.put(idx_url("p.example"), index("p.example", &[], &dels));
    let e = discover("p.example", &opt, &f).unwrap_err().to_string();
    assert!(e.ends_with("lists 5 delegates; at most 4 are read"), "{e}");
}

#[test]
fn only_trusted_documents_keep_their_bytes() {
    let mut f = Fake::default();
    let good = "https://p.example/good.json";
    let foreign = "https://p.example/foreign.json";
    f.put(
        idx_url("p.example"),
        index("p.example", &[doc(good, ""), doc(foreign, "")], &[]),
    );
    f.put(good, screens("p.example", "n", &["S"]));
    f.put(foreign, screens("q.example", "n", &["S"]));
    let res = discover("p.example", &opts(), &f).unwrap();
    assert!(!by_url(&res, good).raw.is_empty());
    let d = by_url(&res, foreign);
    assert_eq!(d.status, Status::Rejected);
    assert!(d.raw.is_empty());
}

/// A publisher chooses every URL a check fetches. Addresses, local-only names
/// and URLs with a password in them are rejected with a reason and never
/// requested, and an address can't be a delegate's domain.
#[test]
fn urls_that_point_inside_are_rejected_without_a_request() {
    let mut f = Fake::default();
    let bad = [
        "https://127.0.0.1:6379/x",
        "https://169.254.169.254/latest/meta-data/",
        "https://user:pw@spec.example/admin",
        "https://user@10.0.0.1:8443/admin",
        "https://[::1]/",
        "https://[fd00::1]/x",
        "https://0x7f.1/x",
        "https://2130706433/x",
        "https://metadata.google.internal/x",
        "https://printer.local/x",
        "https://localhost/x",
        "https://app.localhost/x",
    ];
    let good = "https://p.example/ok.screens.json";
    let mut docs: Vec<_> = bad.iter().map(|u| doc(u, "")).collect();
    docs.push(doc(good, ""));
    let dels = [
        delegate("10.0.0.5", ""),
        delegate("127.0.0.1", ""),
        delegate("169.254.169.254", ""),
        delegate("metadata.google.internal", ""),
    ];
    f.put(idx_url("p.example"), index("p.example", &docs, &dels));
    f.put(good, screens("p.example", "n", &["S1"]));
    let res = discover("p.example", &opts(), &f).unwrap();
    for u in bad {
        let d = by_url(&res, u);
        assert_eq!(d.status, Status::Rejected, "{u}");
        assert!(
            !d.reason.is_empty() && d.raw.is_empty(),
            "{u}: {}",
            d.reason
        );
    }
    assert_eq!(by_url(&res, good).status, Status::Trusted);
    for dr in &res.delegates {
        assert!(
            dr.error.contains("isn't a publisher domain"),
            "{}: {}",
            dr.domain,
            dr.error
        );
    }
    // Only the index and the one good document were requested.
    let mut asked: Vec<String> = f.tokens.borrow().keys().cloned().collect();
    asked.sort();
    let mut want = [good.to_string(), idx_url("p.example")];
    want.sort();
    assert_eq!(asked, want);
    // Local testing still reaches localhost.
    let local = Options {
        allow_local_http: true,
        ..opts()
    };
    assert!(unfetchable("http://localhost:8080/a.json", &local).is_none());
    assert!(unfetchable("http://localhost:8080/a.json", &opts()).is_some());
    assert!(is_dns_name("harbourline.example"));
    assert!(!is_dns_name("10.0.0.5") && !is_dns_name("a.1") && !is_dns_name("x.internal"));
}

/// One index can't make a check fetch without end: the run stops at its
/// limit, with a reason in `limits`, and a delegate gets a smaller share.
#[test]
fn fetches_are_capped_per_run_and_per_delegate() {
    let mut f = Fake::default();
    let urls = |host: &str, n: usize| -> Vec<IndexDocument> {
        (0..n)
            .map(|i| doc(&format!("https://{host}/d{i}.screens.json"), ""))
            .collect()
    };
    f.put(
        idx_url("p.example"),
        index(
            "p.example",
            &urls("cdn.example", 300),
            &[delegate("d1.example", ""), delegate("d2.example", "")],
        ),
    );
    for (d, victim) in [("d1.example", "v1.example"), ("d2.example", "v2.example")] {
        f.put(idx_url(d), index(d, &urls(victim, 400), &[]));
    }
    // A delegate's share is 250 documents.
    let res = discover("p.example", &opts(), &f).unwrap();
    let asked = f.tokens.borrow().len();
    // The index, 300 own documents, two delegate indexes and 250 each.
    assert_eq!(asked, 1 + 300 + 2 + 2 * 250, "{asked}");
    assert_eq!(res.limits.len(), 2, "{:?}", res.limits);
    assert!(
        res.limits[0].contains("only the first 250"),
        "{:?}",
        res.limits
    );

    // The run as a whole: 100 fetches in all.
    let f2 = Fake {
        bodies: f.bodies.clone(),
        ..Fake::default()
    };
    let opt = Options {
        max_fetches: 100,
        ..opts()
    };
    let res = discover("p.example", &opt, &f2).unwrap();
    assert_eq!(f2.tokens.borrow().len(), 100);
    assert_eq!(res.documents.len(), 99);
    assert!(
        res.limits[0].contains("only the first 99") && res.limits[0].contains("at most 100 URLs"),
        "{:?}",
        res.limits
    );
}
