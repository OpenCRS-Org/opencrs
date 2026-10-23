// Runs one conformance case through the WebAssembly module, as the adapter
// for `opencrs conformance -cmd`. The command gets the case directory as its
// last argument and prints the reports as a JSON array.
//
//   OPENCRS_WASM=/tmp/wasm cargo run -p opencrs-cli -- conformance -cmd "node js/conformance-adapter.mjs"
//
// OPENCRS_WASM is the OUT_DIR given to scripts/build-wasm.sh.
import { readFile } from "node:fs/promises";
import { join, dirname } from "node:path";
import { pathToFileURL } from "node:url";

const caseDir = process.argv[2];
const wasmDir = process.env.OPENCRS_WASM;
if (!caseDir || !wasmDir) {
  console.error("usage: OPENCRS_WASM=OUT_DIR node conformance-adapter.mjs CASE_DIR");
  process.exit(2);
}
const { load } = await import(pathToFileURL(join(wasmDir, "opencrs.js")));
const api = await load(await readFile(join(wasmDir, "opencrs.wasm")));

const c = JSON.parse(await readFile(join(caseDir, "case.json"), "utf8"));
const specs = await Promise.all(c.specs.map((s) => readFile(join(caseDir, s), "utf8")));
const manifestText = await readFile(join(caseDir, c.creative), "utf8");
let manifest;
try {
  manifest = JSON.parse(manifestText);
} catch (e) {
  console.error(`${c.creative}: ${e.message}`);
  process.exit(2);
}
// Creative files present next to the manifest are read; others are judged on
// the manifest's declared values.
const files = {};
for (const f of manifest.files ?? []) {
  try {
    files[f.file] = new Uint8Array(await readFile(join(dirname(join(caseDir, c.creative)), f.file)));
  } catch (e) {
    if (e.code !== "ENOENT") throw e;
  }
}
const res = JSON.parse(await api.check(specs, manifestText, files, c.now));
if (res.error) {
  console.error(res.error);
  process.exit(2);
}
console.log(JSON.stringify(res.reports));
