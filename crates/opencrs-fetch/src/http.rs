//! The real `Fetcher`: ureq over HTTPS, with the spec's size limit and
//! redirects followed by hand so unsafe ones can be refused. Port of
//! `HTTPFetcher` in `discover/fetch.go`. [`Http`] is the one client under
//! both [`UreqFetcher`] (discovery) and the publisher check's page fetching.
//!
//! Every host, redirects included, is resolved once and refused if any of
//! its addresses is loopback, private, link-local, shared (CGNAT),
//! multicast, broadcast, unspecified, reserved or for documentation, so an
//! index can't point the fetcher at a service on the machine or its
//! network. The connection goes to the addresses that were checked. In
//! local testing mode (`-allow-local-http`), `localhost`, `127.0.0.1` and
//! `[::1]` may be reached. Proxy settings from the environment are not
//! used, since a proxy would resolve hosts where they can't be checked.

use opencrs::discover::{
    FetchError, Fetcher, MAX_DOCUMENT_BYTES, Request, Response, scheme_allowed,
};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};
use ureq::config::Config;
use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use url::Url;

/// Why an IPv4 address may not be fetched from, if it may not.
fn refused_v4(ip: Ipv4Addr) -> Option<&'static str> {
    let [a, b, c, _] = ip.octets();
    Some(match (a, b, c) {
        (0, ..) => "an unspecified address",
        (127, ..) => "a loopback address",
        (10, ..) | (172, 16..=31, _) | (192, 168, _) => "a private address",
        (100, 64..=127, _) => "a shared (CGNAT) address",
        (169, 254, _) => "a link-local address",
        (192, 0, 0) => "an IETF protocol address",
        (192, 0, 2) | (198, 51, 100) | (203, 0, 113) => "a documentation address",
        (198, 18..=19, _) => "a benchmarking address",
        (224..=239, ..) => "a multicast address",
        (240..=255, ..) => "a reserved or broadcast address",
        _ => return None,
    })
}

/// Why an address may not be fetched from, if it may not. IPv6 forms that
/// carry an IPv4 address (mapped, compatible, NAT64, 6to4) are judged by
/// it.
pub(crate) fn refused(ip: IpAddr) -> Option<&'static str> {
    let v6 = match ip {
        IpAddr::V4(v4) => return refused_v4(v4),
        IpAddr::V6(v6) => v6,
    };
    if v6 == Ipv6Addr::UNSPECIFIED {
        return Some("an unspecified address");
    }
    if v6 == Ipv6Addr::LOCALHOST {
        return Some("a loopback address");
    }
    let s = v6.segments();
    let embedded = |hi: u16, lo: u16| Ipv4Addr::from(((hi as u32) << 16) | lo as u32);
    // ::ffff:a.b.c.d (mapped) and ::a.b.c.d (compatible).
    if s[..5] == [0; 5] && (s[5] == 0xffff || s[5] == 0) {
        return refused_v4(embedded(s[6], s[7]));
    }
    // 64:ff9b::/96, the well-known NAT64 prefix.
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return refused_v4(embedded(s[6], s[7]));
    }
    // 2002::/16, 6to4.
    if s[0] == 0x2002 {
        return refused_v4(embedded(s[1], s[2]));
    }
    Some(match s[0] {
        0xfc00..=0xfdff => "a private (unique local) address",
        0xfe80..=0xfebf => "a link-local address",
        0xfec0..=0xfeff => "a site-local address",
        0xff00..=0xffff => "a multicast address",
        0x2001 if s[1] == 0x0db8 => "a documentation address",
        0x2001 if s[1] == 0 => "a Teredo address",
        0x0064 if s[1] == 0xff9b => "a local NAT64 address",
        0x0100 if s[1..4] == [0, 0, 0] => "a discard address",
        _ => return None,
    })
}

/// Whether `host` (as a URL has it) names this machine.
fn is_local_name(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "[::1]" | "::1"
    )
}

/// Whether `ip` may be connected to for `host`.
pub(crate) fn check_address(host: &str, ip: IpAddr, allow_local: bool) -> Result<(), String> {
    let Some(why) = refused(ip) else {
        return Ok(());
    };
    let loopback = match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|m| m.is_loopback()),
    };
    if loopback && allow_local && is_local_name(host) {
        return Ok(());
    }
    Err(format!("refusing to connect to {host} ({ip}): it is {why}"))
}

/// The system resolver, with every answer checked by [`check_address`].
/// ureq connects only to the addresses this returns, so nothing is
/// resolved twice.
#[derive(Debug)]
struct GuardedResolver {
    allow_local: bool,
}

impl Resolver for GuardedResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addrs = DefaultResolver::default().resolve(uri, config, timeout)?;
        let host = uri.host().unwrap_or("");
        for a in addrs.iter() {
            check_address(host, a.ip(), self.allow_local).map_err(|why| {
                ureq::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    why,
                ))
            })?;
        }
        Ok(addrs)
    }
}

