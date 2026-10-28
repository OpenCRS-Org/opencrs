//! A deterministic work budget.
//!
//! The core counts units of work as it goes (rules merged, rule items
//! checked, findings and screen entries written, views built) and allows a
//! fixed multiple of the input's size. A document built to make the checks do
//! far more than its size warrants runs out of budget, and the operation
//! stops and says so with a `CRS-TOO-COMPLEX` finding that names where the
//! work came from. Only work is counted, never time, so every runtime
//! (native, WebAssembly, any number of threads) gives the same answer.
//!
//! The finding is a safeguard of this validator, not a verdict on the
//! document: a document that trips it is too complex for the validator, and
//! may well be valid.

use crate::model::{Document, Network, RuleSet};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Units of work allowed for each byte of input.
pub const UNITS_PER_BYTE: u64 = 16;

/// Units allowed on top, so a tiny document can still be checked fully.
pub const BASE_UNITS: u64 = 1 << 20;

/// What a unit of work was spent on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub(crate) enum Work {
    /// Rule lists merged into the rules of a level.
    RuleMerge,
    /// Inherited rule items checked against a creative.
    RuleCheck,
    /// Rule pack references looked up.
    Packs,
    /// Findings written.
    Findings,
    /// Screen views built.
    Views,
    /// Screens visited to answer a reference.
    Screens,
    /// Kinds of screen folded into strictest requirements.
    Strictest,
    /// A format's blocked regions rebuilt for a screen whose overrides
    /// change them.
    Overrides,
}

struct State {
    limit: u64,
    bytes: u64,
    used: AtomicU64,
    tripped: AtomicBool,
    /// Per (kind, where it came from): units, and how many times work of that
    /// kind was done there. Updated once per piece of work, not per unit.
    by: Mutex<HashMap<(Work, usize), (u64, u64)>>,
    explanation: Mutex<Option<String>>,
}

/// The work an operation may do. Cloning shares the count.
#[derive(Clone)]
pub struct Budget(Arc<State>);

thread_local! {
    static CURRENT: RefCell<Option<Budget>> = const { RefCell::new(None) };
}

/// Makes a budget current for this thread until dropped.
pub struct Guard(Option<Budget>);

impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|c| *c = self.0.take());
    }
}

impl Budget {
    /// A budget for an input of `bytes` bytes.
    pub fn for_input(bytes: usize) -> Budget {
        Budget(Arc::new(State {
            limit: BASE_UNITS + UNITS_PER_BYTE * bytes as u64,
            bytes: bytes as u64,
            used: AtomicU64::new(0),
            tripped: AtomicBool::new(false),
            by: Mutex::new(HashMap::new()),
            explanation: Mutex::new(None),
        }))
    }

    /// Makes this the current budget on this thread (as work is counted
    /// against the current one) until the guard is dropped.
    pub fn install(&self) -> Guard {
        Guard(CURRENT.with_borrow_mut(|c| c.replace(self.clone())))
    }

    pub fn used(&self) -> u64 {
        self.0.used.load(Ordering::Relaxed)
    }

    pub fn limit(&self) -> u64 {
        self.0.limit
    }

    pub fn tripped(&self) -> bool {
        self.0.tripped.load(Ordering::Relaxed)
    }
}

/// Runs `f` against the current budget, or one for `bytes` of input if there
/// isn't one, and says what a new one counted.
pub fn ensure<R>(bytes: usize, f: impl FnOnce() -> R) -> R {
    if CURRENT.with_borrow(|c| c.is_some()) {
        return f();
    }
    let budget = Budget::for_input(bytes);
    let _guard = budget.install();
    f()
}

/// Runs `f` with no budget current, for work already counted.
pub(crate) fn unmetered<R>(f: impl FnOnce() -> R) -> R {
    let saved = CURRENT.with_borrow_mut(|c| c.take());
    let r = f();
    CURRENT.with_borrow_mut(|c| *c = saved);
    r
}

/// Runs `f` with a new budget for `bytes` of input and returns the budget
/// beside its result, so tests and callers can see how much work was done.
pub fn measure<R>(bytes: usize, f: impl FnOnce() -> R) -> (R, Budget) {
    let budget = Budget::for_input(bytes);
    let guard = budget.install();
    let r = f();
    drop(guard);
    (r, budget)
}

/// The current budget, if any.
pub fn current() -> Option<Budget> {
    CURRENT.with_borrow(|c| c.clone())
}

/// Counts `units` of `work` that came from `origin` (the address of the
/// document part responsible). False once the budget is spent: the caller
/// should stop.
pub(crate) fn charge(work: Work, origin: usize, units: u64) -> bool {
    CURRENT.with_borrow(|c| {
        let Some(b) = c else { return true };
        let s = &b.0;
        if let Ok(mut by) = s.by.lock() {
            let e = by.entry((work, origin)).or_default();
            e.0 += units;
            e.1 += 1;
        }
        let used = s.used.fetch_add(units, Ordering::Relaxed) + units;
        if used > s.limit {
            s.tripped.store(true, Ordering::Relaxed);
        }
        !s.tripped.load(Ordering::Relaxed)
    })
}

