//! Resolving a reference straight from a document's text, a screen at a
//! time, and writing the answer as JSON with no tree of values in between.
//!
//! Resolving a format of hundreds of thousands of screens the ordinary way
//! decodes the whole document, builds a view for every screen and then the
//! JSON of all of them: several times the document in memory. Here the
//! document's shell (everything but the screens) is decoded, then the
//! screens are read one by one from the text; a screen of the format is
//! grouped by its requirements and only the first of each group has its view
//! built and written out as JSON, once. What is kept is the groups' JSON
//! and, for each screen, its id and group, and the answer is written into a
//! buffer of exactly its size. The answer, and every error, is what
//! [`Document::resolve`] and a view of each result would give.

use crate::check::CODE_TOO_COMPLEX;
use crate::document::{Root, ScreenStream};
use crate::error::Error;
use crate::json;
use crate::load::{check_schema, decode_document};
use crate::model::{Document, Network, Screen};
use crate::resolve::{Ref, ResolveError, RuleCache, parse_ref};
use crate::schema::Kind;
use std::collections::HashMap;

/// A resolved reference: one result for each screen the reference covers,
/// in document order (or one for a format no screen uses), kept as the JSON
/// of each distinct view and each result's view and screen id.
pub struct Resolved {
    /// Each distinct view's JSON, as Go writes it, with no `screen` member.
    groups: Vec<Vec<u8>>,
    /// For each result, its view and its screen's id; None for the result
    /// of a format with no screens.
    members: Vec<(u32, Option<String>)>,
}

impl Resolved {
    /// Resolves `reference` in the document `raw`. The errors are those of
    /// [`decode_document`], then [`parse_ref`], then [`Document::resolve`];
    /// a budget that runs out is `CRS-TOO-COMPLEX: ` and the reason.
    pub fn from_text(raw: &[u8], reference: &str) -> Result<Resolved, Error> {
        check_schema(Kind::Screens, raw)?;
        // A document that can't be read a screen at a time (not UTF-8, a
        // repeated key, a screen the model refuses) is resolved the
        // ordinary way, which gives the exact error, or the same answer.
        let Ok(text) = std::str::from_utf8(raw) else {
            return Self::whole(raw, reference);
        };
        let Some(root) = serde_json::from_str::<Root>(text).ok() else {
            return Self::whole(raw, reference);
        };
        let Some(head) = root.head() else {
            return Self::whole(raw, reference);
        };
        let Ok(mut doc) = serde_json::from_str::<Document>(&head) else {
            return Self::whole(raw, reference);
        };
        let mut lists = Vec::with_capacity(root.networks.len());
        for net in &root.networks {
            let Some((head, screens)) = net.without("screens", "[]") else {
                return Self::whole(raw, reference);
            };
            let (Ok(shell), Some(screens)) = (serde_json::from_str::<Network>(&head), screens)
            else {
                return Self::whole(raw, reference);
            };
            doc.networks.push(shell);
            lists.push(screens);
        }
        doc.size = raw.len();

        // The reference is parsed now but judged after the screens are
        // read, as a document that doesn't decode is the first error.
        let r = parse_ref(reference);
        let mut aim = r.as_ref().ok().and_then(|r| Aim::new(&doc, r).ok());
        let mut out = Resolved {
            groups: Vec::new(),
            members: Vec::new(),
        };
        let mut cache = RuleCache::default();
        let mut by_key: HashMap<Vec<u8>, u32> = HashMap::new();
        let mut key = Vec::new();
        let mut failed: Option<ResolveError> = None;
        for (ni, list) in lists.iter().enumerate() {
            let mut de = serde_json::Deserializer::from_str(list.get());
            let mut each = |s: Screen| {
                let Some(a) = aim.as_mut().filter(|a| a.network == ni && !a.done) else {
                    return true;
                };
                let n = &doc.networks[ni];
                match a.what {
                    What::Screen(ref id) => {
                        if s.id != *id {
                            return true;
                        }
                        a.done = true;
                        match doc.resolve_screen_in(&mut cache, n, &s, None) {
                            Ok(req) => out.push(&req, &mut by_key, &mut key),
                            Err(e) => failed = Some(e),
                        }
                    }
                    What::Format(fi) => {
                        let f = &n.formats[fi];
                        if s.format != f.id {
                            return true;
                        }
                        // Each reference to a format walks its screens.
                        if !crate::budget::charge(
                            crate::budget::Work::Screens,
                            f as *const _ as usize,
                            8,
                        ) {
                            a.done = true;
                            return true;
                        }
                        match doc.resolve_screen_in(&mut cache, n, &s, Some(f)) {
                            Ok(req) => {
                                a.any = true;
                                out.push(&req, &mut by_key, &mut key);
                            }
                            Err(e) => {
                                failed = Some(e);
                                a.done = true;
                            }
                        }
                    }
                }
                true
            };
            let read = serde::de::DeserializeSeed::deserialize(
                ScreenStream::<Screen>::new(&mut each),
                &mut de,
            );
            if read.is_err() || de.end().is_err() {
                return Self::whole(raw, reference);
            }
        }
        let r = r?;
        let a = Aim::new(&doc, &r)?;
        if let Some(e) = failed {
            return Err(e.into());
        }
        let n = &doc.networks[a.network];
        match a.what {
            What::Screen(ref id) if !aim.as_ref().is_some_and(|a| a.done) => {
                return Err(ResolveError::Other(format!(
                    "screen {} not found in network {}",
                    crate::gofmt::quote(id),
                    crate::gofmt::quote(&n.id)
                ))
                .into());
            }
            What::Format(fi)
                if !aim.as_ref().is_some_and(|a| a.any) && !crate::budget::tripped() =>
            {
                let req = doc.build_with(&mut cache, n, &n.formats[fi], None, None);
                out.push(&req, &mut by_key, &mut key);
            }
            _ => {}
        }
        if let Some(why) = crate::budget::explain(Some(&doc)) {
            return Err(Error::Message(format!("{CODE_TOO_COMPLEX}: {why}")));
        }
        Ok(out)
    }

