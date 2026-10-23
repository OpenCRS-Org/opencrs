//! What discovery needs from the network, without the network: the request
//! and response shapes, the `Fetcher` trait callers implement, and the URL
//! rules that decide what may be fetched.

use std::fmt;

use url::Url;

/// The spec's limit on a single document.
pub const MAX_DOCUMENT_BYTES: usize = 50 << 20;

/// A fetched document and the caching headers that came with it.
#[derive(Debug, Clone, Default)]
pub struct Response {
    pub body: Vec<u8>,
    pub etag: String,
    pub cache_control: String,
    pub last_modified: String,
}

/// What to fetch and how carefully.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub url: String,
    /// If set, sent as a bearer token. Only partner documents get one, and
    /// only from the publisher that issued it.
    pub token: Option<String>,
    /// Refuse redirects to another host. Indexes and anything carrying a
    /// token use it, so neither can be handed elsewhere.
    pub same_host: bool,
}

/// Why a fetch failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// A non-200 response.
    Status { url: String, code: u16 },
    /// Anything else, as text.
    Other(String),
}

impl FetchError {
    /// Whether the server refused the caller's credentials.
    pub fn unauthorised(&self) -> bool {
        matches!(
            self,
            FetchError::Status {
                code: 401 | 403,
                ..
            }
        )
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Status { url, code } => write!(f, "{url}: HTTP {code}"),
            FetchError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for FetchError {}

/// Retrieves documents. The CLI supplies the real one; tests use fakes.
pub trait Fetcher {
    fn fetch(&self, req: &Request) -> Result<Response, FetchError>;
}

/// `localhost`, `127.0.0.1` or `[::1]`, with an optional `:port`.
pub(crate) fn is_local_host(s: &str) -> bool {
    for h in ["localhost", "127.0.0.1", "[::1]"] {
        if let Some(rest) = s.strip_prefix(h) {
            return match rest.strip_prefix(':') {
                None => rest.is_empty(),
                Some(port) => !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()),
            };
        }
    }
    false
}

/// The host as Go's `url.URL.Host` has it: with the port when the URL
/// gives one.
fn host_with_port(u: &Url) -> String {
    let host = u.host_str().unwrap_or("");
    match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    }
}

/// Whether a URL may be fetched: HTTPS, or HTTP to localhost when
/// `allow_local` is set.
pub fn scheme_allowed(u: &Url, allow_local: bool) -> bool {
    u.scheme() == "https"
        || (allow_local && u.scheme() == "http" && is_local_host(&host_with_port(u)))
}

/// Splits a URL's text into (scheme, host, path, query, fragment) without
/// normalising anything, as Go's `url.URL` fields hold them.
fn raw_parts(s: &str) -> Option<(String, String, String, String, String)> {
    let (scheme, rest) = s.split_once("://")?;
    let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = match rest.find('/') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some((
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        path.to_string(),
        query.to_string(),
        fragment.to_string(),
    ))
}

/// Compares two URLs exactly, except that scheme and host are
/// case-insensitive. A trailing slash or a different query is a different
/// URL.
///
/// The text must match part for part, and the URLs as parsed (and so as
/// fetched) must match too, so text that the parser reads differently
/// from a plain split can't pass for another URL. Text with a `\`, which
/// the parser treats as `/`, never matches.
pub fn same_url(a: &str, b: &str) -> bool {
    if a.contains('\\') || b.contains('\\') {
        return false;
    }
    let (Ok(pa), Ok(pb)) = (Url::parse(a), Url::parse(b)) else {
        return false;
    };
    let parsed = |u: &Url| {
        (
            u.scheme().to_string(),
            u.username().to_string(),
            u.password().map(str::to_string),
            u.host_str().map(str::to_string),
            u.port_or_known_default(),
            u.path().to_string(),
            u.query().map(str::to_string),
            u.fragment().map(str::to_string),
        )
    };
    if parsed(&pa) != parsed(&pb) {
        return false;
    }
    match (raw_parts(a), raw_parts(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hosts() {
        for ok in [
            "localhost",
            "localhost:8080",
            "127.0.0.1:1",
            "[::1]",
            "[::1]:99",
        ] {
            assert!(is_local_host(ok), "{ok}");
        }
        for bad in [
            "localhost:",
            "localhost:x",
            "localhost.evil.example",
            "evil.example",
            "[::2]",
        ] {
            assert!(!is_local_host(bad), "{bad}");
        }
    }

    #[test]
    fn schemes() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(scheme_allowed(&u("https://a.example/x"), false));
        assert!(!scheme_allowed(&u("http://localhost:8080/x"), false));
        assert!(scheme_allowed(&u("http://localhost:8080/x"), true));
        assert!(!scheme_allowed(&u("http://a.example/x"), true));
    }

    #[test]
    fn same_url_rules() {
        assert!(same_url(
            "https://A.example/x?y=1",
            "HTTPS://a.EXAMPLE/x?y=1"
        ));
        assert!(!same_url("https://a.example/x", "https://a.example/x/"));
        assert!(!same_url("https://a.example/x", "https://a.example/x?v=2"));
        assert!(!same_url("https://a.example/x", "https://a.example/X"));
        assert!(!same_url("https://a.example", "https://a.example/"));
        assert!(!same_url("::bad", "::bad"));
    }

    /// The review's M1: a plain split took the host after the `@`, the
    /// parser (and so the fetch) the one before the `\`.
    #[test]
    fn same_url_agrees_with_the_parser() {
        assert!(!same_url(
            "https://evil.example\\@real.example/doc.json",
            "https://real.example/doc.json"
        ));
        assert!(!same_url(
            "https://real.example/doc.json",
            "https://evil.example\\@real.example/doc.json"
        ));
        assert!(!same_url(
            "https://real.example\\doc.json",
            "https://real.example\\doc.json"
        ));
        assert!(!same_url(
            "https://user@real.example/doc.json",
            "https://real.example/doc.json"
        ));
        assert!(same_url(
            "https://real.example:443/doc.json",
            "https://real.example:443/doc.json"
        ));
    }

    #[test]
    fn status_errors() {
        let e = FetchError::Status {
            url: "u".into(),
            code: 403,
        };
        assert!(e.unauthorised());
        assert_eq!(e.to_string(), "u: HTTP 403");
        assert!(
            !FetchError::Status {
                url: "u".into(),
                code: 404
            }
            .unauthorised()
        );
        assert!(!FetchError::Other("x".into()).unauthorised());
    }
}
