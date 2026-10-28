// Calls the WebAssembly build (the browser's and Go's core) for a Python test
// to compare with: `node wasm_call.mjs OP` reads one JSON request on stdin and
// prints the result as JSON.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../../..");
const { load } = await import(pathToFileURL(join(root, "js/opencrs.js")).href);
const wasm = await load(readFileSync(join(root, "opencrs.wasm")));
const req = JSON.parse(readFileSync(0, "utf8"));

const ops = {
  legibility: () => wasm.legibility(req),
  dwell: () => wasm.dwell(req),
  rule_packs: () => wasm.rulePacks(),
  publisher: () => wasm.publisher(req.domain, req.pages, req.now, req.missing),
};
const op = ops[process.argv[2]];
if (!op) throw new Error(`unknown operation ${process.argv[2]}`);
process.stdout.write(op());