/// Counts work with nothing to attribute it to.
pub(crate) fn charge_flat(units: u64) -> bool {
    CURRENT.with_borrow(|c| {
        let Some(b) = c else { return true };
        let used = b.0.used.fetch_add(units, Ordering::Relaxed) + units;
        if used > b.0.limit {
            b.0.tripped.store(true, Ordering::Relaxed);
        }
        !b.0.tripped.load(Ordering::Relaxed)
    })
}

/// Whether the current budget is spent, so a caller building something for
/// each screen (such as views) should stop and ask [`explain`] why.
pub fn tripped() -> bool {
    CURRENT.with_borrow(|c| c.as_ref().is_some_and(Budget::tripped))
}

/// Why the current budget ran out, in plain words, worked out once with the
/// document at hand. None if it hasn't.
pub fn explain(doc: Option<&Document>) -> Option<String> {
    let b = current()?;
    if !b.tripped() {
        return None;
    }
    let mut cache = b.0.explanation.lock().ok()?;
    if let Some(e) = cache.as_ref()
        && (doc.is_none() || !e.ends_with(FALLBACK_TAIL))
    {
        return Some(e.clone());
    }
    let text = render(&b, doc);
    *cache = Some(text.clone());
    Some(text)
}

const FALLBACK_TAIL: &str = "(Where it came from isn't known here.)";

fn render(b: &Budget, doc: Option<&Document>) -> String {
    let s = &b.0;
    let mut rows: Vec<(u64, u64, String, String)> = Vec::new();
    if let Ok(by) = s.by.lock() {
        for (&(work, origin), &(units, calls)) in by.iter() {
            let (sentence, key) = describe(work, origin, units, calls, doc);
            rows.push((units, calls, key, sentence));
        }
    }
    // Biggest first; equal sizes in a fixed order, so the text doesn't depend
    // on addresses.
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.cmp(&b.2)));
    let mut text = format!(
        "This document is more work than this validator will do for one this size (about {} units of work needed, {} allowed for {} bytes); that makes it too complex to check in full here, not invalid.",
        num(s.used.load(Ordering::Relaxed)),
        num(s.limit),
        num(s.bytes),
    );
    for (_, _, _, sentence) in rows.iter().take(2) {
        text.push(' ');
        text.push_str(sentence);
    }
    if rows.is_empty() || doc.is_none() {
        text.push(' ');
        text.push_str(FALLBACK_TAIL);
    }
    text
}

/// A sentence for one source of work, and a sort key that names it.
fn describe(
    work: Work,
    origin: usize,
    units: u64,
    calls: u64,
    doc: Option<&Document>,
) -> (String, String) {
    let found = doc.and_then(|d| locate(d, origin));
    let (label, path, lists, net) = match &found {
        Some(f) => (
            f.label.clone(),
            f.path.clone(),
            f.rules.map(list_summary),
            f.net,
        ),
        None => ("the document".to_string(), String::new(), None, None),
    };
    let at = |p: &str| {
        if p.is_empty() {
            String::new()
        } else {
            format!(" ({p})")
        }
    };
    let sentence = match work {
        Work::RuleMerge => format!(
            "{label}{} has {} to merge into the rules of the venues, formats and screens below it ({} merges, {} units).",
            at(&path),
            lists.map_or_else(|| "rule lists".to_string(), |(t, _)| t),
            num(calls),
            num(units)
        ),
        Work::RuleCheck => {
            let (lists, lp) = net
                .and_then(|n| biggest_lists(n, doc?))
                .unwrap_or_else(|| ("rule lists".to_string(), String::new()));
            format!(
                "{label}: {lists}{} are inherited by its screens and checked again for each of at least {} different kinds of screen ({} units in all).",
                at(&lp),
                num(calls),
                num(units)
            )
        }
        Work::Packs => format!(
            "{label}{} lists {} rule pack references ({} units).",
            at(&path),
            num(units),
            num(units)
        ),
        Work::Findings => format!(
            "About {} findings were written{}.",
            num(units / 256),
            at(&path)
        ),
        Work::Screens => format!(
            "The screens of {label}{} were gone through {} times over to answer references to them ({} units).",
            at(&path),
            num(calls),
            num(units)
        ),
        Work::Views => format!(
            "{} screen views were built for {label}{} ({} units).",
            num(calls),
            at(&path),
            num(units)
        ),
        Work::Strictest => format!(
            "{} kinds of screen were combined into the strictest requirements for {label}{} ({} units).",
            num(calls),
            at(&path),
            num(units)
        ),
        Work::Overrides => format!(
            "The blocked regions of {label}{} were rebuilt for each of {} screens whose overrides change them ({} units).",
            at(&path),
            num(calls),
            num(units)
        ),
    };
    let key = format!("{work:?} {path} {label}");
    (sentence, key)
}

