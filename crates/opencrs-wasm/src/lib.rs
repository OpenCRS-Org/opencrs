//! OpenCRS core as a plain-ABI WebAssembly module: no wasm-bindgen. Memory
//! is the only way in or out, plus one import, `env.opencrs_interrupted() ->
//! i32`: the module calls it every so often during long work, and if it
//! returns non-zero the call stops and answers `{"error": "CRS-INTERRUPTED"}`.
//! A host with no way to stop a call returns 0.
//!
//! Every operation takes a request at `(ptr, len)` and returns
//! `(ptr << 32) | len` of a UTF-8 JSON response that the caller frees with
//! `opencrs_free`. A response is the result object or `{"error": "..."}`.
//!
//! Most requests are UTF-8 JSON. The calls that carry documents, which may
//! be tens of megabytes, take them as raw bytes instead, so the module never
//! holds an escaped copy beside the decoded one:
//!
//! - `opencrs_lint`: the request is the document itself.
//! - `opencrs_check`, `opencrs_resolve`, `opencrs_resolve_strictest` and
//!   `opencrs_publisher`: the request is a frame, a
//!   little-endian u32 `n`, then `n` bytes of JSON header, then the
//!   documents back to back. The header's `blobs` lists their lengths.
//!   `check` takes the manifest, then each spec; `resolve` and
//!   `resolve_strictest` take the spec.
//!
//! `check`, `resolve` and `resolve_strictest` take ownership of the request: the module frees it
//! as soon as the documents are decoded, and the caller must not free it.
//! The operations themselves are ordinary functions over byte slices (the
//! `ops` module), so they are tested natively; `abi` is the thin layer of
//! exports.

