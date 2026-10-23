//! Port of `discover/fetch_test.go`, against local servers over plain HTTP
//! (there is no TLS test server here, so the https-to-http redirect is
//! covered by a unit test in `http.rs`).

#[path = "common/server.rs"]
mod server;
use opencrs::discover::{FetchError, Fetcher, Request};
use opencrs_cli::http::UreqFetcher;
use server::{Resp, Server};

fn req(url: String) -> Request {
    Request {
        url,
        ..Request::default()
    }
}

#[test]
fn http_fetcher() {
    let other = Server::start(|_, _| Resp::ok("{}"));
    let other_url = other.url();
    let srv = Server::start(move |r, _| match r.path.as_str() {
        "/ok" => Resp::ok(r#"{"ok":true}"#)
            .header("ETag", "\"abc\"")
            .header("Cache-Control", "max-age=60"),
        "/big" => Resp::ok("x".repeat(101)),
        "/to-other" => Resp::redirect(&format!("{other_url}/x")),
        "/loop" => Resp::redirect("/loop"),
        "/forbidden" => Resp::status(403),
        _ => Resp::status(404),
    });
    let mut f = UreqFetcher::new(true);
    f.max_bytes = 100;

    let r = f.fetch(&req(format!("{}/ok", srv.url()))).unwrap();
    assert_eq!(r.body, br#"{"ok":true}"#);
    assert_eq!(r.etag, "\"abc\"");
    assert_eq!(r.cache_control, "max-age=60");

    let e = f.fetch(&req(format!("{}/big", srv.url()))).unwrap_err();
    assert!(e.to_string().contains("limit"), "{e}");

    // A public document may redirect to another host.
    f.fetch(&req(format!("{}/to-other", srv.url()))).unwrap();

    let before = other.seen.lock().unwrap().len();
    let same_host = Request {
        same_host: true,
        ..req(format!("{}/to-other", srv.url()))
    };
    let token = Request {
        token: Some("s3cret".into()),
        ..req(format!("{}/to-other", srv.url()))
    };
    for r in [same_host, token] {
        let e = f.fetch(&r).unwrap_err();
        assert!(e.to_string().contains("another host"), "{e}");
    }
    assert_eq!(
        other.seen.lock().unwrap().len(),
        before,
        "nothing reached the other host"
    );
    assert!(other.seen.lock().unwrap().iter().all(|(_, a)| a.is_empty()));

    let e = f.fetch(&req(format!("{}/loop", srv.url()))).unwrap_err();
    assert!(e.to_string().contains("too many redirects"), "{e}");

    let e = f
        .fetch(&req(format!("{}/forbidden", srv.url())))
        .unwrap_err();
    assert!(matches!(e, FetchError::Status { code: 403, .. }) && e.unauthorised());
    let e = f.fetch(&req(format!("{}/nope", srv.url()))).unwrap_err();
    assert!(!e.unauthorised());

    // Plain HTTP to a non-local host is refused before any connection.
    let e = f.fetch(&req("http://example.com/x".into())).unwrap_err();
    assert!(e.to_string().contains("only https:// URLs"), "{e}");
    // And local HTTP is refused unless allowed.
    let strict = UreqFetcher::new(false);
    assert!(strict.fetch(&req(format!("{}/ok", srv.url()))).is_err());
}

#[test]
fn token_is_sent_as_bearer() {
    let srv = Server::start(|r, _| Resp::ok(r.authorization.clone()));
    let f = UreqFetcher::new(true);
    let r = f
        .fetch(&Request {
            url: format!("{}/", srv.url()),
            token: Some("t0k".into()),
            same_host: false,
        })
        .unwrap();
    assert_eq!(r.body, b"Bearer t0k");
}

#[test]
fn timeout_covers_the_whole_redirect_chain() {
    use std::time::{Duration, Instant};
    let srv = Server::start(|r, _| {
        std::thread::sleep(Duration::from_millis(250));
        match r.path.as_str() {
            "/a" => Resp::redirect("/b"),
            "/b" => Resp::redirect("/c"),
            _ => Resp::ok("{}"),
        }
    });
    let mut f = UreqFetcher::new(true);
    f.timeout = Duration::from_millis(400);
    // Each hop is under the timeout, the chain isn't.
    let t = Instant::now();
    let e = f.fetch(&req(format!("{}/a", srv.url()))).unwrap_err();
    assert!(
        t.elapsed() < Duration::from_millis(1500),
        "took {:?}",
        t.elapsed()
    );
    assert!(matches!(e, FetchError::Other(_)), "{e}");
    // The same chain succeeds with room to spare.
    f.timeout = Duration::from_secs(10);
    f.fetch(&req(format!("{}/a", srv.url()))).unwrap();
}

/// The review's M3: an index could point the fetcher at the machine
/// itself. Without the local testing flag, nothing on loopback is even
/// connected to.
#[test]
fn loopback_is_refused_before_connecting() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut f = UreqFetcher::new(false);
    f.timeout = std::time::Duration::from_secs(2);
    for url in [
        format!("https://127.0.0.1:{port}/internal/"),
        format!("https://localhost:{port}/internal/"),
    ] {
        let e = f.fetch(&req(url.clone())).unwrap_err().to_string();
        assert!(e.contains("refusing to connect"), "{url}: {e}");
    }
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("something connected: {other:?}"),
    }
}

/// Private and link-local addresses stay refused in local testing mode,
/// including through a redirect from an allowed local server.
#[test]
fn internal_addresses_are_refused_even_when_testing_locally() {
    let srv = Server::start(|r, _| match r.path.as_str() {
        "/metadata" => Resp::redirect("https://169.254.169.254/latest/meta-data/"),
        "/private" => Resp::redirect("https://10.0.0.1/"),
        _ => Resp::status(404),
    });
    let mut f = UreqFetcher::new(true);
    f.timeout = std::time::Duration::from_secs(2);
    for path in ["/metadata", "/private"] {
        let e = f
            .fetch(&req(format!("{}{path}", srv.url())))
            .unwrap_err()
            .to_string();
        assert!(e.contains("refusing to connect"), "{path}: {e}");
    }
    let e = f
        .fetch(&req("https://[fd00::1]/x".into()))
        .unwrap_err()
        .to_string();
    assert!(e.contains("refusing to connect"), "{e}");
}
