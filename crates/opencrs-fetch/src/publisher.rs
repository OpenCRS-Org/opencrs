//! The publisher check end to end: ask the core which URLs it wants, fetch
//! them, ask again. Port of `FetchPublisher` and `FetchPage` in `fetch.go`,
//! with the same limits and the same recorded errors.

use crate::http::Http;
use chrono::{DateTime, FixedOffset};
use opencrs::discover::{MAX_DOCUMENT_BYTES, Request};
use opencrs::publisher::{Page, PublisherCheck, WantedUrl, check_publisher};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use url::Url;

/// How much [`fetch_publisher`] reads in all, as Go's `DefaultMaxTotalBytes`.
pub const DEFAULT_MAX_TOTAL_BYTES: usize = 180 << 20;

/// How many times [`fetch_publisher`] asks the core. The core needs at most
/// three: the index, then its documents and delegate indexes, then the
/// delegates' documents.
pub const DEFAULT_MAX_ROUNDS: usize = 5;

/// How many URLs [`fetch_publisher`] fetches in all.
pub const DEFAULT_MAX_PAGES: usize = 4000;

/// Controls [`fetch_publisher`]. Zero means the default, for every limit.
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// The most one page may be; default 50 MiB.
    pub max_document_bytes: usize,
    /// The most all pages may be together; default 180 MiB. A page past it
    /// is recorded as not fetched.
    pub max_total_bytes: usize,
    /// How many times the core is asked; default 5.
    pub max_rounds: usize,
    /// How many URLs are fetched in all; default 4000. A URL past it is
    /// recorded as not fetched.
    pub max_pages: usize,
    /// For one request, redirects included; default 2 minutes, as Go's
    /// `SafeClient`.
    pub timeout: Duration,
}

/// The result of [`fetch_publisher`].
#[derive(Debug, Clone)]
pub struct Fetched {
    /// The publisher check after the last round.
    pub check: PublisherCheck,
    /// Set when the core still wanted URLs after the last round (Go's
    /// `ErrTooManyRounds`): `check.wanted` says which.
    pub rounds_exhausted: bool,
    /// How many rounds the limit allowed.
    pub rounds: usize,
}

/// A page as held between rounds: [`Page`] owns nothing, so the bodies live
/// here and each round borrows them.
#[derive(Default)]
struct Held {
    status: u16,
    body: Vec<u8>,
    error: String,
    etag: String,
    cache_control: String,
    last_modified: String,
    content_type: String,
    content_encoding: String,
    allow_origin: String,
}

impl Held {
    fn failed(error: String) -> Held {
        Held {
            error,
            ..Held::default()
        }
    }

    fn page(&self) -> Page<'_> {
        Page {
            status: self.status,
            body: &self.body,
            error: self.error.clone(),
            etag: self.etag.clone(),
            cache_control: self.cache_control.clone(),
            last_modified: self.last_modified.clone(),
            content_type: self.content_type.clone(),
            content_encoding: self.content_encoding.clone(),
            allow_origin: self.allow_origin.clone(),
        }
    }
}

