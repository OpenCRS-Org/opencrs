//! The publisher check: what a media owner sees when it checks its own
//! published specs. Discovery runs over pages someone else has already
//! fetched (a web page's server, say), so the trust rules are the library's
//! and not a copy. Each trusted document is then linted, and the way each
//! file is served is compared with what the spec asks of publishers.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::batch::{LintResult, lint};
use crate::check::Severity;
use crate::discover::{
    DiscoveryResult, FetchError, Fetcher, Options, Request, Response, Status, discover, index_url,
};
use crate::schema::{Kind, SchemaOptions};

/// One page as fetched, with the response headers the check looks at. The
/// body is borrowed, so a page of tens of megabytes is never copied to be
/// held; it isn't part of the JSON form (see `opencrs-wasm`'s frame).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Page<'a> {
    /// The HTTP status; zero when the fetch itself failed.
    pub status: u16,
    #[serde(skip)]
    pub body: &'a [u8],
    /// Why the fetch failed, when it did.
    pub error: String,
    pub etag: String,
    pub cache_control: String,
    pub last_modified: String,
    pub content_type: String,
    pub content_encoding: String,
    /// `Access-Control-Allow-Origin`.
    pub allow_origin: String,
}

/// A URL discovery asked for that wasn't among the pages, and what it was
/// for, so a caller can fetch it with the care it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WantedUrl {
    pub url: String,
    /// `index` for a publisher's or delegate's `/.well-known/opencrs.json`,
    /// `document` for anything else.
    pub kind: &'static str,
    /// The fetch must not follow a redirect to another host (an index, or
    /// a request that would carry credentials).
    pub same_host: bool,
}

/// Serves discovery from fetched pages. A URL that wasn't fetched fails with
/// `missing`, and is recorded in `wanted`, once, in the order discovery asked
/// for it.
struct Pages<'a, 'b> {
    pages: &'a BTreeMap<String, Page<'b>>,
    missing: &'a str,
    wanted: RefCell<Vec<WantedUrl>>,
    asked: RefCell<HashSet<String>>,
}

impl Fetcher for Pages<'_, '_> {
    fn fetch(&self, req: &Request) -> Result<Response, FetchError> {
        let Some(p) = self.pages.get(&req.url) else {
            if self.asked.borrow_mut().insert(req.url.clone()) {
                self.wanted.borrow_mut().push(WantedUrl {
                    url: req.url.clone(),
                    kind: if req.index { "index" } else { "document" },
                    same_host: req.same_host,
                });
            }
            return Err(FetchError::Other(format!("{}: {}", req.url, self.missing)));
        };
        if !p.error.is_empty() {
            return Err(FetchError::Other(format!("{}: {}", req.url, p.error)));
        }
        if p.status != 200 {
            return Err(FetchError::Status {
                url: req.url.clone(),
                code: p.status,
            });
        }
        Ok(Response {
            body: p.body.to_vec(),
            etag: p.etag.clone(),
            cache_control: p.cache_control.clone(),
            last_modified: p.last_modified.clone(),
        })
    }
}

/// One thing for the publisher to know, about one file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Advice {
    pub severity: Severity,
    /// The URL it's about.
    pub url: String,
    pub message: String,
}

/// Everything the publisher check found.
#[derive(Debug, Clone, Serialize)]
pub struct PublisherCheck {
    pub domain: String,
    pub index_url: String,
    /// Why the index couldn't be used, if it couldn't. Nothing else is
    /// checked then.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// The index's own schema errors, when it was fetched but isn't valid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_lint: Option<LintResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery: Option<DiscoveryResult>,
    /// Each trusted document's own problems, by URL.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub lints: BTreeMap<String, LintResult>,
    pub advice: Vec<Advice>,
    pub errors: usize,
    pub warnings: usize,
    /// The URLs discovery asked for that aren't in the pages given, in the
    /// order it asked, each once. Fetch them, add them to the pages under
    /// exactly these strings (a failed fetch as a page with status 0 and an
    /// `error`) and check again; when it is empty the result is final. A URL
    /// already in the pages is never in it. The publisher chose every one of
    /// them, so fetch them with the care for an address a stranger picked.
    pub wanted: Vec<String>,
    /// The same URLs, with what each is for.
    pub wanted_detail: Vec<WantedUrl>,
}

/// A document's name for messages: the last part of its path.
fn name_of(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn date(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).ok()
}

/// Larger files than this should be served compressed.
const COMPRESS_FROM: usize = 64 << 10;

