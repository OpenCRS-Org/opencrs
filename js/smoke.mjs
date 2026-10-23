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
  "opencrs_check", "opencrs_resolve", "opencrs_probe_start", "opencrs_probe_feed"]) {
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
  assert.equal(check.requirements["harbourline.example/retail/portrait-totem-75"].length, 2);
  assert.ok(Array.isArray(resolved) && resolved.length === 2);
}

console.log(`ok: opencrs ${api.version} WebAssembly${CHECK_PENDING ? " (check pending)" : ""}`);
process.exit(0);