/// What came back from one request, after redirects. The body is read, with
/// gzip undone, only for a 200; the other headers are as served.
#[derive(Debug, Clone, Default)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub etag: String,
    pub cache_control: String,
    pub last_modified: String,
    pub content_type: String,
    pub content_encoding: String,
    pub allow_origin: String,
}

/// The guarded HTTP client: HTTPS only (HTTP to localhost when allowed),
/// every address checked after DNS and on every redirect, no proxy.
pub struct Http {
    agent: ureq::Agent,
    allow_local: bool,
    timeout: Duration,
}

impl Http {
    /// `timeout` covers one request with its redirects; zero means 30
    /// seconds.
    pub fn new(allow_local: bool, timeout: Duration) -> Http {
        // The system trust store, as Go's client uses.
        Http::with_roots(allow_local, timeout, RootCerts::PlatformVerifier)
    }

    /// [`Http::new`] trusting other roots, for tests with a certificate of
    /// their own.
    pub(crate) fn with_roots(allow_local: bool, timeout: Duration, roots: RootCerts) -> Http {
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .user_agent("opencrs-validator/0.1")
            .tls_config(TlsConfig::builder().root_certs(roots).build())
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            DefaultConnector::default(),
            GuardedResolver { allow_local },
        );
        let timeout = if timeout.is_zero() {
            Duration::from_secs(30)
        } else {
            timeout
        };
        Http {
            agent,
            allow_local,
            timeout,
        }
    }

    /// Fetches `req.url`, following redirects by hand. A body of more than
    /// `limit` bytes (once decompressed) is an error.
    pub fn get(&self, req: &Request, limit: usize) -> Result<Reply, FetchError> {
        let start = Url::parse(&req.url).map_err(|e| {
            FetchError::Other(format!("parse {}: {e}", opencrs::gofmt::quote(&req.url)))
        })?;
        if !scheme_allowed(&start, self.allow_local) {
            return Err(FetchError::Other(format!(
                "{}: only https:// URLs can be fetched",
                req.url
            )));
        }
        // One deadline for the whole request, redirects included, like
        // Go's http.Client.Timeout.
        let deadline = Instant::now() + self.timeout;
        let wrap = |url: &Url, why: String| FetchError::Other(format!("Get \"{url}\": {why}"));

        let mut via = vec![start.clone()];
        let mut current = start;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(wrap(
                    &current,
                    "context deadline exceeded (Client.Timeout exceeded while awaiting headers)"
                        .into(),
                ));
            }
            // Accept-Encoding is set here, not left to ureq: with its own
            // gzip support on, ureq would decode the body and drop the
            // Content-Encoding the publisher check reports on.
            let mut r = self
                .agent
                .get(current.as_str())
                .config()
                .timeout_global(Some(remaining))
                .build()
                .header("Accept", "application/json")
                .header("Accept-Encoding", "gzip");
            if let Some(t) = &req.token {
                r = r.header("Authorization", format!("Bearer {t}"));
            }
            let mut resp = r.call().map_err(|e| wrap(&current, e.to_string()))?;
            let code = resp.status().as_u16();
            if matches!(code, 301 | 302 | 303 | 307 | 308)
                && let Some(loc) = resp.headers().get("location").and_then(|v| v.to_str().ok())
            {
                let next = current
                    .join(loc)
                    .map_err(|e| wrap(&current, format!("invalid redirect location: {e}")))?;
                // Go names the Location as given, not the resolved URL.
                check_redirect(&next, &via, req, self.allow_local)
                    .map_err(|why| FetchError::Other(format!("Get \"{loc}\": {why}")))?;
                via.push(next.clone());
                current = next;
                continue;
            }
            let header = |name: &str| {
                resp.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string()
            };
            let mut reply = Reply {
                status: code,
                etag: header("etag"),
                cache_control: header("cache-control"),
                last_modified: header("last-modified"),
                content_type: header("content-type"),
                content_encoding: header("content-encoding"),
                allow_origin: header("access-control-allow-origin"),
                body: Vec::new(),
            };
            if code != 200 {
                return Ok(reply);
            }
            let raw = resp.body_mut().with_config().limit(u64::MAX).reader();
            let body: Box<dyn Read> = if reply.content_encoding.eq_ignore_ascii_case("gzip") {
                Box::new(flate2::read::GzDecoder::new(raw))
            } else {
                Box::new(raw)
            };
            // The limit is on what a check would hold, so on the
            // decompressed body.
            body.take(limit as u64 + 1)
                .read_to_end(&mut reply.body)
                .map_err(|e| wrap(&current, e.to_string()))?;
            if reply.body.len() > limit {
                return Err(FetchError::Other(format!(
                    "{}: larger than the {limit}-byte limit",
                    req.url
                )));
            }
            return Ok(reply);
        }
    }
}

pub struct UreqFetcher {
    /// Permit plain http:// to localhost, for publishers testing before
    /// they deploy.
    pub allow_local_http: bool,
    /// Zero means [`MAX_DOCUMENT_BYTES`].
    pub max_bytes: usize,
    /// Zero means 30 seconds.
    pub timeout: Duration,
}