/// Compares how one file is served with what the spec asks.
fn served(out: &mut Vec<Advice>, url: &str, what: &str, p: &Page) {
    let mut say = |severity, message: String| {
        out.push(Advice {
            severity,
            url: url.to_string(),
            message,
        })
    };
    // Spec: publishers serve standard HTTP ETag and Cache-Control headers.
    if p.cache_control.is_empty() {
        say(
            Severity::Warning,
            format!(
                "{what} is served without a Cache-Control header. The spec asks for one, so consumers know how long they may keep a copy."
            ),
        );
    }
    if p.etag.is_empty() {
        say(
            Severity::Warning,
            format!(
                "{what} is served without an ETag. The spec asks for one, so consumers can re-check it daily without downloading it again."
            ),
        );
    }
    // Spec: documents are served compressed.
    if p.body.len() >= COMPRESS_FROM && p.content_encoding.is_empty() {
        say(
            Severity::Info,
            format!(
                "{what} is {} KB and isn't served compressed. The spec asks for compression, which usually makes a spec a tenth of the size.",
                p.body.len() >> 10
            ),
        );
    }
    // Not required, but it's what makes the file usable from other sites.
    if p.allow_origin.is_empty() {
        say(
            Severity::Info,
            format!(
                "{what} doesn't send Access-Control-Allow-Origin, so web tools on other sites can't read it directly. Sending `*` is safe for public specs."
            ),
        );
    }
    if !p.content_type.to_ascii_lowercase().contains("json") {
        let as_ = if p.content_type.is_empty() {
            "without a Content-Type".to_string()
        } else {
            format!("as {}", p.content_type)
        };
        say(
            Severity::Info,
            format!("{what} is served {as_}; application/json is expected."),
        );
    }
}

/// Checks a publisher's published specs from `pages`, keyed by URL. `missing`
/// says why a URL that discovery wanted isn't there (the fetcher's limits).
pub fn check_publisher(
    domain: &str,
    pages: &BTreeMap<String, Page<'_>>,
    missing: &str,
    now: DateTime<FixedOffset>,
) -> PublisherCheck {
    let bytes = pages.values().map(|p| p.body.len()).sum();
    crate::budget::ensure(bytes, || {
        check_publisher_within(domain, pages, missing, now)
    })
}

