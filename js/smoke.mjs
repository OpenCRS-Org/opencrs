// Runs the WebAssembly build in Node and checks it against the CLI's
// examples. From the repo root:
//
//   scripts/build-wasm.sh /tmp/wasm
//   node js/smoke.mjs /tmp/wasm
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";
import assert from "node:assert/strict";

// TODO(task-9 phase 2): flip to false once the core's `validate` is wired
// into opencrs_check and opencrs_resolve. While true, those two calls are
// only required to answer with a JSON error.
const CHECK_PENDING = false;

const outDir = process.argv[2];
if (!outDir) throw new Error("usage: node smoke.mjs OUT_DIR");
const wasmPath = `${outDir.replace(/\/$/, "")}/opencrs.wasm`;
const examples = new URL("../examples/harbourline/", import.meta.url);

// The module takes and gives nothing but numbers: no imports at all.
const wasmBytes = await readFile(wasmPath);
const compiled = new WebAssembly.Module(wasmBytes);
assert.equal(WebAssembly.Module.imports(compiled).length, 0, "the module must have no imports");
const exported = WebAssembly.Module.exports(compiled).map((e) => e.name).sort();
for (const name of ["memory", "opencrs_alloc", "opencrs_free", "opencrs_version", "opencrs_lint",
  "opencrs_check", "opencrs_resolve", "opencrs_probe_start", "opencrs_probe_feed",
  "opencrs_legibility", "opencrs_dwell", "opencrs_rule_packs", "opencrs_publisher"]) {
  assert.ok(exported.includes(name), `missing export ${name}`);
}

const { load } = await import(pathToFileURL(`${outDir.replace(/\/$/, "")}/opencrs.js`));
const api = await load(wasmBytes);

const text = (name) => readFile(new URL(name, examples), "utf8");
const bytes = async (name) => new Uint8Array(await readFile(new URL(name, examples)));
const screens = await text("retail.screens.json");
const creative = await text("creative.json");

assert.equal(api.version, "0.1");
const lint = JSON.parse(api.lint(screens));
assert.equal(lint.kind, "screens");
assert.ok(!lint.schema_errors && !lint.document_checks, "the example spec should lint clean");
assert.ok(JSON.parse(api.lint("{}")).error, "an unrecognisable document should be an error");

const files = {
  "lunch-portrait.png": await bytes("lunch-portrait.png"),
  "lunch-portrait-logo-right.png": await bytes("lunch-portrait-logo-right.png"),
};

// Probing: the session answers range requests, and hashing happens outside.
for (const [name, data] of Object.entries(files)) {
  const p = await api.probe(data);
  assert.equal(p.sha256, createHash("sha256").update(data).digest("hex"), name);
  assert.equal(p.bytes, data.length);
  assert.equal(p.media_type, "image/png");
  assert.equal(p.pixels.length, 2);
}

// A 50 MB file probes by range without the module's memory growing past 16 MB.
{
  const big = new Uint8Array(50 * 1024 * 1024);
  big.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0);
  const before = api.memoryBytes();
  const p = await api.probe(big);
  assert.equal(p.bytes, big.length);
  const after = api.memoryBytes();
  assert.ok(after <= 16 * 1024 * 1024, `memory grew to ${after} (from ${before})`);
}

// A document at the spec's 50 MB limit (190,000 screens, all on one format
// that carries one finding, reported once) lints within 128 MiB and checks within 256 MiB,
// and a target covering every screen answers in seconds, not minutes.
{
  const rows = [];
  for (let i = 0; i < 190000; i++) {
    rows.push(`{"id":"S-${String(i).padStart(7, "0")}","format":"fmt-0","venue":"venue-${i % 500}","placement":{"environment":"indoor","level":"L1","zone":"food court area number ${i % 97}"},"viewing":{"distance_m":[2,6],"dwell_s":4,"approach":"head-on","light":"indoor"},"platform_ids":{"cms-a":"frame-${i}"}}`);
  }
  const venues = Array.from({ length: 500 }, (_, i) => ({ id: `venue-${i}`, name: `Venue ${i}` }));
  const format = { id: "fmt-0", pixels: [1080, 1920], media: { static: ["image/png"] }, rules: { rule_packs: ["nope@1"] } };
  const big = `{"opencrs":"0.1","publisher":"big.example","updated_at":"2026-10-20T09:00:00+11:00","networks":[{"id":"net","name":"Big","venues":${JSON.stringify(venues)},"formats":[${JSON.stringify(format)}],"screens":[${rows.join(",")}]}]}`;
  assert.ok(big.length > 40 * 1024 * 1024 && big.length < 50 * 1024 * 1024, `document is ${big.length} bytes`);
  const manifest = JSON.stringify({ opencrs: "0.1", files: [{ file: "a.png", media_type: "image/png", pixels: [1080, 1920], targets: ["big.example/net/fmt-0"] }] });

  const linter = await load(wasmBytes);
  const lintBig = JSON.parse(linter.lint(big));
  assert.equal(lintBig.document_checks.length, 1);
  // The pack is in the format's rules, so it is reported once, for the format.
  assert.equal(lintBig.document_checks[0].format, "fmt-0");
  assert.equal(lintBig.document_checks[0].screens, undefined);
  assert.ok(linter.memoryBytes() <= 128 * 1024 * 1024, `lint used ${linter.memoryBytes()} bytes`);

  const checker = await load(wasmBytes);
  const started = Date.now();
  const checkBig = JSON.parse(await checker.check([big], manifest, {}, "2026-10-28T00:00:00Z"));
  assert.ok(!checkBig.error, checkBig.error);
  // Screens that differ only by id share an entry; these differ by venue.
  const groups = checkBig.requirements["big.example/net/fmt-0"];
  assert.equal(groups.length, 500);
  assert.equal(groups.reduce((n, g) => n + g.screens.length, 0), 190000);
  assert.ok(JSON.stringify(checkBig.requirements).length < 8 * 1024 * 1024, "the answer should be compact");
  assert.ok(Date.now() - started < 60_000, `check took ${Date.now() - started} ms`);
  assert.ok(checker.memoryBytes() <= 256 * 1024 * 1024, `check used ${checker.memoryBytes()} bytes`);
}