struct Found<'a> {
    label: String,
    path: String,
    rules: Option<&'a RuleSet>,
    net: Option<&'a Network>,
}

/// Finds the part of `doc` at address `origin`, with where it is.
fn locate<'d>(doc: &'d Document, origin: usize) -> Option<Found<'d>> {
    let addr = |p: *const ()| p as usize;
    for (ni, n) in doc.networks.iter().enumerate() {
        let np = format!("/networks/{ni}");
        let name = format!("network `{}`", n.id);
        let hit =
            |p: usize, label: String, path: String, r: Option<&'d RuleSet>| -> Option<Found<'d>> {
                (p == origin).then_some(Found {
                    label,
                    path,
                    rules: r,
                    net: Some(n),
                })
            };
        if let Some(f) = hit(
            addr(n as *const Network as *const ()),
            name.clone(),
            np.clone(),
            n.rules.as_ref(),
        ) {
            return Some(f);
        }
        if let Some(r) = &n.rules
            && let Some(f) = hit(
                addr(r as *const RuleSet as *const ()),
                name.clone(),
                format!("{np}/rules"),
                Some(r),
            )
        {
            return Some(f);
        }
        for (i, v) in n.venues.iter().enumerate() {
            let (label, path) = (format!("venue `{}`", v.id), format!("{np}/venues/{i}"));
            if let Some(r) = &v.rules
                && let Some(f) = hit(
                    addr(r as *const RuleSet as *const ()),
                    label.clone(),
                    format!("{path}/rules"),
                    Some(r),
                )
            {
                return Some(f);
            }
        }
        for (i, f) in n.formats.iter().enumerate() {
            let (label, path) = (format!("format `{}`", f.id), format!("{np}/formats/{i}"));
            if addr(f as *const _ as *const ()) == origin {
                return Some(Found {
                    label,
                    path,
                    rules: f.fields.rules.as_ref(),
                    net: Some(n),
                });
            }
            if let Some(r) = &f.fields.rules
                && let Some(h) = hit(
                    addr(r as *const RuleSet as *const ()),
                    label.clone(),
                    format!("{path}/rules"),
                    Some(r),
                )
            {
                return Some(h);
            }
        }
        for (i, s) in n.screens.iter().enumerate() {
            let (label, path) = (format!("screen `{}`", s.id), format!("{np}/screens/{i}"));
            if let Some(r) = s.rules.as_deref()
                && let Some(h) = hit(
                    addr(r as *const RuleSet as *const ()),
                    label.clone(),
                    format!("{path}/rules"),
                    Some(r),
                )
            {
                return Some(h);
            }
            if let Some(r) = s.overrides.as_ref().and_then(|o| o.fields.rules.as_ref())
                && let Some(h) = hit(
                    addr(r as *const RuleSet as *const ()),
                    label.clone(),
                    format!("{path}/overrides/rules"),
                    Some(r),
                )
            {
                return Some(h);
            }
        }
    }
    None
}

/// "2,000 required elements" for the biggest list of a rule set, and which.
fn list_summary(r: &RuleSet) -> (String, &'static str) {
    let lists = [
        (
            r.required_elements.len(),
            "required elements",
            "required_elements",
        ),
        (r.rule_packs.len(), "rule packs", "rule_packs"),
        (
            r.restricted_categories.len(),
            "restricted categories",
            "restricted_categories",
        ),
    ];
    let (n, what, field) = lists
        .iter()
        .max_by_key(|l| l.0)
        .copied()
        .expect("three lists");
    (format!("{} {what}", num(n as u64)), field)
}

/// The biggest rule list among a network's own rules and its venues' and
/// formats', with its path.
fn biggest_lists(n: &Network, doc: &Document) -> Option<(String, String)> {
    let ni = doc.networks.iter().position(|x| std::ptr::eq(x, n))?;
    let np = format!("/networks/{ni}");
    let mut best: Option<(usize, String, String)> = None;
    let mut see = |r: Option<&RuleSet>, path: String| {
        if let Some(r) = r {
            let (text, field) = list_summary(r);
            let size = [
                r.required_elements.len(),
                r.rule_packs.len(),
                r.restricted_categories.len(),
            ]
            .into_iter()
            .max()
            .unwrap_or(0);
            if best.as_ref().is_none_or(|b| size > b.0) {
                best = Some((size, text, format!("{path}/{field}")));
            }
        }
    };
    see(n.rules.as_ref(), format!("{np}/rules"));
    for (i, v) in n.venues.iter().enumerate() {
        see(v.rules.as_ref(), format!("{np}/venues/{i}/rules"));
    }
    for (i, f) in n.formats.iter().enumerate() {
        see(f.fields.rules.as_ref(), format!("{np}/formats/{i}/rules"));
    }
    best.map(|(_, t, p)| (t, p))
}

/// 1234567 as "1,234,567".
fn num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
