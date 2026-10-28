//! Finds a publisher's OpenCRS documents through its
//! `/.well-known/opencrs.json` index and decides which of them to trust.
//! Pure: the network sits behind [`Fetcher`] and the clock is [`Options::now`].

mod fetch;
mod merge;

use std::collections::HashMap;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use fetch::{
    FetchError, Fetcher, MAX_DOCUMENT_BYTES, Request, Response, same_url, scheme_allowed,
};

/// The most documents, and the most delegates, one index may list.
pub const MAX_INDEX_ENTRIES: usize = 1000;

/// The most bytes one discovery reads, indexes and documents together:
/// ten documents at the spec's 50 MiB limit, or a hundred the size of a
/// 20,000-screen network.
pub const MAX_RUN_BYTES: usize = 512 << 20;

/// The most URLs one discovery asks for, indexes and documents together,
/// however many an index lists. A publisher chooses every URL a check
/// fetches, so without a limit one index could aim a check at a victim a
/// million times. The same limit applies to the CLI's `fetch`.
pub const MAX_FETCHES_PER_RUN: usize = 2000;

/// The most documents read from one delegate's index. A delegate is
/// another party's domain, so it gets a smaller share than the publisher.
pub const MAX_DOCUMENTS_PER_DELEGATE: usize = 250;
pub use merge::merge;

use crate::error::Error;
use crate::schema::{Kind, SchemaOptions, validate_schema};
use fetch::is_local_host;

/// A publisher's `/.well-known/opencrs.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    pub opencrs: String,
    pub publisher: IndexPublisher,
    pub documents: Vec<IndexDocument>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegates: Vec<Delegate>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub changes_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexPublisher {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub contact: String,
}

/// One document an index vouches for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexDocument {
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub network: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub visibility: String,
}

/// Names another domain that may write documents for this publisher's
/// screens.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Delegate {
    pub domain: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

/// The trust decision for one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Authorised and current: safe to validate against.
    Trusted,
    /// Authorised but past `expires_at`: must not be used.
    Expired,
    /// Not authorised, or not a valid document.
    Rejected,
    /// A partner document with no usable credentials.
    Skipped,
    /// Couldn't be fetched.
    Failed,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Status::Trusted => "trusted",
            Status::Expired => "expired",
            Status::Rejected => "rejected",
            Status::Skipped => "skipped",
            Status::Failed => "failed",
        })
    }
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// One discovered document and the decision about it.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredDocument {
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub network: String,
    /// `public` or `partner`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub visibility: String,
    /// The domain whose index lists the URL.
    pub listed_by: String,
    /// Set for a document trusted through delegation: the publisher that
    /// named `listed_by` as its delegate.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub delegated_by: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub networks: Vec<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub screens: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub updated_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub expires_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub etag: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cache_control: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_modified: String,
    pub status: Status,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// The document exactly as served, so nothing a 0.1 reader doesn't
    /// understand is lost when it's saved.
    #[serde(skip)]
    pub raw: Vec<u8>,
}

/// What following one delegate found.
#[derive(Debug, Clone, Serialize)]
pub struct DelegateResult {
    pub domain: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scope: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub index_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// Everything discovered for one publisher.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryResult {
    pub domain: String,
    pub index_url: String,
    pub index: Index,
    pub documents: Vec<DiscoveredDocument>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub delegates: Vec<DelegateResult>,
    /// Where discovery stopped short of what the indexes list, and why: a
    /// safeguard of this validator, not a fault in the publisher's files.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<String>,
}

impl DiscoveryResult {
    /// The documents that are safe to validate against.
    pub fn trusted(&self) -> Vec<&DiscoveredDocument> {
        self.documents
            .iter()
            .filter(|d| d.status == Status::Trusted)
            .collect()
    }
}

/// Controls discovery.
#[derive(Debug, Clone)]
pub struct Options {
    /// Maps a publisher's domain to the bearer token it issued for its
    /// partner documents. A token is only ever sent to partner documents
    /// listed in that publisher's own index.
    pub tokens: HashMap<String, String>,
    /// Accept `http://localhost` indexes and documents, and an index on
    /// localhost naming any publisher, for local testing. (The CLI's
    /// fetcher also lets this mode connect to localhost, 127.0.0.1 and
    /// `[::1]`, which it otherwise refuses.)
    pub allow_local_http: bool,
    pub now: DateTime<FixedOffset>,
    /// The most documents (and delegates) one index may list; zero means
    /// [`MAX_INDEX_ENTRIES`].
    pub max_index_entries: usize,
    /// The most bytes one discovery reads; zero means [`MAX_RUN_BYTES`].
    /// Documents past it are `failed`.
    pub max_run_bytes: usize,
    /// The most URLs one discovery asks for; zero means
    /// [`MAX_FETCHES_PER_RUN`]. Ones past it are not fetched, and the
    /// result's `limits` says so.
    pub max_fetches: usize,
    /// Keep each trusted document's bytes in the result (`raw`). A caller
    /// that already holds them can leave this off and save a copy.
    pub keep_raw: bool,
}

