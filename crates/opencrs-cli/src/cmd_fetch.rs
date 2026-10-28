use crate::cli::FetchArgs;
use crate::http::UreqFetcher;
use opencrs::discover::{
    DiscoveredDocument, DiscoveryResult, Fetcher, Options, Status, discover, merge, verify,
};
use opencrs::json;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;

#[derive(Serialize)]
struct Output<'a> {
    result: &'a DiscoveryResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified: Option<&'a DiscoveredDocument>,
}

pub fn run(args: &FetchArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    run_with(args, stdout, stderr, &|allow| {
        Box::new(UreqFetcher::new(allow))
    })
}

type MakeFetcher<'a> = &'a dyn Fn(bool) -> Box<dyn Fetcher>;

pub fn run_with(
    a: &FetchArgs,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    make: MakeFetcher<'_>,
) -> i32 {
    let tokens = &a.token;
    let out_dir = a.output.clone().unwrap_or_default();
    let as_json = a.json;
    let local_http = a.allow_local_http;
    let target = a.target.clone();
    let n = chrono::Local::now();
    let mut opt = Options {
        tokens: HashMap::new(),
        allow_local_http: local_http,
        now: n.with_timezone(n.offset()),
        max_index_entries: 0,
        max_run_bytes: 0,
        max_fetches: 0,
        keep_raw: true,
    };
    for t in tokens {
        match t.split_once('=') {
            Some((d, tok)) if !d.is_empty() && !tok.is_empty() => {
                opt.tokens.insert(d.to_lowercase(), tok.to_string());
            }
            _ => {
                let _ = writeln!(
                    stderr,
                    "opencrs fetch: --token wants DOMAIN=TOKEN, got {}",
                    opencrs::gofmt::quote(t)
                );
                return 2;
            }
        }
    }

    let fetcher = make(local_http);
    let (res, verified) = if target.contains("://") {
        match verify(&target, &opt, fetcher.as_ref()) {
            Ok((r, v)) => (r, Some(v)),
            Err(e) => {
                let _ = writeln!(stderr, "opencrs fetch: {e}");
                return 2;
            }
        }
    } else {
        match discover(&target, &opt, fetcher.as_ref()) {
            Ok(r) => (r, None),
            Err(e) => {
                let _ = writeln!(stderr, "opencrs fetch: {e}");
                return 2;
            }
        }
    };

    let mut status = 0;
    for d in &res.documents {
        if matches!(
            d.status,
            Status::Rejected | Status::Failed | Status::Expired
        ) {
            status = 1;
        }
    }
    if verified
        .as_ref()
        .is_some_and(|v| v.status != Status::Trusted)
    {
        status = 1;
    }

    if !out_dir.is_empty()
        && let Err(e) = save_trusted(&res, &out_dir, stdout)
    {
        let _ = writeln!(stderr, "opencrs fetch: {e}");
        return 2;
    }
    if as_json {
        let out = Output {
            result: &res,
            verified: verified.as_ref(),
        };
        let _ = stdout.write_all(json::to_string_pretty(&out).as_bytes());
        return status;
    }

    let _ = writeln!(
        stdout,
        "{} ({}): index at {}",
        res.domain, res.index.publisher.name, res.index_url
    );
    for d in &res.documents {
        let _ = writeln!(
            stdout,
            "  {:<9} {:<8} {}",
            d.status.to_string(),
            d.visibility,
            d.url
        );
        let mut facts = Vec::new();
        if !d.networks.is_empty() {
            facts.push(format!(
                "networks {}, {} screens",
                d.networks.join(", "),
                d.screens
            ));
        }
        if !d.updated_at.is_empty() {
            facts.push(format!("updated {}", d.updated_at));
        }
        if !d.expires_at.is_empty() {
            facts.push(format!("expires {}", d.expires_at));
        }
        if !d.cache_control.is_empty() {
            facts.push(format!("Cache-Control: {}", d.cache_control));
        }
        if !facts.is_empty() {
            let _ = writeln!(stdout, "            {}", facts.join("; "));
        }
        let _ = writeln!(stdout, "            {}", d.reason);
    }
    for dr in &res.delegates {
        let scope = if dr.scope.is_empty() {
            "all networks"
        } else {
            &dr.scope
        };
        let mut line = format!("  delegate  {} ({scope})", dr.domain);
        if !dr.error.is_empty() {
            line.push_str(&format!(": {}", dr.error));
        }
        let _ = writeln!(stdout, "{line}");
    }
    for why in &res.limits {
        let _ = writeln!(stdout, "  limit     {why}");
    }
    if let Some(v) = &verified {
        let _ = writeln!(stdout, "\n{target}: {}. {}", v.status, v.reason);
    }
    status
}

