//! The publisher check end to end: ask the core which URLs it wants, fetch
//! them, ask again. Port of `FetchPublisher` and `FetchPage` in `fetch.go`,
//! with the same limits and the same recorded errors.

use crate::http::{Control, Http};
use chrono::{DateTime, FixedOffset};
use opencrs::discover::{MAX_DOCUMENT_BYTES, Request};
use opencrs::publisher::{Page, PublisherCheck, WantedUrl, check_publisher};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use url::Url;

/// How much [`fetch_publisher`] reads in all, as Go's `DefaultMaxTotalBytes`.
/// Bytes are counted as they are read: from the network, and again once
/// decompressed.
pub const DEFAULT_MAX_TOTAL_BYTES: usize = 180 << 20;

/// The most any caller may ask [`fetch_publisher`] to read in all.
pub const MAX_TOTAL_BYTES: usize = 512 << 20;

/// How many times [`fetch_publisher`] asks the core. The core needs at most
/// three: the index, then its documents and delegate indexes, then the
/// delegates' documents.
pub const DEFAULT_MAX_ROUNDS: usize = 5;

/// How many URLs [`fetch_publisher`] fetches in all.
pub const DEFAULT_MAX_PAGES: usize = 4000;

/// How long a whole check may take unless told otherwise. A check is a few
/// small requests: an index, then its documents, a round or two more for a
/// delegate. Five minutes is generous for that, even over a slow link or for
/// documents of tens of megabytes, and it bounds what a publisher that
/// answers as slowly as it can can cost.
pub const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);

/// The longest any timeout may be.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(3600);