    /// The ordinary way: the whole document decoded and every screen's view
    /// built. For documents that can't be streamed, so an error is the one
    /// decoding gives.
    fn whole(raw: &[u8], reference: &str) -> Result<Resolved, Error> {
        let doc = decode_document(raw)?;
        let r = parse_ref(reference)?;
        let mut out = Resolved {
            groups: Vec::new(),
            members: Vec::new(),
        };
        let mut by_key = HashMap::new();
        let mut key = Vec::new();
        doc.resolve_each(&r, |req| {
            if !crate::budget::tripped() {
                out.push(&req, &mut by_key, &mut key);
            }
        })?;
        if let Some(why) = crate::budget::explain(Some(&doc)) {
            return Err(Error::Message(format!("{CODE_TOO_COMPLEX}: {why}")));
        }
        Ok(out)
    }

    /// Adds one result: its view is written as JSON if no screen of its kind
    /// has been seen, and the screen is listed under it. Writing each
    /// result is counted as work, so an answer far larger than the document
    /// (a format of many regions on many screens) is stopped.
    fn push(
        &mut self,
        req: &crate::resolve::Requirements,
        by_key: &mut HashMap<Vec<u8>, u32>,
        key: &mut Vec<u8>,
    ) {
        if crate::budget::tripped() {
            return;
        }
        key.clear();
        let keyed = req.key(key);
        let g = match by_key.get(key.as_slice()) {
            Some(&g) if keyed => g,
            _ => {
                let mut view = req.view();
                view.screen.clear();
                self.groups.push(json::to_string(&view).into_bytes());
                let g = (self.groups.len() - 1) as u32;
                if keyed {
                    by_key.insert(key.clone(), g);
                }
                g
            }
        };
        crate::budget::charge(
            crate::budget::Work::Views,
            req.network as *const _ as usize,
            (self.groups[g as usize].len() / 16 + 1) as u64,
        );
        self.members.push((g, req.screen.map(|s| s.id.clone())));
    }