/// Fetches one wanted URL and returns it as a page, never failing: a fetch
/// that did not work has status 0 and an `error`, and a response that is not
/// 200 has its status. A page of more than `max_bytes` (zero means 50 MiB)
/// is an error. Only HTTPS is fetched, with no user name or password;
/// `w.same_host` makes a redirect to another host an error. The body is read
/// as served with gzip undone, and the headers the check looks at are kept.
fn fetch_held(http: &Http, allow_local: bool, w: &WantedUrl, max_bytes: usize) -> Held {
    let max_bytes = if max_bytes == 0 {
        MAX_DOCUMENT_BYTES
    } else {
        max_bytes
    };
    let Ok(u) = Url::parse(&w.url) else {
        return Held::failed(format!(
            "parse {}: invalid URL",
            opencrs::gofmt::quote(&w.url)
        ));
    };
    let local = allow_local && opencrs::discover::scheme_allowed(&u, true);
    if !(u.scheme() == "https" || local) || !u.username().is_empty() || u.password().is_some() {
        return Held::failed(format!(
            "{}: only https:// URLs without a user name can be fetched",
            redacted(&u)
        ));
    }
    let req = Request {
        url: w.url.clone(),
        token: None,
        same_host: w.same_host,
        index: w.kind == "index",
    };
    match http.get(&req, max_bytes) {
        Ok(r) => Held {
            status: r.status,
            body: r.body,
            error: String::new(),
            etag: r.etag,
            cache_control: r.cache_control,
            last_modified: r.last_modified,
            content_type: r.content_type,
            content_encoding: r.content_encoding,
            allow_origin: r.allow_origin,
        },
        Err(e) => Held::failed(e.to_string()),
    }
}

/// A URL for messages, without a password.
fn redacted(u: &Url) -> String {
    if u.password().is_some() {
        let mut v = u.clone();
        let _ = v.set_password(Some("xxxxx"));
        v.to_string()
    } else {
        u.to_string()
    }
}

/// Runs the publisher check end to end: asks the core which URLs it wants,
/// fetches them, and asks again, until nothing more is wanted or
/// `opts.max_rounds` is reached. Every URL was chosen by the publisher, so
/// the fetching is as careful as the command line's: HTTPS only, no
/// connection to a non-public address (checked after DNS and on every
/// redirect), no proxy. A fetch that fails is recorded as a failed page,
/// which the result reports against its document. Pages are fetched one at a
/// time, in the order the core asks.
pub fn fetch_publisher(domain: &str, now: DateTime<FixedOffset>, opts: &FetchOptions) -> Fetched {
    let timeout = if opts.timeout.is_zero() {
        Duration::from_secs(120)
    } else {
        opts.timeout
    };
    let http = Http::new(false, timeout);
    run(domain, now, opts, &mut |w, limit| {
        fetch_held(&http, false, w, limit)
    })
}