fn check_publisher_within(
    domain: &str,
    pages: &BTreeMap<String, Page<'_>>,
    missing: &str,
    now: DateTime<FixedOffset>,
) -> PublisherCheck {
    let domain = domain.to_ascii_lowercase();
    let mut res = PublisherCheck {
        index_url: index_url(&domain, false),
        domain: domain.clone(),
        error: String::new(),
        index_lint: None,
        discovery: None,
        lints: BTreeMap::new(),
        advice: Vec::new(),
        errors: 0,
        warnings: 0,
        wanted: Vec::new(),
        wanted_detail: Vec::new(),
    };
    let opt = Options {
        tokens: Default::default(),
        allow_local_http: false,
        now,
        max_index_entries: 0,
        max_run_bytes: 0,
        max_fetches: 0,
        // The pages are held already: a trusted document is linted from
        // its page, not from a second copy.
        keep_raw: false,
    };
    let fetcher = Pages {
        pages,
        missing,
        wanted: RefCell::new(Vec::new()),
        asked: RefCell::new(HashSet::new()),
    };
    let found = discover(&domain, &opt, &fetcher);
    res.wanted_detail = fetcher.wanted.take();
    res.wanted = res.wanted_detail.iter().map(|w| w.url.clone()).collect();
    let index_page = pages.get(&res.index_url);
    let found = match found {
        Ok(f) => f,
        Err(e) => {
            res.error = e.to_string();
            // An index that arrived but isn't valid: list everything wrong.
            if let Some(p) = index_page.filter(|p| p.status == 200 && p.error.is_empty()) {
                res.index_lint = lint(p.body, Some(Kind::Index), &SchemaOptions::default())
                    .ok()
                    .filter(|l| !l.ok());
            }
            res.errors = 1;
            return res;
        }
    };

    if let Some(p) = index_page {
        served(&mut res.advice, &res.index_url, "The index", p);
    }
    for why in &found.limits {
        res.advice.push(Advice {
            severity: Severity::Warning,
            url: res.index_url.clone(),
            message: why.clone(),
        });
    }
    if found.index.documents.is_empty() {
        res.advice.push(Advice {
            severity: Severity::Warning,
            url: res.index_url.clone(),
            message: "The index lists no documents, so there's nothing to check creative against."
                .into(),
        });
    }
    for del in &found.delegates {
        if !del.error.is_empty() {
            res.advice.push(Advice {
                severity: Severity::Warning,
                url: del.index_url.clone(),
                message: format!(
                    "The index names {} as a delegate, but: {}",
                    del.domain, del.error
                ),
            });
        }
    }
    for d in &found.documents {
        // The host may be asking this call to stop.
        if crate::budget::interrupted() {
            break;
        }
        let what = name_of(&d.url);
        let mut say = |severity, message: String| {
            res.advice.push(Advice {
                severity,
                url: d.url.clone(),
                message,
            })
        };
        match d.status {
            Status::Trusted | Status::Expired => {}
            Status::Skipped => {
                say(
                    Severity::Info,
                    format!(
                        "{what} is a partner document, so it can't be read without a token: {}.",
                        d.reason
                    ),
                );
                continue;
            }
            Status::Rejected | Status::Failed => {
                say(
                    Severity::Error,
                    format!("{what} can't be used: {}.", d.reason),
                );
                continue;
            }
        }
        if d.status == Status::Expired {
            say(
                Severity::Error,
                format!("{what} has expired: {}. Validators won't use it.", d.reason),
            );
        } else if let Some(exp) = date(&d.expires_at) {
            let days = (exp - now).num_days();
            if days <= 30 {
                say(
                    Severity::Warning,
                    format!(
                        "{what} expires on {}, in {days} day{}. After that validators won't use it.",
                        &d.expires_at[..10],
                        if days == 1 { "" } else { "s" }
                    ),
                );
            }
        }
        if let Some(up) = date(&d.updated_at)
            && (now - up).num_days() > 365
        {
            say(
                Severity::Info,
                format!(
                    "{what} was last updated on {}, more than a year ago. Worth a look to make sure it's still right.",
                    &d.updated_at[..10]
                ),
            );
        }
        if let Some(p) = pages.get(&d.url) {
            served(&mut res.advice, &d.url, &what, p);
        }
        // Each document has a work budget of its own, so one that is too
        // complex to check can't leave the rest unchecked.
        let body = pages.get(&d.url).map_or(&d.raw[..], |p| p.body);
        if d.status == Status::Trusted
            && let Ok(l) = crate::budget::measure(body.len(), || {
                lint(body, Some(Kind::Screens), &SchemaOptions::default())
            })
            .0
        {
            for c in &l.document_checks {
                match c.severity {
                    Severity::Error => res.errors += 1,
                    Severity::Warning => res.warnings += 1,
                    _ => {}
                }
            }
            if !l.document_checks.is_empty() {
                res.lints.insert(d.url.clone(), l);
            }
        }
    }
    for a in &res.advice {
        match a.severity {
            Severity::Error => res.errors += 1,
            Severity::Warning => res.warnings += 1,
            _ => {}
        }
    }
    res.discovery = Some(found);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN: &str = "harbourline.example";
    const INDEX: &str = "https://harbourline.example/.well-known/opencrs.json";
    const RETAIL: &str = "https://harbourline.example/opencrs/retail.screens.json";

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-25T12:00:00+11:00").unwrap()
    }

    fn example(rel: &str) -> String {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    /// A page for a test; the body is leaked so it can be borrowed for good.
    fn good(body: String) -> Page<'static> {
        Page {
            status: 200,
            body: Box::leak(body.into_bytes().into_boxed_slice()),
            etag: "\"1\"".into(),
            cache_control: "max-age=3600".into(),
            content_type: "application/json".into(),
            allow_origin: "*".into(),
            ..Page::default()
        }
    }

    fn harbourline() -> BTreeMap<String, Page<'static>> {
        let index = example("examples/harbourline/.well-known/opencrs.json");
        let mut pages = BTreeMap::new();
        pages.insert(INDEX.to_string(), good(index));
        pages.insert(
            RETAIL.to_string(),
            good(example("examples/harbourline/retail.screens.json")),
        );
        pages
    }

    #[test]
    fn a_well_served_publisher_is_clean() {
        let pages = harbourline();
        let r = check_publisher(DOMAIN, &pages, "not fetched", now());
        assert!(r.error.is_empty(), "{}", r.error);
        let d = r.discovery.as_ref().unwrap();
        assert_eq!(d.documents.len(), 1);
        assert_eq!(d.documents[0].status, Status::Trusted, "{d:?}");
        assert!(r.advice.is_empty(), "{:?}", r.advice);
        assert_eq!((r.errors, r.warnings), (0, 0));
    }

    #[test]
    fn wanted_lists_unfetched_urls_once_and_never_a_fetched_one() {
        // Nothing fetched: the index.
        let mut pages = BTreeMap::new();
        let r = check_publisher(DOMAIN, &pages, "not fetched", now());
        assert_eq!(r.wanted, [INDEX]);
        assert!(r.error.contains("not fetched"), "{}", r.error);

        // The index is in: its document.
        let all = harbourline();
        pages.insert(INDEX.to_string(), all[INDEX].clone());
        let r = check_publisher(DOMAIN, &pages, "not fetched", now());
        assert_eq!(r.wanted, [RETAIL]);

        // A failed fetch counts as fetched.
        pages.insert(
            RETAIL.to_string(),
            Page {
                error: "timed out".into(),
                ..Page::default()
            },
        );
        let r = check_publisher(DOMAIN, &pages, "not fetched", now());
        assert!(r.wanted.is_empty(), "{:?}", r.wanted);

        let r = check_publisher(DOMAIN, &all, "not fetched", now());
        assert!(r.wanted.is_empty(), "{:?}", r.wanted);
    }

    #[test]
    fn missing_headers_are_spec_warnings_and_cors_is_advice() {
        let mut pages = harbourline();
        let p = pages.get_mut(RETAIL).unwrap();
        p.etag.clear();
        p.cache_control.clear();
        p.allow_origin.clear();
        let r = check_publisher(DOMAIN, &pages, "not fetched", now());
        let about: Vec<_> = r
            .advice
            .iter()
            .filter(|a| a.url == RETAIL)
            .map(|a| a.severity)
            .collect();
        assert_eq!(
            about,
            [Severity::Warning, Severity::Warning, Severity::Info]
        );
    }

    #[test]
    fn a_document_that_wasnt_fetched_fails_with_the_reason() {
        let mut pages = harbourline();
        pages.remove(RETAIL);
        let r = check_publisher(DOMAIN, &pages, "not fetched (limit)", now());
        assert_eq!(r.errors, 1);
        assert!(
            r.advice[0].message.ends_with("not fetched (limit)."),
            "{}",
            r.advice[0].message
        );
    }

    #[test]
    fn an_unusable_index_stops_the_check() {
        let mut pages = BTreeMap::new();
        pages.insert(INDEX.to_string(), good(r#"{"opencrs":"0.1"}"#.into()));
        let r = check_publisher(DOMAIN, &pages, "not fetched", now());
        assert!(r.error.contains("isn't a valid index"), "{}", r.error);
        assert!(r.index_lint.is_some());
        assert_eq!(r.errors, 1);

        let r = check_publisher(DOMAIN, &BTreeMap::new(), "not fetched", now());
        assert!(r.error.contains("fetching the index"), "{}", r.error);

        let mut pages = BTreeMap::new();
        pages.insert(
            INDEX.to_string(),
            Page {
                status: 404,
                ..Page::default()
            },
        );
        let r = check_publisher(DOMAIN, &pages, "not fetched", now());
        assert!(r.error.contains("HTTP 404"), "{}", r.error);
    }

    #[test]
    fn expiry_soon_and_old_updates_are_flagged() {
        let mut pages = harbourline();
        let p = pages.get_mut(RETAIL).unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(p.body).unwrap();
        v["expires_at"] = "2026-11-05T00:00:00+11:00".into();
        v["updated_at"] = "2025-01-01T00:00:00+11:00".into();
        p.body = Box::leak(v.to_string().into_bytes().into_boxed_slice());
        let r = check_publisher(DOMAIN, &pages, "not fetched", now());
        let msgs: Vec<_> = r
            .advice
            .iter()
            .filter(|a| a.url == RETAIL)
            .map(|a| (a.severity, a.message.as_str()))
            .collect();
        assert_eq!(msgs.len(), 2, "{msgs:?}");
        assert_eq!(msgs[0].0, Severity::Warning);
        assert!(
            msgs[0]
                .1
                .starts_with("retail.screens.json expires on 2026-11-05, in 10 days"),
            "{}",
            msgs[0].1
        );
        assert_eq!(msgs[1].0, Severity::Info);
    }

    #[test]
    fn document_problems_come_from_lint() {
        let mut pages = harbourline();
        let p = pages.get_mut(RETAIL).unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(p.body).unwrap();
        v["networks"][0]["rules"]["rule_packs"] =
            serde_json::json!(["au-nsw-transport-corridor@1999-01"]);
        p.body = Box::leak(v.to_string().into_bytes().into_boxed_slice());
        let r = check_publisher(DOMAIN, &pages, "not fetched", now());
        let l = r.lints.get(RETAIL).expect("a lint for retail");
        assert_eq!(l.document_checks[0].code, "CRS-RULE-PACK");
        assert!(r.warnings >= 1);
    }
}