/// What's left of a discovery's byte and fetch allowances.
struct Budget {
    limit: usize,
    used: std::cell::Cell<usize>,
    max_fetches: usize,
    fetches: std::cell::Cell<usize>,
}

impl Budget {
    fn new(opt: &Options) -> Budget {
        Budget {
            limit: if opt.max_run_bytes == 0 {
                MAX_RUN_BYTES
            } else {
                opt.max_run_bytes
            },
            used: std::cell::Cell::new(0),
            max_fetches: if opt.max_fetches == 0 {
                MAX_FETCHES_PER_RUN
            } else {
                opt.max_fetches
            },
            fetches: std::cell::Cell::new(0),
        }
    }

    /// Counts one more URL to fetch; false if that passes the limit.
    fn fetch_ok(&self) -> bool {
        if self.fetches.get() >= self.max_fetches {
            return false;
        }
        self.fetches.set(self.fetches.get() + 1);
        true
    }

    fn fetch_refusal(&self) -> String {
        format!(
            "not fetched: a check asks for at most {} URLs",
            self.max_fetches
        )
    }

    fn spent(&self) -> bool {
        self.used.get() >= self.limit
    }

    /// Counts `n` more bytes; false if that passes the limit.
    fn take(&self, n: usize) -> bool {
        self.used.set(self.used.get().saturating_add(n));
        self.used.get() <= self.limit
    }

    fn refusal(&self, url: &str) -> String {
        format!(
            "{url}: past the {}-byte limit for everything one fetch reads",
            self.limit
        )
    }
}

/// Whether `s` is a publisher domain as the schema defines one: lower-case
/// labels of letters, digits and hyphens, at least two of them, separated
/// by dots (`^[a-z0-9-]+(\.[a-z0-9-]+)+$`). No port, user or path.
pub fn is_domain(s: &str) -> bool {
    let mut labels = 0;
    for label in s.split('.') {
        if label.is_empty()
            || !label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return false;
        }
        labels += 1;
    }
    labels >= 2
}

/// Whether `s` is a DNS name that can belong to a publisher: a domain by
/// [`is_domain`] whose last label isn't all digits (so `10.0.0.5` is an
/// address, not a name) and which isn't one of the names reserved for the
/// local machine or network.
pub fn is_dns_name(s: &str) -> bool {
    is_domain(s)
        && s.rsplit('.')
            .next()
            .is_some_and(|tld| tld.bytes().any(|b| b.is_ascii_lowercase()))
        && !reserved_name(s)
}

fn reserved_name(s: &str) -> bool {
    let s = s.trim_end_matches('.');
    s == "localhost"
        || [".localhost", ".local", ".internal", ".localdomain"]
            .iter()
            .any(|suffix| s.ends_with(suffix))
}

/// A domain, or in local testing mode a local host and port.
fn may_publish(s: &str, opt: &Options) -> bool {
    is_dns_name(s) || (opt.allow_local_http && is_local_host(s))
}

/// Why a listed document's URL must not be fetched, if it mustn't. A
/// publisher chooses these URLs, so one that points a fetcher at an address
/// or a local-only name, or carries a user name and password, is rejected
/// without a request. (Where a name resolves is the fetcher's to check.)
fn unfetchable(url: &str, opt: &Options) -> Option<String> {
    let Ok(u) = url::Url::parse(url) else {
        return Some("not a valid URL".to_string());
    };
    if !u.username().is_empty() || u.password().is_some() {
        return Some("the URL has a user name or password in it".to_string());
    }
    let host = u.host_str().unwrap_or("");
    let at = match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    if opt.allow_local_http && is_local_host(&at) {
        return None;
    }
    match u.host() {
        Some(url::Host::Domain(d)) if !reserved_name(d) => None,
        Some(url::Host::Domain(_)) => {
            Some("the URL is at a name that only the local machine or network knows".to_string())
        }
        _ => Some(
            "the URL is at an IP address; documents must be at a host name, as the publisher's domain is what vouches for them"
                .to_string(),
        ),
    }
}