// Natively only the tests call these; on wasm32 the exports do.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod ops {
    use chrono::DateTime;
    use opencrs::SPEC_VERSION;
    use opencrs::batch::{GroupedViews, check_manifest, lint as lint_document};
    use opencrs::calc::{self, ScreenWidth};
    use opencrs::json;
    use opencrs::load::{decode_document, decode_manifest};
    use opencrs::probe::{Probe, Prober, Step};
    use opencrs::publisher::{Page, check_publisher};
    use opencrs::report::{Options, Report};
    use opencrs::resolve::parse_ref;
    use opencrs::schema::SchemaOptions;
    use opencrs::stream::Resolved;
    use opencrs::strictest::Strictest;
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};
    use std::cell::RefCell;
    use std::collections::{BTreeMap, HashMap};

    pub fn error(msg: impl std::fmt::Display) -> Vec<u8> {
        json::to_string(&json!({ "error": msg.to_string() })).into_bytes()
    }

    fn reply(v: &Value) -> Vec<u8> {
        json::to_string(v).into_bytes()
    }

    fn parse<'a, T: Deserialize<'a>>(req: &'a [u8], usage: &str) -> Result<T, Vec<u8>> {
        serde_json::from_slice(req).map_err(|e| error(format!("usage: {usage}: {e}")))
    }

    /// A request that carries documents: the JSON header, and the raw
    /// documents in the order the header's `blobs` gives their lengths.
    struct Frame<'a, T> {
        header: T,
        blobs: Vec<&'a [u8]>,
    }

    #[derive(Deserialize)]
    struct Lengths {
        #[serde(default)]
        blobs: Vec<usize>,
    }

    fn frame<'a, T: Deserialize<'a>>(req: &'a [u8], usage: &str) -> Result<Frame<'a, T>, Vec<u8>> {
        let bad = || error(format!("usage: {usage}: malformed request frame"));
        let (n, rest) = req.split_first_chunk::<4>().ok_or_else(bad)?;
        let n = u32::from_le_bytes(*n) as usize;
        if n > rest.len() {
            return Err(bad());
        }
        let (head, mut rest) = rest.split_at(n);
        let lens: Lengths = parse(head, usage)?;
        let mut blobs = Vec::with_capacity(lens.blobs.len());
        for len in lens.blobs {
            if len > rest.len() {
                return Err(bad());
            }
            let (blob, tail) = rest.split_at(len);
            blobs.push(blob);
            rest = tail;
        }
        if !rest.is_empty() {
            return Err(bad());
        }
        Ok(Frame {
            header: parse(head, usage)?,
            blobs,
        })
    }

    pub fn version(_req: &[u8]) -> Vec<u8> {
        reply(&json!({ "version": SPEC_VERSION }))
    }

    /// Runs an operation that the host may stop (see
    /// `opencrs::budget::checkpoint`): if it did, whatever the operation
    /// made is dropped for an error that says so.
    fn stoppable(f: impl FnOnce() -> Vec<u8>) -> Vec<u8> {
        opencrs::budget::clear_interrupt();
        let out = f();
        if opencrs::budget::interrupted() {
            opencrs::budget::clear_interrupt();
            return error(opencrs::budget::INTERRUPTED_CODE);
        }
        out
    }

    pub fn lint(doc: &[u8]) -> Vec<u8> {
        stoppable(|| lint_inner(doc))
    }

    fn lint_inner(doc: &[u8]) -> Vec<u8> {
        let opt = SchemaOptions {
            list: true,
            ..SchemaOptions::default()
        };
        match lint_document(doc, None, &opt) {
            Ok(res) => json::to_bytes_sized(&res),
            Err(e) => error(e),
        }
    }

    #[derive(Deserialize)]
    struct CheckReq {
        #[serde(default)]
        probes: HashMap<String, Probe>,
        now: String,
    }

    #[derive(Serialize)]
    struct CheckResult<'a> {
        reports: Vec<Report>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        notes: Vec<String>,
        /// Each report's target to the screens it covers, for drawing proofs:
        /// entries of `{"screens": [ids], ...requirements}`, one for each
        /// distinct set of requirements.
        requirements: BTreeMap<String, GroupedViews<'a>>,
    }

    fn try_check(req: Vec<u8>) -> Result<Vec<u8>, Vec<u8>> {
        let bytes = req.len();
        opencrs::budget::ensure(bytes, || try_check_within(req))
    }

    fn try_check_within(req: Vec<u8>) -> Result<Vec<u8>, Vec<u8>> {
        let (r, docs, m) = {
            let f: Frame<CheckReq> = frame(
                &req,
                r#"{"probes": {...}, "now": "RFC 3339", "blobs": [manifest, spec, ...]}"#,
            )?;
            let [manifest, specs @ ..] = f.blobs.as_slice() else {
                return Err(error("usage: check needs a manifest"));
            };
            let docs = specs
                .iter()
                .map(|s| decode_document(s))
                .collect::<Result<Vec<_>, _>>()
                .map_err(error)?;
            let m = decode_manifest(manifest).map_err(error)?;
            (f.header, docs, m)
        };
        drop(req);
        let now = DateTime::parse_from_rfc3339(&r.now).map_err(error)?;
        let (reports, mut notes) =
            check_manifest(&docs, &m, &r.probes, &Options::new(now)).map_err(error)?;
        let mut requirements = BTreeMap::new();
        for rep in &reports {
            if requirements.contains_key(&rep.target) {
                continue;
            }
            let Ok(rf) = parse_ref(&rep.target) else {
                continue;
            };
            // The first document that resolves the target answers for it.
            let mut found = None;
            for d in &docs {
                match GroupedViews::new(d, rf.clone()) {
                    Ok(g) => {
                        found = Some(g);
                        break;
                    }
                    // Too much to list: the reports say so, and so does a note.
                    Err(opencrs::resolve::ResolveError::TooComplex(why)) => {
                        notes.push(format!("{}: {why}", rep.target));
                        break;
                    }
                    Err(_) => {}
                }
            }
            if let Some(g) = found {
                requirements.insert(rep.target.clone(), g);
            }
        }
        Ok(json::to_bytes_sized(&CheckResult {
            reports,
            notes,
            requirements,
        }))
    }

    pub fn check(req: Vec<u8>) -> Vec<u8> {
        stoppable(|| try_check(req).unwrap_or_else(|e| e))
    }

    #[derive(Deserialize)]
    struct ResolveReq {
        #[serde(rename = "ref")]
        reference: String,
        /// Answer as the distinct views and each screen's view and id
        /// (`Resolved::grouped`) rather than one view per screen, for a
        /// caller that puts them together itself.
        #[serde(default)]
        grouped: bool,
    }

    fn try_resolve(req: Vec<u8>) -> Result<Vec<u8>, Vec<u8>> {
        let bytes = req.len();
        opencrs::budget::ensure(bytes, || try_resolve_within(req))
    }

    fn try_resolve_within(req: Vec<u8>) -> Result<Vec<u8>, Vec<u8>> {
        // The document is read a screen at a time and never decoded whole,
        // and only the distinct views are built; the request is freed before
        // the answer is written, so at no point are both in memory.
        let (grouped, resolved) = {
            let f: Frame<ResolveReq> =
                frame(&req, r#"{"ref": "...", "grouped"?: bool, "blobs": [spec]}"#)?;
            let [spec] = f.blobs.as_slice() else {
                return Err(error("usage: resolve needs one spec"));
            };
            (
                f.header.grouped,
                Resolved::from_text(spec, &f.header.reference).map_err(error)?,
            )
        };
        drop(req);
        Ok(if grouped {
            resolved.grouped()
        } else {
            resolved.per_screen()
        })
    }

    pub fn resolve(req: Vec<u8>) -> Vec<u8> {
        stoppable(|| try_resolve(req).unwrap_or_else(|e| e))
    }

    #[derive(Deserialize)]
    struct StrictestReq {
        #[serde(rename = "ref")]
        reference: String,
    }

    fn try_resolve_strictest(req: Vec<u8>) -> Result<Vec<u8>, Vec<u8>> {
        let bytes = req.len();
        opencrs::budget::ensure(bytes, || {
            // As `resolve`: the spec is read a screen at a time, and the
            // request is freed before the answer is written.
            let strictest = {
                let f: Frame<StrictestReq> = frame(&req, r#"{"ref": "...", "blobs": [spec]}"#)?;
                let [spec] = f.blobs.as_slice() else {
                    return Err(error("usage: resolve_strictest needs one spec"));
                };
                Strictest::from_text(spec, &f.header.reference).map_err(error)?
            };
            drop(req);
            Ok(strictest.to_json())
        })
    }

    pub fn resolve_strictest(req: Vec<u8>) -> Vec<u8> {
        stoppable(|| try_resolve_strictest(req).unwrap_or_else(|e| e))
    }

    #[derive(Deserialize)]
    struct ScreenReq {
        width_mm: f64,
        width_px: i64,
    }

    impl ScreenReq {
        fn width(&self) -> ScreenWidth {
            ScreenWidth {
                mm: self.width_mm,
                px: self.width_px,
            }
        }
    }

    #[derive(Deserialize)]
    struct LegibilityReq {
        distance_m: [f64; 2],
        ratio: Option<f64>,
        screen: Option<ScreenReq>,
        dwell_s: Option<f64>,
        rate: Option<f64>,
    }

    pub fn legibility(req: &[u8]) -> Vec<u8> {
        let r: LegibilityReq = match parse(
            req,
            r#"{"distance_m": [near, far], "ratio"?, "screen"?: {"width_mm", "width_px"}, "dwell_s"?, "rate"?}"#,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let s = r.screen.as_ref().map(ScreenReq::width);
        match calc::legibility(r.distance_m, r.ratio, s, r.dwell_s, r.rate) {
            Ok(l) => reply(&json!(l)),
            Err(e) => error(e),
        }
    }

    #[derive(Deserialize)]
    struct DwellReq {
        speed_kmh: f64,
        distance_m: [f64; 2],
        ratio: Option<f64>,
        rate: Option<f64>,
        screen: Option<ScreenReq>,
        #[serde(default)]
        rule_packs: Vec<String>,
    }

    pub fn dwell(req: &[u8]) -> Vec<u8> {
        let r: DwellReq = match parse(
            req,
            r#"{"speed_kmh", "distance_m": [near, far], "ratio"?, "rate"?, "screen"?, "rule_packs"?: [...]}"#,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let s = r.screen.as_ref().map(ScreenReq::width);
        match calc::dwell(r.speed_kmh, r.distance_m, r.ratio, r.rate, s, &r.rule_packs) {
            Ok(d) => reply(&json!(d)),
            Err(e) => error(e),
        }
    }

    /// A page's headers, by the URL it was fetched from. Its body is the
    /// blob at the same position in the frame.
    #[derive(Deserialize)]
    struct PageEntry {
        url: String,
        #[serde(flatten)]
        page: Page<'static>,
    }

    #[derive(Deserialize)]
    struct PublisherReq {
        domain: String,
        pages: Vec<PageEntry>,
        now: String,
        #[serde(default)]
        missing: String,
    }

    fn try_publisher(req: Vec<u8>) -> Result<Vec<u8>, Vec<u8>> {
        let f: Frame<PublisherReq> = frame(
            &req,
            r#"{"domain": "...", "pages": [{"url", "status", ...}], "now": "RFC 3339", "missing"?: "...", "blobs": [body of each page]}"#,
        )?;
        let r = f.header;
        if r.pages.len() != f.blobs.len() {
            return Err(error(
                "usage: publisher needs one blob for each page, in order",
            ));
        }
        // The bodies stay where they arrived; a page borrows its own.
        let pages: BTreeMap<String, Page<'_>> = r
            .pages
            .into_iter()
            .zip(f.blobs)
            .map(|(e, body)| (e.url, Page { body, ..e.page }))
            .collect();
        let now = DateTime::parse_from_rfc3339(&r.now).map_err(error)?;
        let missing = if r.missing.is_empty() {
            "not fetched"
        } else {
            &r.missing
        };
        Ok(reply(&json!(check_publisher(
            &r.domain, &pages, missing, now
        ))))
    }

    pub fn publisher(req: Vec<u8>) -> Vec<u8> {
        stoppable(|| try_publisher(req).unwrap_or_else(|e| e))
    }

    pub fn rule_packs(_req: &[u8]) -> Vec<u8> {
        reply(&json!({ "rule_packs": calc::pack_list() }))
    }

    /// The most probe sessions open at once. Starting another ends the
    /// oldest, so sessions a host abandons can't pile up.
    pub const MAX_SESSIONS: usize = 64;

    #[derive(Default)]
    struct Sessions {
        last: u32,
        /// Oldest first.
        live: Vec<(u32, Prober)>,
    }

    impl Sessions {
        fn get_mut(&mut self, id: u32) -> Option<&mut Prober> {
            self.live.iter_mut().find(|(i, _)| *i == id).map(|(_, p)| p)
        }

        fn remove(&mut self, id: u32) {
            self.live.retain(|(i, _)| *i != id);
        }
    }

    thread_local! {
        static SESSIONS: RefCell<Sessions> = RefCell::new(Sessions::default());
    }

    #[derive(Deserialize)]
    struct StartReq {
        size: u64,
        sha256: String,
    }

    /// Describes where a probe session stands; frees the session when done.
    fn step(s: &mut Sessions, id: u32) -> Vec<u8> {
        let Some(p) = s.get_mut(id) else {
            return error(format!("no probe session {id}"));
        };
        match p.next() {
            Step::Need { offset, len } => reply(&json!({
                "session": id,
                "need": { "offset": offset, "len": len },
            })),
            Step::Done(probe) => {
                s.remove(id);
                let probe = serde_json::to_value(&probe).unwrap_or(Value::Null);
                reply(&json!({ "session": 0, "need": null, "probe": probe }))
            }
        }
    }

    pub fn probe_start(req: &[u8]) -> Vec<u8> {
        let r: StartReq = match parse(req, r#"{"size": n, "sha256": "..."}"#) {
            Ok(r) => r,
            Err(e) => return e,
        };
        SESSIONS.with_borrow_mut(|s| {
            s.last = s.last.wrapping_add(1).max(1);
            let id = s.last;
            s.remove(id); // the counter wrapped onto a live session
            if s.live.len() >= MAX_SESSIONS {
                s.live.remove(0);
            }
            s.live.push((id, Prober::new(r.size, r.sha256)));
            step(s, id)
        })
    }

    pub fn probe_feed(session: u32, bytes: &[u8]) -> Vec<u8> {
        SESSIONS.with_borrow_mut(|s| {
            match s.get_mut(session) {
                Some(p) => p.feed(bytes),
                None => return error(format!("no probe session {session}")),
            }
            step(s, session)
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn val(b: Vec<u8>) -> Value {
            serde_json::from_slice(&b).unwrap()
        }

        /// A frame as `ops` reads them: header, then the documents.
        fn framed(mut header: Value, blobs: &[&[u8]]) -> Vec<u8> {
            header["blobs"] = blobs.iter().map(|b| b.len()).collect::<Vec<_>>().into();
            let head = serde_json::to_vec(&header).unwrap();
            let mut out = (head.len() as u32).to_le_bytes().to_vec();
            out.extend(head);
            blobs.iter().for_each(|b| out.extend_from_slice(b));
            out
        }

        fn example(name: &str) -> Vec<u8> {
            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/harbourline")
                .join(name);
            std::fs::read(p).unwrap()
        }

        #[test]
        fn version_is_the_spec_version() {
            assert_eq!(val(version(b"")), json!({"version": "0.1"}));
        }

        #[test]
        fn lint_clean_example_and_garbage() {
            let v = val(lint(&example("retail.screens.json")));
            assert_eq!(v["kind"], "screens");
            assert!(v.get("schema_errors").is_none() && v.get("document_checks").is_none());
            assert!(val(lint(b"{}"))["error"].is_string());
            assert!(val(lint(b"nonsense"))["error"].is_string());
        }

        #[test]
        fn check_and_resolve_the_example() {
            let png = |n: &str| example(n);
            let mut probes = serde_json::Map::new();
            for n in ["lunch-portrait.png", "lunch-portrait-logo-right.png"] {
                let data = png(n);
                let mut v = val(probe_start(
                    &serde_json::to_vec(&json!({"size": data.len(), "sha256": "00"})).unwrap(),
                ));
                while !v["need"].is_null() {
                    let off = v["need"]["offset"].as_u64().unwrap() as usize;
                    let end = (off + v["need"]["len"].as_u64().unwrap() as usize).min(data.len());
                    v = val(probe_feed(
                        v["session"].as_u64().unwrap() as u32,
                        &data[off..end],
                    ));
                }
                probes.insert(n.into(), v["probe"].clone());
            }
            let spec = example("retail.screens.json");
            let req = framed(
                json!({"probes": probes, "now": "2026-10-22T12:00:00+11:00"}),
                &[&example("creative.json"), &spec],
            );
            let v = val(check(req));
            assert!(v["error"].is_null(), "{v}");
            assert!(v["reports"].as_array().unwrap().len() >= 2);
            let t = "harbourline.example/retail/portrait-totem-75";
            assert_eq!(v["requirements"][t].as_array().unwrap().len(), 2);

            let v = val(resolve(framed(json!({"ref": t}), &[&spec])));
            assert_eq!(v.as_array().unwrap().len(), 2);
            let v = val(resolve_strictest(framed(json!({"ref": t}), &[&spec])));
            assert_eq!(v["screens"], 2);
            assert_eq!(v["ref"], t);
            let r = framed(json!({"ref": "nonsense"}), &[&spec]);
            assert!(val(resolve_strictest(r))["error"].is_string());
            let r = framed(json!({"ref": "nonsense"}), &[&spec]);
            assert!(val(resolve(r))["error"].is_string());
            let r = framed(json!({"now": ""}), &[b"{}"]);
            assert!(val(check(r))["error"].is_string());
            // Malformed frames are errors, not panics.
            for bad in [&b""[..], b"\xff\xff\xff\xff{}", b"\x02\0\0\0{}"] {
                assert!(val(check(bad.to_vec()))["error"].is_string());
                assert!(val(resolve(bad.to_vec()))["error"].is_string());
                assert!(val(resolve_strictest(bad.to_vec()))["error"].is_string());
            }
            let r = framed(json!({"now": "2026-10-22T12:00:00+11:00"}), &[b"{}"]);
            let mut short = r.clone();
            short.pop();
            assert!(
                val(check(short))["error"]
                    .as_str()
                    .unwrap()
                    .contains("frame")
            );
            let mut long = r;
            long.push(b' ');
            assert!(
                val(check(long))["error"]
                    .as_str()
                    .unwrap()
                    .contains("frame")
            );
        }

        #[test]
        fn abandoned_sessions_are_capped() {
            let start = || {
                val(probe_start(br#"{"size":100,"sha256":"00"}"#))["session"]
                    .as_u64()
                    .unwrap() as u32
            };
            let first = start();
            for _ in 0..(MAX_SESSIONS * 4) {
                start();
            }
            let n = SESSIONS.with_borrow(|s| s.live.len());
            assert_eq!(n, MAX_SESSIONS);
            // The oldest went first; the newest still works.
            assert!(val(probe_feed(first, b""))["error"].is_string());
            let newest = start();
            assert!(val(probe_feed(newest, b""))["error"].is_null());
        }

        #[test]
        fn probe_session_reads_a_png() {
            let png = example("lunch-portrait.png");
            let req = json!({"size": png.len(), "sha256": "ab"});
            let mut v = val(probe_start(&serde_json::to_vec(&req).unwrap()));
            while !v["need"].is_null() {
                let off = v["need"]["offset"].as_u64().unwrap() as usize;
                let len = v["need"]["len"].as_u64().unwrap() as usize;
                let end = (off + len).min(png.len());
                v = val(probe_feed(
                    v["session"].as_u64().unwrap() as u32,
                    &png[off..end],
                ));
            }
            assert_eq!(v["probe"]["media_type"], "image/png");
            assert_eq!(v["probe"]["bytes"], png.len());
            assert_eq!(v["probe"]["sha256"], "ab");
            assert!(v["probe"]["pixels"].is_array());
            // the session is gone once done
            assert!(val(probe_feed(1, b""))["error"].is_string());
        }

        #[test]
        fn empty_file_is_done_at_once() {
            let v = val(probe_start(br#"{"size":0,"sha256":"e3"}"#));
            assert!(v["need"].is_null());
            assert_eq!(v["probe"]["bytes"], 0);
        }

        #[test]
        fn calculators() {
            let v = val(legibility(
                br#"{"distance_m":[2,6],"screen":{"width_mm":940,"width_px":1080}}"#,
            ));
            assert_eq!(v["key"]["px"], 46);
            assert_eq!(v["legal"]["px"], 16);
            let v = val(dwell(
                br#"{"speed_kmh":80,"distance_m":[20,150],"rule_packs":["au-nsw-transport-corridor@2017-11"]}"#,
            ));
            assert_eq!(v["glance"]["words"], 4);
            assert_eq!(v["packs"][0]["min_dwell_s"], 25.0);
            assert!(
                val(dwell(
                    br#"{"speed_kmh":80,"distance_m":[20,150],"rule_packs":["x@1"]}"#
                ))["error"]
                    .is_string()
            );
            assert!(val(legibility(b"{}"))["error"].is_string());
            assert_eq!(
                val(rule_packs(b""))
                    .get("rule_packs")
                    .and_then(|p| p.as_array())
                    .map(Vec::len),
                Some(6)
            );
        }

        #[test]
        fn publisher_check() {
            const INDEX: &str = "https://harbourline.example/.well-known/opencrs.json";
            let ask = |pages: Value, bodies: &[&[u8]]| {
                val(publisher(framed(
                    json!({"domain": "harbourline.example", "pages": pages, "now": "2026-10-25T12:00:00Z"}),
                    bodies,
                )))
            };
            let v = ask(json!([{"url": INDEX, "status": 404}]), &[b""]);
            assert!(v["error"].as_str().unwrap().contains("HTTP 404"), "{v}");
            assert_eq!(v["errors"], 1);
            assert_eq!(v["wanted"], json!([]));
            let v = ask(json!([]), &[]);
            assert_eq!(v["wanted"], json!([INDEX]));
            assert_eq!(
                v["wanted_detail"],
                json!([{"url": INDEX, "kind": "index", "same_host": true}])
            );
            // Bodies are bytes, not JSON text: markup and non-UTF-8 arrive
            // as they are, and the index is read from its blob.
            let v = ask(
                json!([{"url": INDEX, "status": 200, "content_type": "application/json"}]),
                &[b"<&>\xff"],
            );
            assert!(v["error"].is_string() && v["errors"] == 1, "{v}");
            // One blob for each page.
            let v = ask(json!([{"url": INDEX, "status": 200}]), &[]);
            assert!(v["error"].as_str().unwrap().starts_with("usage:"), "{v}");
            assert!(
                val(publisher(b"{}".to_vec()))["error"]
                    .as_str()
                    .unwrap()
                    .starts_with("usage:")
            );
        }

        #[test]
        fn bad_requests_are_errors() {
            assert!(val(probe_start(b"{}"))["error"].is_string());
            assert!(val(probe_feed(99, b"x"))["error"].is_string());
        }
    }
}

/// The exported functions. Pointers are 32-bit linear-memory addresses.
#[cfg(target_arch = "wasm32")]
mod abi {
    use super::ops;

    /// Reads a request the host wrote into memory.
    ///
    /// # Safety
    /// `(ptr, len)` must be a live allocation from `opencrs_alloc`.
    unsafe fn input<'a>(ptr: u32, len: u32) -> &'a [u8] {
        if len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
        }
    }

    /// Hands a response to the host as `(ptr << 32) | len`.
    fn give(v: Vec<u8>) -> u64 {
        let b = v.into_boxed_slice();
        let len = b.len() as u64;
        let ptr = Box::into_raw(b) as *mut u8 as usize as u64;
        (ptr << 32) | len
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn opencrs_alloc(len: u32) -> u32 {
        let b = vec![0u8; len as usize].into_boxed_slice();
        Box::into_raw(b) as *mut u8 as usize as u32
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn opencrs_free(ptr: u32, len: u32) {
        // Every allocation is a boxed slice of exactly `len` bytes.
        drop(unsafe { Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize) });
    }

    macro_rules! op {
        ($name:ident, $f:path) => {
            #[unsafe(no_mangle)]
            pub extern "C" fn $name(ptr: u32, len: u32) -> u64 {
                give($f(unsafe { input(ptr, len) }))
            }
        };
    }

    op!(opencrs_version, ops::version);
    /// Takes ownership of the allocation at `(ptr, len)`.
    ///
    /// # Safety
    /// `(ptr, len)` must be a live allocation from `opencrs_alloc` that the
    /// caller never uses or frees again.
    unsafe fn owned(ptr: u32, len: u32) -> Vec<u8> {
        unsafe { Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize) }
    }

    macro_rules! consuming {
        ($name:ident, $f:path) => {
            #[unsafe(no_mangle)]
            pub extern "C" fn $name(ptr: u32, len: u32) -> u64 {
                give($f(unsafe { owned(ptr, len) }))
            }
        };
    }

    op!(opencrs_lint, ops::lint);
    consuming!(opencrs_check, ops::check);
    consuming!(opencrs_resolve, ops::resolve);
    consuming!(opencrs_resolve_strictest, ops::resolve_strictest);
    consuming!(opencrs_publisher, ops::publisher);
    op!(opencrs_legibility, ops::legibility);
    op!(opencrs_dwell, ops::dwell);
    op!(opencrs_rule_packs, ops::rule_packs);
    op!(opencrs_probe_start, ops::probe_start);

    #[unsafe(no_mangle)]
    pub extern "C" fn opencrs_probe_feed(session: u32, ptr: u32, len: u32) -> u64 {
        give(ops::probe_feed(session, unsafe { input(ptr, len) }))
    }
}

/// getrandom's custom backend. The core has no randomness by design, and the
/// only caller is the hash-map seed inside the schema validator, which needs
/// no secrecy here; a fixed fill also keeps the module deterministic.
#[cfg(target_arch = "wasm32")]
mod entropy {
    /// # Safety
    /// Called by getrandom with a writable `(dest, len)`.
    #[unsafe(no_mangle)]
    unsafe extern "Rust" fn __getrandom_v03_custom(
        dest: *mut u8,
        len: usize,
    ) -> Result<(), getrandom::Error> {
        for i in 0..len {
            unsafe { dest.add(i).write(0x5a ^ i as u8) };
        }
        Ok(())
    }
}
