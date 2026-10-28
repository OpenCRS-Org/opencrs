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

pub use http::{Http, Reply, UreqFetcher};
pub use publisher::{FetchOptions, Fetched, fetch_publisher};

use chrono::{DateTime, FixedOffset};
use opencrs::discover::{DiscoveryResult, Options};
use std::collections::HashMap;

/// Discovery over the network, as `opencrs fetch DOMAIN` does it: reads the
/// publisher's index and each document it lists, follows delegations, and
/// marks each document trusted, expired, rejected, skipped or failed.
/// `tokens` maps a domain to the bearer token for its partner documents.
/// `allow_local_http` is local testing: it allows http:// and connections
/// to localhost, 127.0.0.1 and [::1], and no other internal address. An
/// error means the index itself couldn't be fetched or trusted.
pub fn discover(
    domain: &str,
    tokens: &HashMap<String, String>,
    allow_local_http: bool,
    now: DateTime<FixedOffset>,
) -> Result<DiscoveryResult, opencrs::Error> {
    let opt = Options {
        tokens: tokens
            .iter()
            .map(|(d, t)| (d.to_lowercase(), t.clone()))
            .collect(),
        allow_local_http,
        now,
        max_index_entries: 0,
        max_run_bytes: 0,
        max_fetches: 0,
        keep_raw: false,
    };
    opencrs::discover::discover(domain, &opt, &UreqFetcher::new(allow_local_http))
}