/// Where a domain's index lives.
pub fn index_url(domain: &str, allow_local: bool) -> String {
    let scheme = if allow_local && is_local_host(domain) {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{domain}/.well-known/opencrs.json")
}

/// Fetches a publisher's index and every document it lists, then follows
/// its delegates one hop. Each document gets a trust decision:
///
/// - A document listed in P's own index is trusted if it declares P as its
///   publisher, wherever it's hosted.
/// - A document listed in P's index that declares another publisher is
///   rejected: an index only vouches for its own publisher's documents.
/// - If P's index names M as a delegate, documents listed in M's index that
///   declare P as publisher are trusted, provided every network in them
///   falls inside the delegated scope. Both sides vouch: P names M, and M
///   lists the URL.
///
/// An error means the index itself couldn't be fetched or trusted.
pub fn discover(
    domain: &str,
    opt: &Options,
    fetcher: &dyn Fetcher,
) -> Result<DiscoveryResult, Error> {
    let domain = domain.to_lowercase();
    if !may_publish(&domain, opt) {
        return Err(Error::msg(format!(
            "{} isn't a publisher domain (a host name such as harbourline.example)",
            crate::gofmt::quote(&domain)
        )));
    }
    let budget = Budget::new(opt);
    let (idx, url) = fetch_index(&domain, opt, fetcher, &budget)?;
    let publisher = idx.publisher.id.clone();
    let token = |d: &str| opt.tokens.get(d).map(String::as_str).unwrap_or("");
    let mut res = DiscoveryResult {
        domain: domain.clone(),
        index_url: url,
        index: idx.clone(),
        documents: Vec::new(),
        delegates: Vec::new(),
        limits: Vec::new(),
    };

    for (i, e) in idx.documents.iter().enumerate() {
        // The host may be asking this call to stop.
        if crate::budget::checkpoint() {
            return Ok(res);
        }
        if !budget.fetch_ok() {
            res.limits.push(format!(
                "{domain}'s index lists {} documents; only the first {i} were fetched, as a check asks for at most {} URLs. That is a limit of this validator, not a fault in the index.",
                idx.documents.len(),
                budget.max_fetches
            ));
            break;
        }
        let (mut d, undecided) = fetch_document(e, &domain, token(&domain), fetcher, &budget, opt);
        if undecided {
            decide(&mut d, &publisher, None, opt.now);
        }
        keep_raw_if_trusted(&mut d, opt);
        res.documents.push(d);
    }

    for del in &idx.delegates {
        if crate::budget::checkpoint() {
            return Ok(res);
        }
        let mut dr = DelegateResult {
            domain: del.domain.to_lowercase(),
            scope: del.scope.clone(),
            index_url: String::new(),
            error: String::new(),
        };
        let Some(networks) = parse_scope(&del.scope) else {
            dr.error = format!(
                "unrecognised scope {}, which grants nothing",
                crate::gofmt::quote(&del.scope)
            );
            res.delegates.push(dr);
            continue;
        };
        if dr.domain == domain {
            res.delegates.push(dr);
            continue;
        }
        let fetched = fetch_index(&dr.domain, opt, fetcher, &budget);
        let (didx, durl) = match fetched {
            Ok(x) => x,
            Err(e) => {
                dr.index_url = index_url(&dr.domain, opt.allow_local_http);
                dr.error = e.to_string();
                res.delegates.push(dr);
                continue;
            }
        };
        dr.index_url = durl;
        let dr_domain = dr.domain.clone();
        res.delegates.push(dr);
        // One hop only: the delegate's own delegates aren't followed.
        if didx.documents.len() > MAX_DOCUMENTS_PER_DELEGATE {
            res.limits.push(format!(
                "{dr_domain}'s index lists {} documents; only the first {MAX_DOCUMENTS_PER_DELEGATE} were fetched, as a check reads at most that many from one delegate. That is a limit of this validator, not a fault in the index.",
                didx.documents.len()
            ));
        }
        for e in didx.documents.iter().take(MAX_DOCUMENTS_PER_DELEGATE) {
            if crate::budget::checkpoint() {
                return Ok(res);
            }
            if !budget.fetch_ok() {
                res.limits.push(format!(
                    "{dr_domain}'s documents were not all fetched, as a check asks for at most {} URLs. That is a limit of this validator, not a fault in the index.",
                    budget.max_fetches
                ));
                break;
            }
            let (mut d, undecided) =
                fetch_document(e, &dr_domain, token(&dr_domain), fetcher, &budget, opt);
            if d.publisher != publisher {
                continue; // the delegate's own documents, or unreadable ones
            }
            if undecided {
                d.delegated_by = domain.clone();
                decide(&mut d, &publisher, Some(&networks), opt.now);
            }
            keep_raw_if_trusted(&mut d, opt);
            res.documents.push(d);
        }
    }
    Ok(res)
}

/// Fetches the document at `doc_url` and checks whether the publisher it
/// declares vouches for that exact URL, directly or through a delegate.
pub fn verify(
    doc_url: &str,
    opt: &Options,
    fetcher: &dyn Fetcher,
) -> Result<(DiscoveryResult, DiscoveredDocument), Error> {
    let resp = fetcher
        .fetch(&Request {
            url: doc_url.to_string(),
            ..Request::default()
        })
        .map_err(|e| Error::msg(e.to_string()))?;
    // Require a JSON object: serde would read a struct from an array, Go doesn't.
    let publisher = match serde_json::from_slice::<serde_json::Value>(&resp.body) {
        Ok(serde_json::Value::Object(m)) => match m.get("publisher") {
            Some(serde_json::Value::String(p)) => Some(p.clone()),
            _ => None,
        },
        _ => None,
    }
    .filter(|p| !p.is_empty())
    .ok_or_else(|| Error::msg(format!("{doc_url} doesn't declare a publisher")))?;
    // The publisher becomes the index's host: anything but a bare domain
    // could send the index request somewhere else.
    if !may_publish(&publisher, opt) {
        return Err(Error::msg(format!(
            "{doc_url} declares publisher {}, which isn't a domain name",
            crate::gofmt::quote(&publisher)
        )));
    }
    let res = discover(&publisher, opt, fetcher)?;
    if let Some(d) = res.documents.iter().find(|d| same_url(&d.url, doc_url)) {
        let d = d.clone();
        return Ok((res, d));
    }
    let d = DiscoveredDocument {
        reason: format!("not listed by {publisher}'s index or by any delegate it names"),
        ..blank(doc_url, Status::Rejected, &publisher)
    };
    Ok((res, d))
}

fn blank(url: &str, status: Status, publisher: &str) -> DiscoveredDocument {
    DiscoveredDocument {
        url: url.to_string(),
        network: String::new(),
        visibility: String::new(),
        listed_by: String::new(),
        delegated_by: String::new(),
        publisher: publisher.to_string(),
        networks: Vec::new(),
        screens: 0,
        updated_at: String::new(),
        expires_at: String::new(),
        etag: String::new(),
        cache_control: String::new(),
        last_modified: String::new(),
        status,
        reason: String::new(),
        raw: Vec::new(),
    }
}

/// Only a trusted document's bytes are kept (to be saved); the rest are
/// dropped as soon as they're decided.
fn keep_raw_if_trusted(d: &mut DiscoveredDocument, opt: &Options) {
    if d.status != Status::Trusted || !opt.keep_raw {
        d.raw = Vec::new();
    }
}

fn fetch_index(
    domain: &str,
    opt: &Options,
    fetcher: &dyn Fetcher,
    budget: &Budget,
) -> Result<(Index, String), Error> {
    // A delegate's domain comes from someone else's index: it must be a
    // name, never an address, before anything is requested.
    if !may_publish(domain, opt) {
        return Err(Error::msg(format!(
            "{} isn't a publisher domain (a host name such as harbourline.example)",
            crate::gofmt::quote(domain)
        )));
    }
    if !budget.fetch_ok() {
        return Err(Error::msg(format!(
            "fetching the index: {}",
            budget.fetch_refusal()
        )));
    }
    let u = index_url(domain, opt.allow_local_http);
    // `localhost`, `127.0.0.1` or `[::1]` with an optional port, for testing.
    let local = opt.allow_local_http && is_local_host(domain);
    // Otherwise the URL must go to the domain itself, on the default port:
    // the parser reads some names (such as `0x7f.1`) as something else.
    let goes_to_domain = local
        || url::Url::parse(&u).is_ok_and(|p| {
            p.username().is_empty()
                && p.password().is_none()
                && p.port().is_none()
                && p.host_str() == Some(domain)
        });
    if !goes_to_domain {
        return Err(Error::msg(format!(
            "{} isn't a publisher domain (a host name such as harbourline.example)",
            crate::gofmt::quote(domain)
        )));
    }
    let resp = fetcher
        .fetch(&Request {
            url: u.clone(),
            token: None,
            same_host: true,
            index: true,
        })
        .map_err(|e| Error::msg(format!("fetching the index: {e}")))?;
    if !budget.take(resp.body.len()) {
        return Err(Error::msg(format!(
            "fetching the index: {}",
            budget.refusal(&u)
        )));
    }
    let so = SchemaOptions {
        list: false,
        allow_local_http: opt.allow_local_http,
    };
    let errs = validate_schema(Kind::Index, &resp.body, &so)
        .map_err(|e| Error::msg(format!("{u}: {e}")))?;
    if let Some(first) = errs.first() {
        return Err(Error::msg(format!("{u} isn't a valid index: {first}")));
    }
    let idx: Index = serde_json::from_slice(&resp.body)?;
    let max = if opt.max_index_entries == 0 {
        MAX_INDEX_ENTRIES
    } else {
        opt.max_index_entries
    };
    for (n, what) in [
        (idx.documents.len(), "documents"),
        (idx.delegates.len(), "delegates"),
    ] {
        if n > max {
            return Err(Error::msg(format!(
                "the index at {u} lists {n} {what}; at most {max} are read"
            )));
        }
    }
    if !local && !idx.publisher.id.eq_ignore_ascii_case(domain) {
        return Err(Error::msg(format!(
            "the index at {u} names publisher {}; an index can only speak for its own domain",
            idx.publisher.id
        )));
    }
    Ok((idx, u))
}

#[derive(Default, Deserialize)]
struct DocHead {
    publisher: Option<String>,
    updated_at: Option<String>,
    expires_at: Option<String>,
    networks: Option<Vec<NetHead>>,
}

#[derive(Default, Deserialize)]
struct NetHead {
    id: Option<String>,
    screens: Option<Vec<serde::de::IgnoredAny>>,
}

/// Fetches and parses one listed document. The bool is true when the
/// document still needs a trust decision; otherwise its status is already
/// set because it can't be considered at all.
fn fetch_document(
    e: &IndexDocument,
    listed_by: &str,
    token: &str,
    fetcher: &dyn Fetcher,
    budget: &Budget,
    opt: &Options,
) -> (DiscoveredDocument, bool) {
    let mut d = DiscoveredDocument {
        network: e.network.clone(),
        visibility: if e.visibility.is_empty() {
            "public".into()
        } else {
            e.visibility.clone()
        },
        listed_by: listed_by.to_string(),
        ..blank(&e.url, Status::Trusted, "")
    };
    if let Some(why) = unfetchable(&e.url, opt) {
        d.status = Status::Rejected;
        d.reason = why;
        return (d, false);
    }
    let mut req = Request {
        url: e.url.clone(),
        ..Request::default()
    };
    let partner = d.visibility == "partner";
    if partner {
        if token.is_empty() {
            d.status = Status::Skipped;
            d.reason = format!("partner document; needs a token issued by {listed_by}");
            return (d, false);
        }
        req.token = Some(token.to_string());
        req.same_host = true;
    }
    if budget.spent() {
        d.status = Status::Failed;
        d.reason = format!("not fetched: {}", budget.refusal(&e.url));
        return (d, false);
    }
    let resp = match fetcher.fetch(&req) {
        Ok(r) if !budget.take(r.body.len()) => {
            d.status = Status::Failed;
            d.reason = budget.refusal(&e.url);
            return (d, false);
        }
        Ok(r) => r,
        Err(err) => {
            match &err {
                FetchError::Status { code, .. } if err.unauthorised() && partner => {
                    d.status = Status::Skipped;
                    d.reason = format!("{listed_by} refused the credentials (HTTP {code})");
                }
                _ => {
                    d.status = Status::Failed;
                    d.reason = err.to_string();
                }
            }
            return (d, false);
        }
    };
    d.etag = resp.etag;
    d.cache_control = resp.cache_control;
    d.last_modified = resp.last_modified;
    d.raw = resp.body;

    // Read the head straight from the text when it is an object that
    // decodes cleanly, so a large document isn't built as a `Value` first.
    // Anything else (a repeated key, a wrong type, not JSON) takes the
    // longer route below, which words the problem.
    let direct = d
        .raw
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .filter(|b| **b == b'{')
        .and_then(|_| serde_json::from_slice::<DocHead>(&d.raw).ok());
    let head = match direct {
        Some(h) => Ok(h),
        None => match serde_json::from_slice::<Value>(&d.raw) {
            Ok(Value::Null) => Ok(DocHead::default()),
            Ok(v @ Value::Object(_)) => serde_json::from_value::<DocHead>(v),
            Ok(_) => Err(<serde_json::Error as serde::de::Error>::custom(
                "cannot unmarshal into a document",
            )),
            Err(err) => Err(err),
        },
    };
    // A document on another host may be anything, including an internal
    // service's reply: its reasons name where it's wrong, not what it says.
    let third_party = !served_by(&e.url, listed_by);
    let head = match head {
        Ok(h) => h,
        Err(err) => {
            d.status = Status::Rejected;
            d.reason = if third_party && !err.is_syntax() && !err.is_eof() {
                "not JSON: not a screens document".to_string()
            } else {
                format!("not JSON: {err}")
            };
            return (d, false);
        }
    };
    d.publisher = head.publisher.unwrap_or_default();
    d.updated_at = head.updated_at.unwrap_or_default();
    d.expires_at = head.expires_at.unwrap_or_default();
    for n in head.networks.unwrap_or_default() {
        d.networks.push(n.id.unwrap_or_default());
        d.screens += n.screens.map_or(0, |s| s.len());
    }
    match validate_schema(Kind::Screens, &d.raw, &SchemaOptions::default()) {
        Err(err) => {
            d.status = Status::Rejected;
            d.reason = err.to_string();
            (d, false)
        }
        Ok(errs) if !errs.is_empty() => {
            d.status = Status::Rejected;
            d.reason = if third_party {
                let at = if errs[0].path.is_empty() {
                    "/"
                } else {
                    &errs[0].path
                };
                format!(
                    "not a valid screens document ({} schema errors, first at {at})",
                    errs.len()
                )
            } else {
                format!(
                    "not a valid screens document ({} schema errors, first: {})",
                    errs.len(),
                    errs[0]
                )
            };
            (d, false)
        }
        Ok(_) => (d, true),
    }
}

/// Whether `url` is on `domain`'s own host (and port, for a local one).
fn served_by(url: &str, domain: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        let host = u.host_str().unwrap_or("");
        let at = match u.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        at.eq_ignore_ascii_case(domain)
    })
}