/// Controls [`fetch_publisher`]. Zero means the default, for every limit;
/// a limit above the most allowed is the most allowed.
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// The most one page may be; default and most 50 MiB.
    pub max_document_bytes: usize,
    /// The most all pages may be together, counting bytes read from the
    /// network and again decompressed; default 180 MiB, most 512 MiB. A page
    /// past it is recorded as not fetched.
    pub max_total_bytes: usize,
    /// How many times the core is asked; default 5, most 10.
    pub max_rounds: usize,
    /// How many URLs are fetched in all; default 4000, most 10,000. A URL
    /// past it is recorded as not fetched.
    pub max_pages: usize,
    /// For one request, redirects included; default 2 minutes, as Go's
    /// `SafeClient`; most an hour.
    pub timeout: Duration,
    /// For the whole check; default 5 minutes ([`DEFAULT_TOTAL_TIMEOUT`]);
    /// most an hour. A request never runs past it, and a URL not yet
    /// fetched when it passes is recorded as not fetched.
    pub total_timeout: Duration,
    /// Set by another thread to stop the check between pages and within a
    /// read. [`Fetched::cancelled`] says it did.
    pub cancel: Option<Arc<AtomicBool>>,
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
    /// Set when `FetchOptions::cancel` stopped the check. `check` is then
    /// from the last round that finished.
    pub cancelled: bool,
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
/// What it reads is counted into `control` however it ends.
fn fetch_held_with(
    http: &Http,
    allow_local: bool,
    w: &WantedUrl,
    max_bytes: usize,
    control: &Control,
) -> Held {
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
    match http.get_controlled(&req, max_bytes, control) {
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

#[cfg(test)]
fn fetch_held(http: &Http, allow_local: bool, w: &WantedUrl, max_bytes: usize) -> Held {
    fetch_held_with(http, allow_local, w, max_bytes, &Control::default())
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
/// time, in the order the core asks. The whole check is held to
/// `opts.total_timeout`, and `opts.cancel` stops it early.
pub fn fetch_publisher(domain: &str, now: DateTime<FixedOffset>, opts: &FetchOptions) -> Fetched {
    let timeout = if opts.timeout.is_zero() {
        Duration::from_secs(120)
    } else {
        opts.timeout.min(MAX_TIMEOUT)
    };
    let http = Http::new(false, timeout);
    run(domain, now, opts, &mut |w, limit, control| {
        fetch_held_with(&http, false, w, limit, control)
    })
}

/// The loop of [`fetch_publisher`], with the fetching handed in so tests can
/// serve the pages from somewhere local.
fn run(
    domain: &str,
    now: DateTime<FixedOffset>,
    opts: &FetchOptions,
    fetch: &mut dyn FnMut(&WantedUrl, usize, &Control) -> Held,
) -> Fetched {
    let or = |v: usize, d: usize, most: usize| if v == 0 { d } else { v.min(most) };
    let max_doc = or(
        opts.max_document_bytes,
        MAX_DOCUMENT_BYTES,
        MAX_DOCUMENT_BYTES,
    );
    let max_total = or(
        opts.max_total_bytes,
        DEFAULT_MAX_TOTAL_BYTES,
        MAX_TOTAL_BYTES,
    );
    let max_rounds = or(opts.max_rounds, DEFAULT_MAX_ROUNDS, 10);
    let max_pages = or(opts.max_pages, DEFAULT_MAX_PAGES, 10_000);
    let total_timeout = if opts.total_timeout.is_zero() {
        DEFAULT_TOTAL_TIMEOUT
    } else {
        opts.total_timeout.min(MAX_TIMEOUT)
    };
    let control = Control::new(
        Instant::now().checked_add(total_timeout),
        opts.cancel.clone(),
    );
    let mut held: HashMap<String, Held> = HashMap::new();
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
                cancelled: control.cancelled(),
            };
        }
        for w in &res.wanted_detail {
            if held.contains_key(&w.url) {
                continue; // never fetched twice, whatever the core says
            }
            if control.cancelled() {
                return Fetched {
                    check: res,
                    rounds_exhausted: false,
                    rounds: max_rounds,
                    cancelled: true,
                };
            }
            // What was read counts, whether or not the page was kept.
            let total = control.bytes_read();
            let h = if held.len() >= max_pages {
                Held::failed(format!(
                    "not fetched: a check fetches at most {max_pages} URLs"
                ))
            } else if total >= max_total {
                Held::failed(format!(
                    "not fetched: a check reads at most {max_total} bytes in all"
                ))
            } else if let Some(why) = control.stopped() {
                Held::failed(format!("not fetched: {why}"))
            } else {
                fetch(w, max_doc.min(max_total - total), &control)
            };
            held.insert(w.url.clone(), h);
        }
        last = Some(res);
    }
    Fetched {
        check: last.expect("at least one round"),
        rounds_exhausted: true,
        rounds: max_rounds,
        cancelled: control.cancelled(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::Ordering;

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
    ) -> impl FnMut(&WantedUrl, usize, &Control) -> Held + 'a {
        let http = Http::new(true, Duration::from_secs(10));
        move |w, limit, control| {
            calls.set(calls.get() + 1);
            let url = w.url.replace("https://harbourline.example", &srv.url());
            fetch_held_with(&http, true, &WantedUrl { url, ..w.clone() }, limit, control)
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
        let local = crate::DiscoverOptions {
            allow_local_http: true,
            ..Default::default()
        };
        let r = crate::discover(&srv.addr, now(), &local).unwrap().result;
        assert_eq!(r.index.publisher.name, "Harbourline");
        assert_eq!(r.documents.len(), 1);
        assert_eq!(r.documents[0].status.to_string(), "trusted");
        assert_eq!(r.documents[0].cache_control, "max-age=60");
        assert!(r.documents[0].raw.is_empty(), "raw is not kept");
        let e = crate::discover(&srv.addr, now(), &Default::default()).unwrap_err();
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
            &mut |w, limit, control| {
                calls.set(calls.get() + 1);
                let url = w.url.replace("https://harbourline.example", &srv.url());
                fetch_held_with(&http, true, &WantedUrl { url, ..w.clone() }, limit, control)
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

    /// A server that answers with a body of a hundred megabytes and sends a
    /// byte every `ms` milliseconds: as slow as a publisher can be without
    /// ever stopping.
    fn drip(ms: u64) -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                std::thread::spawn(move || {
                    let mut buf = [0u8; 1024];
                    let _ = s.read(&mut buf);
                    let head =
                        "HTTP/1.1 200 X\r\nContent-Length: 100000000\r\nConnection: close\r\n\r\n";
                    if s.write_all(head.as_bytes()).is_err() {
                        return;
                    }
                    while s.write_all(b" ").and_then(|_| s.flush()).is_ok() {
                        std::thread::sleep(Duration::from_millis(ms));
                    }
                });
            }
        });
        port
    }

    fn local_req(url: String) -> Request {
        Request {
            url,
            ..Request::default()
        }
    }

    #[test]
    fn the_total_deadline_stops_a_body_that_never_ends() {
        let port = drip(20);
        let http = Http::new(true, Duration::from_secs(60));
        let control = Control::new(Instant::now().checked_add(Duration::from_millis(400)), None);
        let t = Instant::now();
        let e = http
            .get_controlled(
                &local_req(format!("http://127.0.0.1:{port}/")),
                usize::MAX,
                &control,
            )
            .unwrap_err();
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "took {:?}",
            t.elapsed()
        );
        assert!(
            t.elapsed() >= Duration::from_millis(300),
            "stopped early: {e}"
        );
        assert!(!control.cancelled());
        // What was read before it stopped is counted.
        assert!(control.bytes_read() > 0, "{e}");
    }

    #[test]
    fn cancelling_stops_a_body_that_never_ends() {
        let port = drip(20);
        let http = Http::new(true, Duration::from_secs(60));
        let flag = Arc::new(AtomicBool::new(false));
        let control = Control::new(None, Some(flag.clone()));
        let f2 = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            f2.store(true, Ordering::Relaxed);
        });
        let t = Instant::now();
        let e = http
            .get_controlled(
                &local_req(format!("http://127.0.0.1:{port}/")),
                usize::MAX,
                &control,
            )
            .unwrap_err();
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "took {:?}",
            t.elapsed()
        );
        assert!(e.to_string().contains("cancelled"), "{e}");
        assert!(control.cancelled());
    }

    #[test]
    fn the_checks_total_time_limit_ends_the_whole_loop() {
        let port = drip(20);
        let http = Http::new(true, Duration::from_secs(60));
        let opts = FetchOptions {
            total_timeout: Duration::from_millis(500),
            ..FetchOptions::default()
        };
        let t = Instant::now();
        let f = run(
            "harbourline.example",
            now(),
            &opts,
            &mut |w, limit, control| {
                let url = w.url.replace(
                    "https://harbourline.example",
                    &format!("http://127.0.0.1:{port}"),
                );
                fetch_held_with(&http, true, &WantedUrl { url, ..w.clone() }, limit, control)
            },
        );
        assert!(
            t.elapsed() < Duration::from_secs(4),
            "took {:?}",
            t.elapsed()
        );
        assert!(!f.cancelled);
        assert!(f.check.error.contains("time limit"), "{}", f.check.error);
    }

    #[test]
    fn cancelling_ends_the_whole_loop() {
        let port = drip(20);
        let http = Http::new(true, Duration::from_secs(60));
        let flag = Arc::new(AtomicBool::new(false));
        let opts = FetchOptions {
            cancel: Some(flag.clone()),
            ..FetchOptions::default()
        };
        let f2 = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            f2.store(true, Ordering::Relaxed);
        });
        let t = Instant::now();
        let f = run(
            "harbourline.example",
            now(),
            &opts,
            &mut |w, limit, control| {
                let url = w.url.replace(
                    "https://harbourline.example",
                    &format!("http://127.0.0.1:{port}"),
                );
                fetch_held_with(&http, true, &WantedUrl { url, ..w.clone() }, limit, control)
            },
        );
        assert!(
            t.elapsed() < Duration::from_secs(4),
            "took {:?}",
            t.elapsed()
        );
        assert!(f.cancelled);
    }

    #[test]
    fn a_page_past_its_limit_still_counts_towards_the_total() {
        let srv = Server::start(|r, _| match r.path.as_str() {
            "/big" => Resp::ok("x".repeat(10_000)),
            // Gzip, and a long run of empty stored blocks after its header:
            // nothing ever decodes, and it never ends.
            "/blocks" => {
                let mut b = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
                for _ in 0..20_000 {
                    b.extend_from_slice(&[0, 0, 0, 0xff, 0xff]);
                }
                Resp::ok(b).header("Content-Encoding", "gzip")
            }
            "/bomb" => Resp::ok(gzip(&vec![b'x'; 1 << 20])).header("Content-Encoding", "gzip"),
            _ => Resp::status(404),
        });
        let http = Http::new(true, Duration::from_secs(10));
        let read = |path: &str| {
            let control = Control::default();
            let e = http
                .get_controlled(&local_req(format!("{}{path}", srv.url())), 1000, &control)
                .unwrap_err();
            (e.to_string(), control.bytes_read())
        };
        let (e, n) = read("/big");
        assert!(e.contains("larger than the 1000-byte limit"), "{e}");
        assert!(n >= 1001, "{n}");
        let (e, n) = read("/blocks");
        assert!(e.contains("larger than the 1000-byte limit"), "{e}");
        // Capped near the limit plus a little, not the hundred kilobytes sent.
        assert!((4000..20_000).contains(&n), "{n}");
        let (e, n) = read("/bomb");
        assert!(e.contains("larger than"), "{e}");
        // Compressed and decompressed both: past the 1001 bytes decoded.
        assert!(n > 1001, "{n}");
    }

    #[test]
    fn a_big_page_is_charged_and_the_next_is_not_fetched() {
        let srv = publisher();
        let calls = Cell::new(0);
        // The index alone is over the budget once read.
        let opts = FetchOptions {
            max_total_bytes: 100,
            ..FetchOptions::default()
        };
        let f = run(
            "harbourline.example",
            now(),
            &opts,
            &mut local(&srv, &calls),
        );
        assert!(f.check.error.contains("larger than"), "{}", f.check.error);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn huge_limits_are_clamped_not_wrapped() {
        let srv = publisher();
        let calls = Cell::new(0);
        let opts = FetchOptions {
            max_document_bytes: usize::MAX,
            max_total_bytes: usize::MAX,
            max_rounds: usize::MAX,
            max_pages: usize::MAX,
            timeout: Duration::MAX,
            total_timeout: Duration::MAX,
            cancel: None,
        };
        let f = run(
            "harbourline.example",
            now(),
            &opts,
            &mut local(&srv, &calls),
        );
        assert_eq!(f.check.errors, 0, "{:?}", f.check.advice);
        assert_eq!(f.rounds, 10);
        // And a limit of the largest number is no overflow in the client.
        let http = Http::new(true, Duration::from_secs(10));
        let r = http
            .get(
                &local_req(format!("{}/.well-known/opencrs.json", srv.url())),
                usize::MAX,
            )
            .unwrap();
        assert_eq!(r.status, 200);
    }

    #[test]
    fn gzip_members_are_all_read_and_trailing_garbage_is_an_error() {
        let srv = Server::start(|r, _| {
            let two = [gzip(b"{\"a\":"), gzip(b"1}")].concat();
            match r.path.as_str() {
                "/two" => Resp::ok(two).header("Content-Encoding", "gzip"),
                "/garbage" => {
                    Resp::ok([gzip(b"{}"), b"garbage-after-the-member".to_vec()].concat())
                        .header("Content-Encoding", "gzip")
                }
                "/one" => Resp::ok(gzip(b"{}")).header("Content-Encoding", "gzip"),
                _ => Resp::status(404),
            }
        });
        let http = Http::new(true, Duration::from_secs(10));
        let get = |p: &str| http.get(&local_req(format!("{}{p}", srv.url())), 1 << 20);
        assert_eq!(get("/one").unwrap().body, b"{}");
        assert_eq!(get("/two").unwrap().body, b"{\"a\":1}");
        let e = get("/garbage").unwrap_err();
        assert!(e.to_string().starts_with("Get \""), "{e}");
    }

    #[test]
    fn a_redirect_with_a_user_name_is_refused() {
        let srv = Server::start(|r, _| match r.path.as_str() {
            "/user" => Resp::redirect(&format!("http://user:pw@127.0.0.1:{}/ok", 1)),
            "/ok" => Resp::ok("{}"),
            _ => Resp::status(404),
        });
        let http = Http::new(true, Duration::from_secs(5));
        let e = http
            .get(&local_req(format!("{}/user", srv.url())), 0)
            .unwrap_err();
        assert!(e.to_string().contains("user name or password"), "{e}");
    }

    #[test]
    fn local_testing_reaches_localhost_only_from_localhost() {
        let u = |s: &str| Url::parse(s).unwrap();
        for (url, allow, want) in [
            ("http://localhost:8080/x", true, true),
            ("http://127.0.0.1:1/x", true, true),
            ("https://[::1]:5/x", true, true),
            ("http://localhost:8080/x", false, false),
            ("https://harbourline.example/x", true, false),
            ("https://localhost.example:8080/x", true, false),
            ("https://evil.example:80/x", true, false),
        ] {
            assert_eq!(
                crate::http::chain_may_be_local(&u(url), allow),
                want,
                "{url}"
            );
        }
    }
}
