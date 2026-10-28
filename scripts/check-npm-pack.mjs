// Checks `npm pack --json` output for @opencrs/opencrs before it's published:
// every platform's binary is in the tarball, with the files that load them.
//
//   node scripts/check-npm-pack.mjs pack.json v0.1.2
import { readFileSync } from "node:fs";

const [file, tag] = process.argv.slice(2);
const [pack] = JSON.parse(readFileSync(file, "utf8"));
const names = pack.files.map((f) => f.path);
const nodes = names.filter((n) => n.endsWith(".node"));
const want = [
  "opencrs.darwin-arm64.node",
  "opencrs.darwin-x64.node",
  "opencrs.linux-x64-gnu.node",
  "opencrs.linux-arm64-gnu.node",
  "opencrs.linux-x64-musl.node",
  "opencrs.linux-arm64-musl.node",
  "opencrs.win32-x64-msvc.node",
];
const missing = [
  ...want.filter((n) => !names.includes(n)),
  ...["package.json", "index.js", "index.d.ts", "binding.js", "README.md", "LICENSE"].filter((n) => !names.includes(n)),
];
if (missing.length || nodes.length !== want.length) {
  console.error(`tarball is missing ${missing.join(", ") || "nothing"}; ${nodes.length} .node files, expected ${want.length}`);
  process.exit(1);
}
if (pack.version !== tag.replace(/^v/, "")) {
  console.error(`tarball version ${pack.version} is not ${tag}`);
  process.exit(1);
}
console.log(`${pack.filename}: ${nodes.join(", ")}`);