/// The loop of [`fetch_publisher`], with the fetching handed in so tests can
/// serve the pages from somewhere local.
fn run(
    domain: &str,
    now: DateTime<FixedOffset>,
    opts: &FetchOptions,
    fetch: &mut dyn FnMut(&WantedUrl, usize) -> Held,
) -> Fetched {
    let or = |v: usize, d: usize| if v == 0 { d } else { v };
    let max_doc = or(opts.max_document_bytes, MAX_DOCUMENT_BYTES);
    let max_total = or(opts.max_total_bytes, DEFAULT_MAX_TOTAL_BYTES);
    let max_rounds = or(opts.max_rounds, DEFAULT_MAX_ROUNDS);
    let max_pages = or(opts.max_pages, DEFAULT_MAX_PAGES);
    let mut held: HashMap<String, Held> = HashMap::new();
    let mut total = 0usize;
    let mut last = None;
    for _ in 0..max_rounds {
        let pages: BTreeMap<String, Page<'_>> =
            held.iter().map(|(u, h)| (u.clone(), h.page())).collect();
        let res = check_publisher(domain, &pages, "not fetched", now);
        drop(pages);
        if res.wanted.is_empty() {
            return Fetched {
                check: res,
                rounds_exhausted: false,
                rounds: max_rounds,
            };
        }
        for w in &res.wanted_detail {
            if held.contains_key(&w.url) {
                continue; // never fetched twice, whatever the core says
            }
            let h = if held.len() >= max_pages {
                Held::failed(format!(
                    "not fetched: a check fetches at most {max_pages} URLs"
                ))
            } else if total >= max_total {
                Held::failed(format!(
                    "not fetched: a check reads at most {max_total} bytes in all"
                ))
            } else {
                let limit = max_doc.min(max_total - total);
                let h = fetch(w, limit);
                total += h.body.len();
                h
            };
            held.insert(w.url.clone(), h);
        }
        last = Some(res);
    }
    Fetched {
        check: last.expect("at least one round"),
        rounds_exhausted: true,
        rounds: max_rounds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashMap;

    use crate::server::{Resp, Server};
    use crate::tls_server::TlsServer;

    fn wanted(url: &str) -> WantedUrl {
        WantedUrl {
            url: url.into(),
            kind: "document",
            same_host: false,
        }
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-25T12:00:00Z").unwrap()
    }

    const DOC: &str = r#"{"opencrs":"0.1","publisher":"harbourline.example","updated_at":"2026-10-20T09:00:00Z","networks":[{"id":"retail","name":"Retail","formats":[{"id":"f","pixels":[1080,1920],"media":{"static":["image/png"]}}],"screens":[{"id":"s1","format":"f"}]}]}"#;

    /// A local server standing in for harbourline.example.
    fn publisher() -> Server {
        Server::start(|r, _| {
            match r.path.as_str() {
            "/.well-known/opencrs.json" => Resp::ok(
                r#"{"opencrs":"0.1","publisher":{"id":"harbourline.example","name":"Harbourline"},"documents":[{"url":"https://harbourline.example/retail.screens.json","network":"retail"}]}"#,
            )
            .header("Content-Type", "application/json")
            .header("Cache-Control", "max-age=60")
            .header("ETag", "\"i1\""),
            "/retail.screens.json" => Resp::ok(gzip(DOC.as_bytes()))
                .header("Content-Type", "application/json")
                .header("Content-Encoding", "gzip")
                .header("Cache-Control", "max-age=60")
                .header("ETag", "\"d1\""),
            _ => Resp::status(404),
        }
        })
    }

    /// Fetches from `srv` over plain HTTP what the check asks of
    /// https://harbourline.example.
    fn local<'a>(
        srv: &'a Server,
        calls: &'a Cell<usize>,
    ) -> impl FnMut(&WantedUrl, usize) -> Held + 'a {
        let http = Http::new(true, Duration::from_secs(10));
        move |w, limit| {
            calls.set(calls.get() + 1);
            let url = w.url.replace("https://harbourline.example", &srv.url());
            fetch_held(&http, true, &WantedUrl { url, ..w.clone() }, limit)
        }
    }

    #[test]
    fn the_loop_fetches_what_the_core_wants() {
        let srv = publisher();
        let calls = Cell::new(0);
        let f = run(
            "harbourline.example",
            now(),
            &FetchOptions::default(),
            &mut local(&srv, &calls),
        );
        assert!(!f.rounds_exhausted);
        assert!(f.check.wanted.is_empty(), "{:?}", f.check.wanted);
        assert_eq!(f.check.errors, 0, "{:?}", f.check.advice);
        assert_eq!(calls.get(), 2, "the index, then its one document");
        let docs = &f.check.discovery.as_ref().unwrap().documents;
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].status.to_string(), "trusted", "{}", docs[0].reason);
        assert_eq!(docs[0].screens, 1, "the gzipped body was read");
        // Served with Cache-Control and an ETag, so only the CORS note.
        let notes: Vec<_> = f.check.advice.iter().map(|a| a.message.as_str()).collect();
        assert!(
            notes
                .iter()
                .all(|m| m.contains("Access-Control-Allow-Origin")),
            "{notes:?}"
        );
        assert_eq!(f.check.warnings, 0);
        let seen = srv.seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "each URL once: {seen:?}");
    }

    #[test]
    fn too_few_rounds_leave_urls_wanted() {
        let srv = publisher();
        let calls = Cell::new(0);
        let opts = FetchOptions {
            max_rounds: 1,
            ..FetchOptions::default()
        };
        let f = run(
            "harbourline.example",
            now(),
            &opts,
            &mut local(&srv, &calls),
        );
        assert!(f.rounds_exhausted);
        assert_eq!(
            f.check.wanted,
            ["https://harbourline.example/.well-known/opencrs.json"]
        );
    }

    #[test]
    fn page_and_byte_caps_are_recorded_as_not_fetched() {
        let srv = publisher();
        let calls = Cell::new(0);
        let opts = FetchOptions {
            max_pages: 1,
            ..FetchOptions::default()
        };
        let f = run(
            "harbourline.example",
            now(),
            &opts,
            &mut local(&srv, &calls),
        );
        let docs = &f.check.discovery.as_ref().unwrap().documents;
        assert!(
            docs[0].reason.contains("a check fetches at most 1 URLs"),
            "{}",
            docs[0].reason
        );
        assert_eq!(calls.get(), 1);
        let opts = FetchOptions {
            max_total_bytes: 1,
            ..FetchOptions::default()
        };
        let f = run(
            "harbourline.example",
            now(),
            &opts,
            &mut local(&srv, &calls),
        );
        // The index alone is past one byte, so the limit fails it.
        assert!(
            f.check.error.contains("larger than the 1-byte limit"),
            "{}",
            f.check.error
        );
    }

    #[test]
    fn a_page_is_read_with_its_headers() {
        let srv = Server::start(|r, _| match r.path.as_str() {
            "/ok" => Resp::ok("{}")
                .header("ETag", "\"v1\"")
                .header("Cache-Control", "max-age=60")
                .header("Last-Modified", "Mon, 01 Jan 2024 00:00:00 GMT")
                .header("Content-Type", "application/json")
                .header("Access-Control-Allow-Origin", "*"),
            "/zip" => Resp::ok(gzip(b"{\"a\":1}")).header("Content-Encoding", "gzip"),
            "/bomb" => Resp::ok(gzip(&vec![b'x'; 1 << 20])).header("Content-Encoding", "gzip"),
            "/big" => Resp::ok("x".repeat(1000)),
            "/redirect" => Resp::redirect("/ok"),
            "/other" => Resp::redirect("https://example.com:1/ok"),
            _ => Resp::status(404),
        });
        let http = Http::new(true, Duration::from_secs(10));
        let get = |path: &str, same_host: bool, max: usize| {
            let w = WantedUrl {
                url: format!("{}{path}", srv.url()),
                kind: "document",
                same_host,
            };
            fetch_held(&http, true, &w, max)
        };
        let h = get("/ok", false, 0);
        assert_eq!(h.status, 200);
        assert_eq!((h.body.as_slice(), h.error.as_str()), (&b"{}"[..], ""));
        assert_eq!(h.etag, "\"v1\"");
        assert_eq!(h.cache_control, "max-age=60");
        assert_eq!(h.last_modified, "Mon, 01 Jan 2024 00:00:00 GMT");
        assert_eq!(h.content_type, "application/json");
        assert_eq!(h.allow_origin, "*");
        assert_eq!(h.content_encoding, "");
        // Gzip is undone, and said to have been used.
        let h = get("/zip", false, 0);
        assert_eq!(h.body, b"{\"a\":1}");
        assert_eq!(h.content_encoding, "gzip");
        // The limit is on the decompressed size.
        let h = get("/bomb", false, 1000);
        assert!(
            h.error.contains("larger than the 1000-byte limit"),
            "{}",
            h.error
        );
        assert!(h.body.is_empty());
        let h = get("/big", false, 999);
        assert!(h.error.contains("larger than"), "{}", h.error);
        // A status that isn't 200 is a page, not an error.
        let h = get("/nope", false, 0);
        assert_eq!((h.status, h.error.as_str(), h.body.len()), (404, "", 0));
        assert_eq!(get("/redirect", true, 0).status, 200);
        let h = get("/other", true, 0);
        assert!(h.error.contains("another host"), "{}", h.error);
        assert_eq!(h.status, 0);
    }

    /// The addresses an index could aim the check at are refused, and
    /// nothing connects to them.
    #[test]
    fn non_public_addresses_are_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let http = Http::new(false, Duration::from_secs(2));
        for url in [
            format!("https://127.0.0.1:{port}/x"),
            format!("https://localhost:{port}/x"),
            format!("https://[::ffff:127.0.0.1]:{port}/x"),
            "https://169.254.169.254/latest/meta-data/".to_string(),
            "https://10.0.0.5/x".to_string(),
            "https://192.168.0.1/x".to_string(),
            "https://100.64.0.1/x".to_string(),
            "https://[fd00::1]/x".to_string(),
        ] {
            let h = fetch_held(&http, false, &wanted(&url), 0);
            assert!(
                h.error.contains("refusing to connect"),
                "{url}: {}",
                h.error
            );
            assert_eq!((h.status, h.body.len()), (0, 0), "{url}");
        }
        assert!(
            matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "something connected"
        );
        // Not even a URL of the wrong kind gets as far as a request.
        for url in [
            "http://example.com/x",
            "ftp://example.com/x",
            "https://user:pw@example.com/x",
            "https://user@example.com/x",
            "http://localhost:1/x",
        ] {
            let h = fetch_held(&http, false, &wanted(url), 0);
            assert!(h.error.contains("only https:// URLs"), "{url}: {}", h.error);
        }
        assert!(
            !fetch_held(&http, false, &wanted("not a url"), 0)
                .error
                .is_empty()
        );
    }

    /// Even in local testing mode a redirect can't lead to a private address.
    #[test]
    fn redirects_to_private_addresses_are_refused_in_local_mode() {
        let srv = Server::start(|r, _| match r.path.as_str() {
            "/metadata" => Resp::redirect("https://169.254.169.254/latest/meta-data/"),
            _ => Resp::status(404),
        });
        let http = Http::new(true, Duration::from_secs(2));
        let h = fetch_held(&http, true, &wanted(&format!("{}/metadata", srv.url())), 0);
        assert!(h.error.contains("refusing to connect"), "{}", h.error);
    }

    /// Discovery with the real client, against a local publisher in local
    /// testing mode, and without the mode.
    #[test]
    fn discover_reads_a_local_publisher() {
        let srv = Server::start(|r, addr| match r.path.as_str() {
            "/.well-known/opencrs.json" => Resp::ok(format!(
                r#"{{"opencrs":"0.1","publisher":{{"id":"harbourline.example","name":"Harbourline"}},"documents":[{{"url":"http://{addr}/retail.screens.json","network":"retail"}}]}}"#
            )),
            "/retail.screens.json" => Resp::ok(gzip(DOC.as_bytes()))
                .header("Content-Encoding", "gzip")
                .header("Cache-Control", "max-age=60"),
            _ => Resp::status(404),
        });
        let tokens = HashMap::new();
        let r = crate::discover(&srv.addr, &tokens, true, now()).unwrap();
        assert_eq!(r.index.publisher.name, "Harbourline");
        assert_eq!(r.documents.len(), 1);
        assert_eq!(r.documents[0].status.to_string(), "trusted");
        assert_eq!(r.documents[0].cache_control, "max-age=60");
        assert!(r.documents[0].raw.is_empty(), "raw is not kept");
        let e = crate::discover(&srv.addr, &tokens, false, now()).unwrap_err();
        assert!(e.to_string().contains("isn't a publisher domain"), "{e}");
    }

    // The same client over a real TLS handshake, against a local server with
    // a certificate of its own.

    fn https_publisher() -> TlsServer {
        TlsServer::start(|r, _| {
            match r.path.as_str() {
            "/.well-known/opencrs.json" => Resp::ok(
                r#"{"opencrs":"0.1","publisher":{"id":"harbourline.example","name":"Harbourline"},"documents":[{"url":"https://harbourline.example/retail.screens.json","network":"retail"}]}"#,
            )
            .header("Content-Type", "application/json")
            .header("Cache-Control", "max-age=60")
            .header("ETag", "\"i1\""),
            "/retail.screens.json" => Resp::ok(gzip(DOC.as_bytes()))
                .header("Content-Type", "application/json")
                .header("Content-Encoding", "gzip")
                .header("Cache-Control", "max-age=60")
                .header("ETag", "\"d1\""),
            _ => Resp::status(404),
        }
        })
    }

    #[test]
    fn https_pages_are_read_with_their_headers() {
        let srv = TlsServer::start(|r, port| match r.path.as_str() {
            "/ok" => Resp::ok("{}")
                .header("ETag", "\"v1\"")
                .header("Content-Type", "application/json")
                .header("Access-Control-Allow-Origin", "*"),
            "/zip" => Resp::ok(gzip(b"{\"a\":1}")).header("Content-Encoding", "gzip"),
            "/redirect" => Resp::redirect("/ok"),
            // HTTPS to HTTP, even to the machine itself, is refused.
            "/down" => Resp::redirect(&format!("http://localhost:{port}/ok")),
            "/private" => Resp::redirect("https://10.0.0.1/x"),
            _ => Resp::status(404),
        });
        let http = Http::with_roots(true, Duration::from_secs(10), srv.roots());
        let get = |path: &str| {
            let w = wanted(&format!("{}{path}", srv.url()));
            fetch_held(&http, true, &w, 0)
        };
        let h = get("/ok");
        assert_eq!(
            (h.status, h.body.as_slice(), h.error.as_str()),
            (200, &b"{}"[..], "")
        );
        assert_eq!(h.etag, "\"v1\"");
        assert_eq!(h.content_type, "application/json");
        assert_eq!(h.allow_origin, "*");
        let h = get("/zip");
        assert_eq!(
            (h.body.as_slice(), h.content_encoding.as_str()),
            (&b"{\"a\":1}"[..], "gzip")
        );
        assert_eq!(get("/redirect").status, 200);
        assert_eq!(get("/nope").status, 404);
        let h = get("/down");
        assert!(
            h.error.contains("refusing redirect") && h.error.contains("not HTTPS"),
            "{}",
            h.error
        );
        assert_eq!(h.status, 0);
        let h = get("/private");
        assert!(
            h.error.contains("refusing to connect to 10.0.0.1"),
            "{}",
            h.error
        );
    }

    #[test]
    fn a_certificate_the_system_does_not_trust_is_refused() {
        let srv = TlsServer::start(|_, _| Resp::ok("{}"));
        let http = Http::new(true, Duration::from_secs(10));
        let h = fetch_held(&http, true, &wanted(&format!("{}/ok", srv.url())), 0);
        assert_eq!((h.status, h.body.len()), (0, 0));
        assert!(
            !h.error.is_empty(),
            "a self-signed certificate was accepted"
        );
    }

    #[test]
    fn the_loop_runs_over_https() {
        let srv = https_publisher();
        let http = Http::with_roots(true, Duration::from_secs(10), srv.roots());
        let calls = Cell::new(0);
        let f = run(
            "harbourline.example",
            now(),
            &FetchOptions::default(),
            &mut |w, limit| {
                calls.set(calls.get() + 1);
                let url = w.url.replace("https://harbourline.example", &srv.url());
                fetch_held(&http, true, &WantedUrl { url, ..w.clone() }, limit)
            },
        );
        assert!(!f.rounds_exhausted);
        assert_eq!(f.check.errors, 0, "{:?}", f.check.advice);
        assert_eq!(calls.get(), 2);
        let docs = &f.check.discovery.as_ref().unwrap().documents;
        assert_eq!(docs[0].status.to_string(), "trusted", "{}", docs[0].reason);
        assert_eq!(docs[0].screens, 1, "the gzipped body was read");
        assert_eq!(
            *srv.seen.lock().unwrap(),
            ["/.well-known/opencrs.json", "/retail.screens.json"]
        );
    }
}