    /// How many results there are.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The results as a JSON list of views, one for each screen with its
    /// `screen` set: what `json::to_string` of the views would give, in a
    /// buffer of exactly that size.
    pub fn per_screen(&self) -> Vec<u8> {
        let prefix = |id: &str| {
            // `{"screen":"<id>"` for a screen with an id; a screen with no id
            // has no screen member, as the view leaves an empty one out.
            if id.is_empty() {
                None
            } else {
                let mut s = String::from("{\"screen\":");
                s.push_str(&json::to_string(id));
                s.push(',');
                Some(s)
            }
        };
        let mut total = 2 + self.members.len().saturating_sub(1);
        for (g, id) in &self.members {
            let group = &self.groups[*g as usize];
            total += match id.as_deref().and_then(prefix) {
                Some(p) => p.len() + group.len() - 1,
                None => group.len(),
            };
        }
        let mut out = Vec::with_capacity(total);
        out.push(b'[');
        for (i, (g, id)) in self.members.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            let group = &self.groups[*g as usize];
            match id.as_deref().and_then(prefix) {
                Some(p) => {
                    out.extend_from_slice(p.as_bytes());
                    out.extend_from_slice(&group[1..]);
                }
                None => out.extend_from_slice(group),
            }
        }
        out.push(b']');
        debug_assert_eq!(out.len(), total);
        out
    }

    /// The results as `{"views": [...], "group": [...], "id": [...]}`: the
    /// distinct views, and for each result in order which view it has and
    /// its screen's id (null for a format with no screens). A caller that
    /// wants one entry per screen puts them together, which keeps a very
    /// large answer small on the way.
    pub fn grouped(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"{\"views\":[");
        for (i, g) in self.groups.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            out.extend_from_slice(g);
        }
        out.extend_from_slice(b"],\"group\":[");
        for (i, (g, _)) in self.members.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            out.extend_from_slice(g.to_string().as_bytes());
        }
        out.extend_from_slice(b"],\"id\":[");
        for (i, (_, id)) in self.members.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            match id {
                Some(id) => out.extend_from_slice(json::to_string(id).as_bytes()),
                None => out.extend_from_slice(b"null"),
            }
        }
        out.extend_from_slice(b"]}");
        out
    }
}

/// What a reference asks of a document's shell: which network, and a
/// screen's id or a format's position in it.
struct Aim {
    network: usize,
    what: What,
    /// Nothing more to resolve: the screen was found, or the work stopped.
    done: bool,
    /// A screen of the format was found.
    any: bool,
}

enum What {
    Screen(String),
    Format(usize),
}