/// The file name a document is saved under: the last non-empty segment
/// of its URL's path as parsed (and so as fetched), percent-decoded. Only
/// letters, digits, `.`, `_` and `-` are allowed, and never a name Windows
/// reserves or treats specially; anything else is saved as
/// `document.json`.
fn save_name(url: &str) -> String {
    const FALLBACK: &str = "document.json";
    let Ok(u) = url::Url::parse(url) else {
        return FALLBACK.into();
    };
    let Some(seg) = u
        .path_segments()
        .and_then(|mut s| s.rfind(|seg| !seg.is_empty()))
    else {
        return FALLBACK.into();
    };
    let name = String::from_utf8_lossy(&percent_decode(seg)).into_owned();
    let allowed = name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if !allowed || reserved || name.starts_with('.') || name.ends_with('.') || name.len() > 200 {
        return FALLBACK.into();
    }
    name
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// Go's `filepath.Join` for two elements: joined, then cleaned.
fn join(a: &str, b: &str) -> String {
    let joined = if a.is_empty() {
        b.to_string()
    } else {
        format!("{a}/{b}")
    };
    crate::io::clean_path(&joined)
}

/// Writes each trusted document exactly as served, one directory per
/// publisher. If a publisher has a trusted partner document, it also writes
/// merged.screens.json combining them.
fn save_trusted(res: &DiscoveryResult, dir: &str, stdout: &mut dyn Write) -> Result<(), String> {
    let mut by_publisher: BTreeMap<&str, Vec<&DiscoveredDocument>> = BTreeMap::new();
    for d in res.trusted() {
        by_publisher.entry(&d.publisher).or_default().push(d);
    }
    for (publisher, docs) in by_publisher {
        let pdir = join(dir, publisher);
        std::fs::create_dir_all(&pdir).map_err(|e| crate::io::go_error("mkdir", &pdir, &e))?;
        let mut used: HashSet<String> = HashSet::new();
        let mut partner = false;
        let mut raws: Vec<&[u8]> = Vec::new();
        for d in docs {
            let base = save_name(&d.url);
            let mut name = base.clone();
            let mut i = 2;
            while used.contains(&name) {
                name = format!("{i}-{base}");
                i += 1;
            }
            used.insert(name.clone());
            let path = join(&pdir, &name);
            if std::path::Path::new(&path).parent() != Some(std::path::Path::new(&pdir)) {
                return Err(format!("refusing to save {} as {path}", d.url));
            }
            std::fs::write(&path, &d.raw).map_err(|e| crate::io::go_error("open", &path, &e))?;
            let _ = writeln!(stdout, "saved {path}");
            partner = partner || d.visibility == "partner";
            raws.push(&d.raw);
        }
        if !partner {
            continue;
        }
        let (merged, conflicts) =
            merge(&raws).map_err(|e| format!("merging {publisher}'s documents: {e}"))?;
        let out = join(&pdir, "merged.screens.json");
        std::fs::write(&out, merged).map_err(|e| crate::io::go_error("open", &out, &e))?;
        let _ = writeln!(stdout, "saved {out} (public and partner documents merged)");
        for c in conflicts {
            let _ = writeln!(stdout, "  conflict: {c}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The review's H2: the name came from the raw URL text, where `\`
    /// (a path separator on Windows) and `..` could survive.
    #[test]
    fn saved_names_stay_in_the_publisher_directory() {
        for (url, want) in [
            (
                "https://p.example/specs/retail.screens.json",
                "retail.screens.json",
            ),
            (
                "https://p.example/specs/retail.screens.json?v=2#x",
                "retail.screens.json",
            ),
            ("https://p.example/specs/retail/", "retail"),
            ("https://p.example/", "document.json"),
            ("https://p.example", "document.json"),
            ("https://p.example/x\\..\\..\\evil.bat", "evil.bat"),
            ("https://p.example/x/..%5C..%5Cevil.bat", "document.json"),
            ("https://p.example/x/%2E%2E", "document.json"),
            ("https://p.example/x/..", "document.json"),
            ("https://p.example/C:evil.json", "document.json"),
            ("https://p.example/CON", "document.json"),
            ("https://p.example/nul.screens.json", "document.json"),
            ("https://p.example/com1.json", "document.json"),
            ("https://p.example/computer.json", "computer.json"),
            ("https://p.example/a%20b.json", "document.json"),
            ("https://p.example/my-spec_v2.json", "my-spec_v2.json"),
            ("https://p.example/.hidden", "document.json"),
            ("https://p.example/trailing.", "document.json"),
            ("not a url", "document.json"),
        ] {
            assert_eq!(save_name(url), want, "{url}");
        }
    }
}