impl UreqFetcher {
    pub fn new(allow_local_http: bool) -> Self {
        UreqFetcher {
            allow_local_http,
            max_bytes: 0,
            timeout: Duration::ZERO,
        }
    }
}

fn host_with_port(u: &Url) -> String {
    let host = u.host_str().unwrap_or("");
    match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    }
}

/// Go's `checkRedirect`: `via` holds the URLs requested so far.
fn check_redirect(next: &Url, via: &[Url], req: &Request, allow_local: bool) -> Result<(), String> {
    if via.len() >= 5 {
        return Err("too many redirects".into());
    }
    let prev = &via[via.len() - 1];
    if !scheme_allowed(next, allow_local) || (prev.scheme() == "https" && next.scheme() != "https")
    {
        return Err(format!(
            "refusing redirect from {prev} to {next}: not HTTPS"
        ));
    }
    let first = host_with_port(&via[0]);
    let to = host_with_port(next);
    if !first.eq_ignore_ascii_case(&to) && (req.same_host || req.token.is_some()) {
        return Err(format!(
            "refusing redirect from {first} to another host, {to}"
        ));
    }
    Ok(())
}

impl Fetcher for UreqFetcher {
    fn fetch(&self, req: &Request) -> Result<Response, FetchError> {
        let limit = if self.max_bytes == 0 {
            MAX_DOCUMENT_BYTES
        } else {
            self.max_bytes
        };
        let r = Http::new(self.allow_local_http, self.timeout).get(req, limit)?;
        if r.status != 200 {
            return Err(FetchError::Status {
                url: req.url.clone(),
                code: r.status,
            });
        }
        Ok(Response {
            body: r.body,
            etag: r.etag,
            cache_control: r.cache_control,
            last_modified: r.last_modified,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn redirects_must_stay_on_https() {
        let r = Request::default();
        let via = [u("https://a.example/x")];
        let e = check_redirect(&u("http://localhost:1/x"), &via, &r, true).unwrap_err();
        assert!(e.contains("not HTTPS"), "{e}");
        let e = check_redirect(&u("http://b.example/x"), &via, &r, true).unwrap_err();
        assert!(e.contains("not HTTPS"), "{e}");
        assert!(check_redirect(&u("https://b.example/x"), &via, &r, false).is_ok());
    }

    #[test]
    fn same_host_and_token_forbid_other_hosts() {
        let via = [u("https://a.example/x")];
        for r in [
            Request {
                same_host: true,
                ..Request::default()
            },
            Request {
                token: Some("t".into()),
                ..Request::default()
            },
        ] {
            let e = check_redirect(&u("https://b.example/x"), &via, &r, false).unwrap_err();
            assert!(e.contains("another host"), "{e}");
            assert!(check_redirect(&u("https://A.example/y"), &via, &r, false).is_ok());
        }
    }

    /// The review's M3: nothing stopped an index sending the fetcher to the
    /// machine itself, its network or a cloud metadata service.
    #[test]
    fn internal_addresses_are_refused() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        for bad in [
            "0.0.0.0",
            "0.1.2.3",
            "127.0.0.1",
            "127.9.9.9",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "100.64.0.1",
            "100.127.255.255",
            "169.254.169.254",
            "192.0.0.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "198.18.0.1",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
            "::0.0.0.1",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::1",
            "2002:a9fe:a9fe::1",
            "2002:7f00:1::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "2001:0:4136:e378::1",
            "100::1",
        ] {
            assert!(refused(ip(bad)).is_some(), "{bad} should be refused");
        }
        for good in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.215.14",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.0.1",
            "172.32.0.1",
            "169.253.0.1",
            "11.0.0.1",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "::ffff:1.1.1.1",
            "64:ff9b::101:101",
            "2002:101:101::1",
        ] {
            assert_eq!(refused(ip(good)), None, "{good} should be allowed");
        }
    }

    #[test]
    fn local_testing_allows_loopback_for_local_names_only() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        for (host, addr) in [
            ("localhost", "127.0.0.1"),
            ("localhost", "::1"),
            ("127.0.0.1", "127.0.0.1"),
            ("[::1]", "::1"),
        ] {
            assert!(check_address(host, ip(addr), true).is_ok(), "{host}");
            assert!(check_address(host, ip(addr), false).is_err(), "{host}");
        }
        // A public name that resolves to loopback, and private addresses,
        // stay refused even in local testing mode.
        assert!(check_address("evil.example", ip("127.0.0.1"), true).is_err());
        assert!(check_address("localhost", ip("10.0.0.1"), true).is_err());
        let e = check_address("meta.example", ip("169.254.169.254"), true).unwrap_err();
        assert_eq!(
            e,
            "refusing to connect to meta.example (169.254.169.254): it is a link-local address"
        );
    }

    #[test]
    fn at_most_four_redirects() {
        let via: Vec<Url> = (0..5).map(|_| u("https://a.example/x")).collect();
        let e = check_redirect(&u("https://a.example/y"), &via, &Request::default(), false);
        assert_eq!(e.unwrap_err(), "too many redirects");
    }
}