/// Makes the trust decision for a fetched, valid document. `scope` is None
/// for a publisher's own index, or the networks a delegation covers (empty
/// meaning all of them).
fn decide(
    d: &mut DiscoveredDocument,
    publisher: &str,
    scope: Option<&[String]>,
    now: DateTime<FixedOffset>,
) {
    if d.publisher != publisher {
        d.status = Status::Rejected;
        d.reason = format!(
            "declares publisher {}, but {}'s index can only vouch for {}'s own documents",
            d.publisher, d.listed_by, publisher
        );
        return;
    }
    if let Some(scope) = scope.filter(|s| !s.is_empty()) {
        for n in &d.networks {
            if !scope.contains(n) {
                d.status = Status::Rejected;
                d.reason = format!(
                    "network {n} is outside the scope {} delegated to {}",
                    scope_string(scope),
                    d.listed_by
                );
                return;
            }
        }
    }
    if !d.expires_at.is_empty()
        && let Ok(exp) = DateTime::parse_from_rfc3339(&d.expires_at)
        && now > exp
    {
        d.status = Status::Expired;
        d.reason = format!(
            "expired at {}; it must not be used for validation",
            d.expires_at
        );
        return;
    }
    d.status = Status::Trusted;
    d.reason = if d.delegated_by.is_empty() {
        format!("listed in {}'s index", d.listed_by)
    } else {
        format!(
            "{} names {} as a delegate, and {}'s index lists this URL",
            d.delegated_by, d.listed_by, d.listed_by
        )
    };
}

/// Reads a delegation scope: "" covers every network, and `network:<id>`
/// one network. Anything else is unrecognised.
fn parse_scope(s: &str) -> Option<Vec<String>> {
    if s.is_empty() {
        return Some(Vec::new());
    }
    match s.strip_prefix("network:") {
        Some(id) if !id.is_empty() => Some(vec![id.to_string()]),
        _ => None,
    }
}

fn scope_string(scope: &[String]) -> String {
    format!("network:{}", scope.join(","))
}

#[cfg(test)]
mod tests;