impl Aim {
    /// Where `r` points in `doc`, or why it doesn't resolve, worded as
    /// [`Document::resolve`] words it.
    fn new(doc: &Document, r: &Ref) -> Result<Aim, ResolveError> {
        use crate::gofmt::quote;
        if r.publisher != doc.publisher {
            return Err(ResolveError::Other(format!(
                "{r} belongs to {}, not {}",
                r.publisher, doc.publisher
            )));
        }
        let (ni, n) = doc
            .networks
            .iter()
            .enumerate()
            .find(|(_, n)| n.id == r.network)
            .ok_or_else(|| {
                ResolveError::Other(format!(
                    "network {} not found in {}",
                    quote(&r.network),
                    doc.publisher
                ))
            })?;
        let what = if !r.screen.is_empty() {
            What::Screen(r.screen.clone())
        } else {
            let f = n.format(&r.format).ok_or_else(|| {
                ResolveError::Other(format!(
                    "format {} not found in network {}",
                    quote(&r.format),
                    quote(&n.id)
                ))
            })?;
            What::Format(crate::resolve::position_of(&n.formats, f).expect("the network's"))
        };
        Ok(Aim {
            network: ni,
            what,
            done: false,
            any: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;
    use serde_json::{Value, json};

    fn old_way(raw: &[u8], reference: &str) -> Result<String, String> {
        let d = decode_document(raw).map_err(|e| e.to_string())?;
        let r = parse_ref(reference).map_err(|e| e.to_string())?;
        let views: Vec<_> = d
            .resolve(&r)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|q| q.view())
            .collect();
        Ok(json::to_string(&views))
    }

    fn new_way(raw: &[u8], reference: &str) -> Result<String, String> {
        Resolved::from_text(raw, reference)
            .map(|r| String::from_utf8(r.per_screen()).unwrap())
            .map_err(|e| e.to_string())
    }

    /// The streamed answer, byte for byte, and every error, are the
    /// ordinary way's, over documents with repeated and empty ids,
    /// overrides, own rules, a format no screen uses and broken references.
    #[test]
    fn streamed_resolve_matches_resolving_the_decoded_document() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let base: Value = serde_json::from_slice(
            &std::fs::read(root.join("examples/harbourline/retail.screens.json")).unwrap(),
        )
        .unwrap();
        let mut docs = vec![base.clone()];
        let mut with = |f: &dyn Fn(&mut Value)| {
            let mut v = base.clone();
            f(&mut v);
            docs.push(v);
        };
        with(&|v| {
            let s = v["networks"][0]["screens"].as_array().unwrap().clone();
            let mut all = Vec::new();
            for i in 0..30 {
                let mut x = s[i % 3].clone();
                x["id"] = format!("X{}", i % 20).into(); // repeats
                if i % 4 == 0 {
                    x["overrides"] = json!({"media": {"static": []}, "pixels": [100, 100 + i]});
                }
                if i % 5 == 0 {
                    x["rules"] = json!({"max_words": 3, "required_elements": [
                        {"role": "legal", "when": "offer", "within_rect_px": [0, 0, 10, 10]}]});
                }
                if i == 7 {
                    x["id"] = "".into();
                }
                all.push(x);
            }
            v["networks"][0]["screens"] = all.into();
        });
        with(&|v| v["networks"][0]["screens"] = json!([]));
        with(&|v| v["networks"][0]["screens"][0]["format"] = "gone".into());
        with(&|v| v["networks"][0]["screens"][1]["venue"] = "nowhere".into());
        with(&|v| {
            let n = v["networks"][0].clone();
            let mut m = n.clone();
            m["id"] = "second".into();
            v["networks"] = json!([n, m]);
        });
        with(&|v| v["networks"][0]["screens"][0]["pixels"] = "typed badly".into());
        let refs = [
            "harbourline.example/retail/portrait-totem-75",
            "harbourline.example/retail/atrium-wrap",
            "harbourline.example/retail/screen/HC-L1-002",
            "harbourline.example/retail/screen/X3",
            "harbourline.example/retail/screen/",
            "harbourline.example/retail/nope",
            "harbourline.example/second/portrait-totem-75",
            "harbourline.example/nope/portrait-totem-75",
            "other.example/retail/portrait-totem-75",
            "not a reference",
        ];
        for (i, v) in docs.iter().enumerate() {
            let text = serde_json::to_vec(v).unwrap();
            for r in refs {
                assert_eq!(new_way(&text, r), old_way(&text, r), "document {i}, {r}");
            }
        }
        // Text the model refuses (a repeated key) gives decoding's error.
        let dup = r#"{"opencrs":"0.1","publisher":"a.example","updated_at":"2026-01-01T00:00:00Z",
            "networks":[{"id":"n","name":"N","formats":[{"id":"f","pixels":[1,1],"media":{"static":["image/png"]}}],
            "screens":[{"id":"s","format":"f","id":"t"}]}]}"#;
        assert_eq!(
            new_way(dup.as_bytes(), "a.example/n/f"),
            old_way(dup.as_bytes(), "a.example/n/f")
        );
        assert!(
            new_way(dup.as_bytes(), "a.example/n/f")
                .unwrap_err()
                .contains("duplicate")
        );
        assert_eq!(
            new_way(b"[]", "a.example/n/f"),
            old_way(b"[]", "a.example/n/f")
        );
    }

    /// The grouped form puts back together to the per-screen one.
    #[test]
    fn grouped_form_expands_to_the_per_screen_form() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let raw = std::fs::read(root.join("examples/harbourline/retail.screens.json")).unwrap();
        let r = Resolved::from_text(&raw, "harbourline.example/retail/portrait-totem-75").unwrap();
        let g: Value = serde_json::from_slice(&r.grouped()).unwrap();
        let views = g["views"].as_array().unwrap();
        let mut expanded = Vec::new();
        for (gi, id) in g["group"]
            .as_array()
            .unwrap()
            .iter()
            .zip(g["id"].as_array().unwrap())
        {
            let mut v = views[gi.as_u64().unwrap() as usize].clone();
            if let Some(id) = id.as_str() {
                v["screen"] = id.into();
            }
            expanded.push(v);
        }
        let per: Value = serde_json::from_slice(&r.per_screen()).unwrap();
        assert_eq!(Value::Array(expanded), per);
        assert_eq!(r.len(), 2);
    }
}
