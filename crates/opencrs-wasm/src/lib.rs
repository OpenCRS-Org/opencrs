//! OpenCRS core as a plain-ABI WebAssembly module: no wasm-bindgen, no
//! imports. Memory is the only way in or out.
//!
//! Every operation takes a UTF-8 JSON request at `(ptr, len)` and returns
//! `(ptr << 32) | len` of a UTF-8 JSON response that the caller frees with
//! `opencrs_free`. A response is the result object or `{"error": "..."}`.
//! The operations themselves are ordinary functions over byte slices (the
//! `ops` module), so they are tested natively; `abi` is the thin layer of
//! exports.

// Natively only the tests call these; on wasm32 the exports do.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod ops {
    use chrono::DateTime;
    use opencrs::SPEC_VERSION;
    use opencrs::batch::{RequirementsView, check_manifest, lint as lint_document};
    use opencrs::calc::{self, ScreenWidth};
    use opencrs::json;
    use opencrs::load::{decode_document, decode_manifest};
    use opencrs::probe::{Probe, Prober, Step};
    use opencrs::report::{Options, Report};
    use opencrs::resolve::parse_ref;
    use opencrs::schema::SchemaOptions;
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

    pub fn version(_req: &[u8]) -> Vec<u8> {
        reply(&json!({ "version": SPEC_VERSION }))
    }

    #[derive(Deserialize)]
    struct LintReq {
        text: String,
    }

    pub fn lint(req: &[u8]) -> Vec<u8> {
        let r: LintReq = match parse(req, r#"{"text": ...}"#) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let opt = SchemaOptions {
            list: true,
            ..SchemaOptions::default()
        };
        match lint_document(r.text.as_bytes(), None, &opt) {
            Ok(res) => json::to_string(&res).into_bytes(),
            Err(e) => error(e),
        }
    }

    #[derive(Deserialize)]
    struct CheckReq {
        specs: Vec<String>,
        manifest: String,
        #[serde(default)]
        probes: HashMap<String, Probe>,
        now: String,
    }

    #[derive(Serialize)]
    struct CheckResult {
        reports: Vec<Report>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        notes: Vec<String>,
        /// Each report's target to the screens it covers, for drawing proofs.
        requirements: BTreeMap<String, Vec<RequirementsView>>,
    }

    fn try_check(req: &[u8]) -> Result<Vec<u8>, Vec<u8>> {
        let r: CheckReq = parse(
            req,
            r#"{"specs": [...], "manifest": "...", "probes": {...}, "now": "RFC 3339"}"#,
        )?;
        let docs = r
            .specs
            .iter()
            .map(|s| decode_document(s.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        let m = decode_manifest(r.manifest.as_bytes()).map_err(error)?;
        let now = DateTime::parse_from_rfc3339(&r.now).map_err(error)?;
        let (reports, notes) =
            check_manifest(&docs, &m, &r.probes, &Options::new(now)).map_err(error)?;
        let mut requirements = BTreeMap::new();
        for rep in &reports {
            if requirements.contains_key(&rep.target) {
                continue;
            }
            let Ok(rf) = parse_ref(&rep.target) else {
                continue;
            };
            for d in &docs {
                if let Ok(reqs) = d.resolve(&rf) {
                    requirements.insert(
                        rep.target.clone(),
                        reqs.iter().map(|q| q.view()).collect::<Vec<_>>(),
                    );
                    break;
                }
            }
        }
        Ok(json::to_string(&CheckResult {
            reports,
            notes,
            requirements,
        })
        .into_bytes())
    }

    pub fn check(req: &[u8]) -> Vec<u8> {
        try_check(req).unwrap_or_else(|e| e)
    }

    #[derive(Deserialize)]
    struct ResolveReq {
        spec: String,
        #[serde(rename = "ref")]
        reference: String,
    }

    fn try_resolve(req: &[u8]) -> Result<Vec<u8>, Vec<u8>> {
        let r: ResolveReq = parse(req, r#"{"spec": "...", "ref": "..."}"#)?;
        let d = decode_document(r.spec.as_bytes()).map_err(error)?;
        let rf = parse_ref(&r.reference).map_err(error)?;
        let views: Vec<RequirementsView> = d
            .resolve(&rf)
            .map_err(error)?
            .iter()
            .map(|q| q.view())
            .collect();
        Ok(json::to_string(&views).into_bytes())
    }

    pub fn resolve(req: &[u8]) -> Vec<u8> {
        try_resolve(req).unwrap_or_else(|e| e)
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
            let text = String::from_utf8(example("retail.screens.json")).unwrap();
            let req = serde_json::to_vec(&json!({ "text": text })).unwrap();
            let v = val(lint(&req));
            assert_eq!(v["kind"], "screens");
            assert!(v.get("schema_errors").is_none() && v.get("document_checks").is_none());
            assert!(val(lint(br#"{"text":"{}"}"#))["error"].is_string());
            assert!(val(lint(b"nonsense"))["error"].is_string());
        }

        #[test]
        fn check_and_resolve_the_example() {
            let text = |n: &str| String::from_utf8(example(n)).unwrap();
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
            let req = json!({"specs": [text("retail.screens.json")], "manifest": text("creative.json"),
                "probes": probes, "now": "2026-10-22T12:00:00+11:00"});
            let v = val(check(&serde_json::to_vec(&req).unwrap()));
            assert!(v["error"].is_null(), "{v}");
            assert!(v["reports"].as_array().unwrap().len() >= 2);
            let t = "harbourline.example/retail/portrait-totem-75";
            assert_eq!(v["requirements"][t].as_array().unwrap().len(), 2);

            let r = json!({"spec": text("retail.screens.json"), "ref": t});
            let v = val(resolve(&serde_json::to_vec(&r).unwrap()));
            assert_eq!(v.as_array().unwrap().len(), 2);
            let r = json!({"spec": text("retail.screens.json"), "ref": "nonsense"});
            assert!(val(resolve(&serde_json::to_vec(&r).unwrap()))["error"].is_string());
            assert!(val(check(br#"{"specs":[],"manifest":"{}","now":""}"#))["error"].is_string());
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
    op!(opencrs_lint, ops::lint);
    op!(opencrs_check, ops::check);
    op!(opencrs_resolve, ops::resolve);
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