// A request more than the module could hold is refused before it is copied.
{
  const huge = new Uint8Array(256 * 1024 * 1024);
  assert.throws(() => api.lint(huge), /memory limit/);
  assert.match(JSON.parse(await api.check([huge], "{}", {}, "2026-10-28T00:00:00Z")).error, /memory limit/);
}

// After a trap the loader discards the instance and the next call works.
assert.throws(() => api._trap(), WebAssembly.RuntimeError);
assert.equal(JSON.parse(api.lint(screens)).kind, "screens", "a fresh instance after a trap");
assert.equal((await api.probe(files["lunch-portrait.png"])).media_type, "image/png");

// Any exception from a call drops the instance, not only a trap: a stack
// overflow surfaces as a RangeError.
{
  const made = api._instances();
  assert.throws(() => api._throw(new RangeError("Maximum call stack size exceeded")), RangeError);
  assert.equal(JSON.parse(api.lint(screens)).kind, "screens");
  assert.equal(api._instances(), made + 1, "a RangeError must reset the instance");
}

const check = JSON.parse(await api.check([screens], creative, files, "2026-10-22T12:00:00+11:00"));
const resolved = JSON.parse(api.resolve(screens, "harbourline.example/retail/portrait-totem-75"));
if (CHECK_PENDING) {
  assert.ok(check.error, "check is a stub until the core's validate lands");
  assert.ok(resolved.error, "resolve is a stub until the core's validate lands");
} else {
  assert.ok(!check.error, check.error);
  // reports.json was written by: opencrs check -json -now 2026-10-22T12:00:00+11:00 ...
  const expected = JSON.parse(await text("reports.json"));
  const got = check.reports.map((r) => ({ ...r }));
  for (const r of [...got, ...expected]) delete r.validator;
  assert.deepEqual(got, expected, "browser and CLI reports differ");
  const totem = check.requirements["harbourline.example/retail/portrait-totem-75"];
  assert.deepEqual(totem.map((g) => g.screens), [["HC-L1-001"], ["HC-L1-002"]]);
  assert.equal(totem[0].screen, undefined, "a group names its screens in `screens`");
  assert.ok(Array.isArray(resolved) && resolved.length === 2);
}

// The calculators give the spec's worked examples.
{
  const l = JSON.parse(api.legibility({ distance_m: [2, 6], screen: { width_mm: 940, width_px: 1080 } }));
  assert.equal(l.key.px, 46);
  const d = JSON.parse(api.dwell({ speed_kmh: 80, distance_m: [20, 150] }));
  assert.equal(d.glance.read_from_m, 64.44);
  assert.equal(d.key.mm, 429.6);
  assert.equal(JSON.parse(api.rulePacks()).rule_packs.length, 6);
}

// The publisher check runs discovery over pages fetched elsewhere.
{
  const index = await text(".well-known/opencrs.json");
  const ok = { status: 200, etag: '"1"', cache_control: "max-age=3600", content_type: "application/json", allow_origin: "*" };
  const pages = {
    "https://harbourline.example/.well-known/opencrs.json": { ...ok, body: index },
    "https://harbourline.example/opencrs/retail.screens.json": { ...ok, body: screens },
  };
  const r = JSON.parse(api.publisher("harbourline.example", pages, "2026-10-25T12:00:00Z"));
  assert.equal(r.errors, 0, JSON.stringify(r.advice));
  assert.equal(r.discovery.documents[0].status, "trusted");
}

console.log(`ok: opencrs ${api.version} WebAssembly${CHECK_PENDING ? " (check pending)" : ""}`);
process.exit(0);
