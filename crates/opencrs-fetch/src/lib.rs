//! What the native callers of OpenCRS share for talking to the network: one
//! guarded HTTP client ([`http`]) and the publisher check's fetch loop
//! ([`publisher`]). The command line and the Node and Python bindings all
//! fetch through here, so they refuse the same addresses and read the same
//! limits. The core itself does no I/O, and the WebAssembly callers (Go,
//! the browser) bring their own.

pub mod http;
pub mod publisher;

// The command line's tests have the local HTTP server; these use it too.
#[cfg(test)]
#[path = "../../opencrs-cli/tests/common/server.rs"]
mod server;
#[cfg(test)]
mod tls_server;

pub use http::{Control, Http, Reply, UreqFetcher};
pub use publisher::{DEFAULT_TOTAL_TIMEOUT, FetchOptions, Fetched, fetch_publisher};

use chrono::{DateTime, FixedOffset};
use opencrs::discover::{DiscoveryResult, Options};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// Controls [`discover`].
#[derive(Debug, Clone, Default)]
pub struct DiscoverOptions {
    /// Maps a domain to the bearer token for its partner documents.
    pub tokens: HashMap<String, String>,
    /// Local testing: allow http:// and connections to localhost, 127.0.0.1
    /// and [::1], and no other internal address. Honoured only for a
    /// publisher that is itself on localhost: a public publisher that lists
    /// or redirects to localhost gets nothing.
    pub allow_local_http: bool,
    /// For one request, redirects included; zero means 30 seconds.
    pub timeout: Duration,
    /// For the whole run; zero means [`DEFAULT_TOTAL_TIMEOUT`]. A URL not yet
    /// fetched when it passes fails as not fetched.
    pub total_timeout: Duration,
    /// Set by another thread to stop the run early.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// What [`discover`] found, and whether it was cut short.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub result: DiscoveryResult,
    /// Set when `DiscoverOptions::cancel` stopped the run.
    pub cancelled: bool,
}

/// Discovery over the network, as `opencrs fetch DOMAIN` does it: reads the
/// publisher's index and each document it lists, follows delegations, and
/// marks each document trusted, expired, rejected, skipped or failed. An
/// error means the index itself couldn't be fetched or trusted.
pub fn discover(
    domain: &str,
    now: DateTime<FixedOffset>,
    opts: &DiscoverOptions,
) -> Result<Discovered, opencrs::Error> {
    let total = if opts.total_timeout.is_zero() {
        DEFAULT_TOTAL_TIMEOUT
    } else {
        opts.total_timeout.min(publisher::MAX_TIMEOUT)
    };
    let mut fetcher = UreqFetcher::new(opts.allow_local_http);
    fetcher.timeout = opts.timeout.min(publisher::MAX_TIMEOUT);
    fetcher.control = Control::new(Instant::now().checked_add(total), opts.cancel.clone());
    let opt = Options {
        tokens: opts
            .tokens
            .iter()
            .map(|(d, t)| (d.to_lowercase(), t.clone()))
            .collect(),
        allow_local_http: opts.allow_local_http,
        now,
        max_index_entries: 0,
        max_run_bytes: 0,
        max_fetches: 0,
        keep_raw: false,
    };
    let result = opencrs::discover::discover(domain, &opt, &fetcher)?;
    Ok(Discovered {
        result,
        cancelled: fetcher.control.cancelled(),
    })
}
