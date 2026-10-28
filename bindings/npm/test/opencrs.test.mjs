import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { createRequire } from "node:module";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { lookup as dnsLookup } from "node:dns";
import { gzipSync } from "node:zlib";
import { fileURLToPath, pathToFileURL } from "node:url";

const require = createRequire(import.meta.url);
const opencrs = require("../index.js");

const root = join(dirname(fileURLToPath(import.meta.url)), "../../..");
const harbourline = join(root, "examples/harbourline");
const spec = () => readFileSync(join(harbourline, "retail.screens.json"), "utf8");

test("version is the spec version", () => {
  assert.equal(opencrs.version(), "0.1");
});

test("lint accepts the Harbourline example", () => {
  const res = opencrs.lint(spec());
  assert.equal(res.kind, "screens");
  assert.equal(res.schema_errors, undefined);
  assert.equal(res.document_checks, undefined);
});

test("lint reports a broken reference", () => {
  const broken = JSON.stringify({
    opencrs: "0.1",
    publisher: "a.example",
    updated_at: "2026-10-20T09:00:00Z",
    networks: [
      {
        id: "n",
        name: "N",
        formats: [{ id: "f", pixels: [1, 1], media: { static: ["image/png"] } }],
        screens: [{ id: "s", format: "g" }],
      },
    ],
  });
  const res = opencrs.lint(broken);
  assert.equal(res.document_checks.length, 1);
  assert.equal(res.document_checks[0].code, "CRS-REFERENCE");
});

test("lint reports schema errors", () => {
  const res = opencrs.lint(
    JSON.stringify({ opencrs: "0.1", publisher: "a.example", networks: [] }),
  );
  assert.ok(res.schema_errors.length > 0);
  assert.equal(typeof res.schema_errors[0].path, "string");
  assert.equal(typeof res.schema_errors[0].message, "string");
});

test("lint throws an Error for an unreadable document", () => {
  assert.throws(() => opencrs.lint("{not json"), Error);
  assert.throws(() => opencrs.lint('{"hello":1}'), (e) => e instanceof Error && e.message.length > 0);
});

test("resolve a format reference gives one result per screen", () => {
  const reqs = opencrs.resolve(spec(), "harbourline.example/retail/portrait-totem-75");
  assert.deepEqual(
    reqs.map((r) => r.screen),
    ["HC-L1-001", "HC-L1-002"],
  );
  assert.equal(reqs[0].format, "portrait-totem-75");
  assert.deepEqual(reqs[0].pixels, [1080, 1920]);
});

test("resolve a screen reference", () => {
  const [r] = opencrs.resolve(spec(), "harbourline.example/retail/screen/HC-L1-002");
  assert.equal(r.venue, "harbour-central");
  assert.deepEqual(
    r.blocked_regions.map((b) => b.name),
    ["network-logo", "column"],
  );
  assert.ok(r.derived.min_key_cap_height.px > 0);
  assert.equal(typeof r.derived.max_words.value, "number");
});

test("resolve errors throw Error with the core's message", () => {
  assert.throws(() => opencrs.resolve(spec(), "a/b"), {
    name: "Error",
    message: '"a/b" is not a format or screen reference',
  });
  assert.throws(() => opencrs.resolve(spec(), "harbourline.example/nope/x"), {
    message: 'network "nope" not found in harbourline.example',
  });
});

test("resolveStrictest combines the screens of a format", () => {
  const s = opencrs.resolveStrictest(spec(), "harbourline.example/retail/portrait-totem-75");
  assert.equal(s.screens, 2);
  assert.equal(s.ref, "harbourline.example/retail/portrait-totem-75");
  // The farther totem sets the text size.
  assert.equal(s.min_cap_height_px.headline, 77);
  assert.equal(s.sources["min_cap_height_px.headline"], "HC-L1-002");
  const one = opencrs.resolveStrictest(spec(), "harbourline.example/retail/screen/HC-L1-002");
  assert.equal(one.screens, 1);
  assert.deepEqual(one.min_cap_height_px, s.min_cap_height_px);
  assert.throws(() => opencrs.resolveStrictest(spec(), "a/b"), {
    message: '"a/b" is not a format or screen reference',
  });
});

test("probe reads a file's own header", () => {
  const p = opencrs.probe(join(harbourline, "lunch-portrait.png"));
  assert.equal(p.media_type, "image/png");
  assert.deepEqual(p.pixels, [1080, 1920]);
  assert.match(p.sha256, /^[0-9a-f]{64}$/);
  const bytes = readFileSync(join(harbourline, "lunch-portrait.png"));
  assert.equal(p.bytes, bytes.length);
  assert.deepEqual(opencrs.probeBytes(new Uint8Array(bytes)), p);
});

test("probe of an unknown file has no pixels", () => {
  const p = opencrs.probeBytes(new Uint8Array([1, 2, 3]));
  assert.equal(p.media_type, "");
  assert.equal(p.pixels, undefined);
  assert.equal(p.bytes, 3);
});

test("probe of a missing file throws", () => {
  assert.throws(() => opencrs.probe(join(harbourline, "missing.png")), Error);
});

const NOW = "2026-10-22T12:00:00+11:00";
const text = (p) => readFileSync(p, "utf8");
const withoutValidator = (reports) => reports.map(({ validator, ...r }) => r);

test("check matches reports.json for Harbourline", () => {
  const probes = {};
  for (const name of ["lunch-portrait.png", "lunch-portrait-logo-right.png"]) {
    probes[name] = opencrs.probe(join(harbourline, name));
  }
  const res = opencrs.check([spec()], text(join(harbourline, "creative.json")), probes, NOW);
  assert.deepEqual(res.notes, []);
  const want = JSON.parse(text(join(harbourline, "reports.json")));
  assert.deepEqual(withoutValidator(res.reports), withoutValidator(want));
  assert.deepEqual(res.reports[0].validator, { name: "opencrs", version: "0.1" });
  const viaDir = opencrs.checkDir(harbourline, [spec()], text(join(harbourline, "creative.json")), NOW);
  assert.deepEqual(viaDir, { ...res, missing: [] });
});

// A file not found is judged on the manifest's own values; the caller must
// be able to see that it happened.
test("checkDir lists the files it couldn't find", () => {
  const manifest = text(join(harbourline, "creative.json"));
  const dir = mkdtempSync(join(tmpdir(), "opencrs-"));
  try {
    copyFileSync(join(harbourline, "lunch-portrait.png"), join(dir, "lunch-portrait.png"));
    assert.deepEqual(opencrs.checkDir(dir, [spec()], manifest, NOW).missing, ["lunch-portrait-logo-right.png"]);
    assert.deepEqual(opencrs.checkDir(join(dir, "typo"), [spec()], manifest, NOW).missing, [
      "lunch-portrait.png",
      "lunch-portrait-logo-right.png",
    ]);
  } finally {
    rmSync(dir, { recursive: true });
  }
});

test("check keeps the offset of now and defaults to the host time", () => {
  const manifest = text(join(harbourline, "creative.json"));
  const res = opencrs.check([spec()], manifest, {}, "2026-10-22T12:00:00+05:30");
  assert.equal(res.reports[0].checked_at, "2026-10-22T12:00:00+05:30");
  const now = opencrs.check([spec()], manifest, {});
  assert.match(now.reports[0].checked_at, /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(Z|[+-]\d\d:\d\d)$/);
});

// What a strictest case expects: objects by the members it gives, lists exactly.
const within = (want, got, at) => {
  if (want === null || want === undefined) return;
  if (Array.isArray(want)) {
    assert.ok(Array.isArray(got) && got.length === want.length, `${at}: ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
    want.forEach((w, i) => within(w, got[i], `${at}[${i}]`));
  } else if (typeof want === "object") {
    assert.equal(typeof got, "object", at);
    for (const [k, v] of Object.entries(want)) {
      assert.ok(k in got, `${at}.${k} is missing`);
      within(v, got[k], `${at}.${k}`);
    }
  } else {
    assert.equal(got, want, at);
  }
};

test("checkDir passes every conformance case", () => {
  const cases = join(root, "conformance/0.1/cases");
  const names = readdirSync(cases).sort();
  assert.ok(names.length > 0);
  for (const name of names) {
    const dir = join(cases, name);
    const c = JSON.parse(text(join(dir, "case.json")));
    const specs = c.specs.map((s) => text(join(dir, s)));
    if (c.strictest?.length) {
      for (const want of c.strictest) {
        if (want.error) {
          assert.throws(() => opencrs.resolveStrictest(specs[0], want.ref), { message: new RegExp(want.error) }, name);
          continue;
        }
        const got = opencrs.resolveStrictest(specs[0], want.ref);
        assert.ok(want.expect !== undefined || want.notes !== undefined, `${name}: the case expects nothing`);
        within(want.expect, got, `${name} ${want.ref}`);
        for (const path of want.absent ?? []) {
          const at = path.split(".").reduce((v, k) => v?.[k], got);
          assert.equal(at, undefined, `${name}: ${path} is there`);
        }
        if (want.notes !== undefined) assert.equal((got.notes ?? []).length, want.notes, name);
      }
      continue;
    }
    const { reports } = opencrs.checkDir(dir, specs, text(join(dir, c.creative)), c.now);
    assert.equal(reports.length, c.expect.length, name);
    for (const want of c.expect) {
      const got = reports.find((r) => r.file === want.file && r.target === want.target);
      assert.ok(got, `${name}: no report for ${want.file} ${want.target}`);
      assert.equal(got.result, want.result, name);
      // Every expected check is present; any other finding must be info.
      const key = (k) => `${k.code}/${k.severity}`;
      const have = got.checks.map(key);
      for (const k of want.checks.map(key)) {
        const i = have.indexOf(k);
        assert.ok(i >= 0, `${name}: missing ${k}`);
        have.splice(i, 1);
      }
      assert.deepEqual(have.filter((k) => !k.endsWith("/info")), [], name);
    }
  }
});

test("check errors throw Error with the core's message", () => {
  const manifest = text(join(harbourline, "creative.json"));
  assert.throws(() => opencrs.check(["{not json"], manifest, {}, NOW), Error);
  assert.throws(() => opencrs.check([spec()], "{not json", {}, NOW), Error);
  assert.throws(() => opencrs.check([spec()], manifest, {}, "yesterday"), /RFC 3339/);
  assert.throws(() => opencrs.check([spec()], manifest, { "a.png": 1 }, NOW), Error);
});

const manifestNaming = (name) => {
  const m = JSON.parse(text(join(harbourline, "creative.json")));
  m.files = [{ ...m.files[0], file: name }];
  return JSON.stringify(m);
};

test("checkDir confines files to the directory", () => {
  for (const name of ["../x", "sub/../../x", "/etc/hosts", "/dev/zero"]) {
    assert.throws(
      () => opencrs.checkDir(harbourline, [spec()], manifestNaming(name), NOW),
      /inside the directory/,
      name,
    );
  }
});

// Unix only: /etc/hosts, /dev/zero and FIFOs don't exist on Windows, and
// symlinks there need admin rights. The core's fs tests cover Windows paths.
test("checkDir rejects symlinks out of the directory, FIFOs and directories", { skip: process.platform === "win32" }, () => {
  const dir = mkdtempSync(join(tmpdir(), "opencrs-"));
  try {
    symlinkSync("/etc/hosts", join(dir, "out"));
    assert.throws(() => opencrs.checkDir(dir, [spec()], manifestNaming("out"), NOW), /outside the directory/);
    execFileSync("mkfifo", [join(dir, "pipe")]);
    assert.throws(() => opencrs.checkDir(dir, [spec()], manifestNaming("pipe"), NOW), /not a regular file/);
    mkdirSync(join(dir, "gone"));
    assert.throws(() => opencrs.checkDir(dir, [spec()], manifestNaming("gone"), NOW), /not a regular file/);
    assert.throws(() => opencrs.probe(join(dir, "pipe")), /not a regular file/);
    assert.throws(() => opencrs.probe("/dev/zero"), /not a regular file/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// The calls that take plain data are compared with the WebAssembly build, the
// same core as the browser and Go run: the results must be equal, not merely
// alike.
const { load } = await import(pathToFileURL(join(root, "js/opencrs.js")).href);
const wasm = await load(readFileSync(join(root, "opencrs.wasm")));
const viaWasm = (s) => JSON.parse(s);

test("every operation in parity/operations.json is exported, and nothing else is", () => {
  const manifest = JSON.parse(readFileSync(join(root, "parity/operations.json"), "utf8"));
  const listed = new Set(manifest.helpers.npm);
  for (const [op, entry] of Object.entries(manifest.operations)) {
    const names = entry.npm;
    if (names === null) {
      assert.ok(manifest.exceptions[`npm.${op}`], `${op}: no npm name and no exception`);
      continue;
    }
    assert.ok(Array.isArray(names) && names.length > 0, `${op}: npm needs a list of names or null`);
    assert.equal(manifest.exceptions[`npm.${op}`], undefined, `${op}: has npm names and an exception`);
    for (const name of names) {
      assert.equal(typeof opencrs[name], "function", `${op}: npm has no ${name}`);
      listed.add(name);
    }
  }
  assert.deepEqual(Object.keys(opencrs).sort(), [...listed].sort(), "an export is missing from the manifest");
});

test("legibility is the spec's worked example, and what the WebAssembly build says", () => {
  const req = { distance_m: [2, 6], screen: { width_mm: 940, width_px: 1080 }, dwell_s: 4 };
  const res = opencrs.legibility(req);
  assert.equal(res.ratio, 150);
  assert.deepEqual(res.key, { mm: 40, px: 46, px_exact: 45.96, basis: "at 6 m, ratio 150" });
  assert.deepEqual(res.legal, { mm: 13.3, px: 16, px_exact: 15.32, basis: "at 2 m, ratio 150" });
  assert.deepEqual(res.words, { value: 8, basis: "4 s dwell at 2 words/s" });
  assert.deepEqual(res, viaWasm(wasm.legibility(req)));
  const bare = { distance_m: [3, 10], ratio: 250 };
  assert.deepEqual(opencrs.legibility(bare), viaWasm(wasm.legibility(bare)));
  assert.equal(opencrs.legibility(bare).key.px, undefined);
});

test("legibility errors throw Error with the core's message", () => {
  assert.throws(() => opencrs.legibility({ distance_m: [6, 2] }), /the near distance \(6 m\) is past the far one \(2 m\)/);
  assert.throws(() => opencrs.legibility({}), (e) => e instanceof Error && e.message.startsWith("usage:"));
  assert.throws(() => opencrs.legibility({ distance_m: [2, 6], ratio: 0 }), /ratio must be a number above zero/);
});

test("dwell gives the glance and each pack's limits, as the WebAssembly build does", () => {
  const req = { speed_kmh: 80, distance_m: [20, 150], rule_packs: ["au-nsw-transport-corridor@2017-11"] };
  const res = opencrs.dwell(req);
  assert.equal(res.glance.words, 4);
  assert.equal(res.glance.read_from_m, 64.44);
  assert.equal(res.key.mm, 429.6);
  assert.equal(res.packs[0].min_dwell_s, 25);
  assert.equal(res.packs[0].motion_allowed, false);
  assert.deepEqual(res, viaWasm(wasm.dwell(req)));
  const slow = { speed_kmh: 60, distance_m: [20, 150], screen: { width_mm: 940, width_px: 1080 } };
  assert.deepEqual(opencrs.dwell(slow), viaWasm(wasm.dwell(slow)));
  assert.throws(() => opencrs.dwell({ ...req, rule_packs: ["x@1"] }), /unknown rule pack x@1/);
});

test("rulePacks lists the built-in packs, as the WebAssembly build does", () => {
  const res = opencrs.rulePacks();
  assert.equal(res.rule_packs.length, 6);
  assert.ok(res.rule_packs.every((p) => p.ref && p.title && p.status));
  assert.deepEqual(res, viaWasm(wasm.rulePacks()));
});

const index = () => readFileSync(join(harbourline, ".well-known/opencrs.json"), "utf8");
const INDEX_URL = "https://harbourline.example/.well-known/opencrs.json";
const DOC_URL = "https://harbourline.example/opencrs/retail.screens.json";
const served = { cache_control: "max-age=60", etag: '"v1"', content_type: "application/json" };

test("publisher asks for the index, then checks what it is given, as the WebAssembly build does", () => {
  const first = opencrs.publisher("harbourline.example", {}, NOW);
  assert.deepEqual(first.wanted, [INDEX_URL]);
  assert.deepEqual(first.wanted_detail, [{ url: INDEX_URL, kind: "index", same_host: true }]);

  const pages = {
    [INDEX_URL]: { status: 200, body: index(), ...served },
    [DOC_URL]: { status: 200, body: Buffer.from(spec()), ...served, allow_origin: "*" },
  };
  const res = opencrs.publisher("harbourline.example", pages, NOW);
  assert.deepEqual(res.wanted, []);
  assert.equal(res.errors, 0);
  assert.equal(res.discovery.documents.length, 1);
  assert.equal(res.discovery.documents[0].status, "trusted");
  assert.equal(res.discovery.documents[0].screens, 3);
  assert.deepEqual(res, viaWasm(wasm.publisher("harbourline.example", pages, NOW)));

  // A document that wasn't fetched, and one that failed.
  const part = { [INDEX_URL]: pages[INDEX_URL] };
  const missing = opencrs.publisher("harbourline.example", part, NOW, "skipped for now");
  assert.deepEqual(missing.wanted, [DOC_URL]);
  assert.match(missing.discovery.documents[0].reason, /skipped for now/);
  assert.deepEqual(missing, viaWasm(wasm.publisher("harbourline.example", part, NOW, "skipped for now")));
  const gone = { ...part, [DOC_URL]: { status: 404 } };
  assert.deepEqual(
    opencrs.publisher("harbourline.example", gone, NOW),
    viaWasm(wasm.publisher("harbourline.example", gone, NOW)),
  );
  const broke = { ...part, [DOC_URL]: { status: 0, error: "connection reset" } };
  assert.match(opencrs.publisher("harbourline.example", broke, NOW).discovery.documents[0].reason, /connection reset/);
});

test("publisher reads bodies as bytes and its advice is about how they were served", () => {
  const bare = { [INDEX_URL]: { status: 200, body: index() }, [DOC_URL]: { status: 200, body: spec() } };
  const res = opencrs.publisher("harbourline.example", bare, NOW);
  assert.ok(res.warnings >= 2, "no Cache-Control or ETag");
  assert.ok(res.advice.some((a) => a.url === DOC_URL && /Cache-Control/.test(a.message)));
  assert.deepEqual(res, viaWasm(wasm.publisher("harbourline.example", bare, NOW)));
  // Not UTF-8: arrives as it is, and is not an index.
  const bad = opencrs.publisher("harbourline.example", { [INDEX_URL]: { status: 200, body: Uint8Array.of(0x3c, 0xff) } }, NOW);
  assert.equal(bad.errors, 1);
  assert.ok(bad.error);
});

test("publisher throws Error for bad arguments", () => {
  assert.throws(() => opencrs.publisher("harbourline.example", {}, "yesterday"), /not an RFC 3339 time/);
  assert.throws(() => opencrs.publisher("harbourline.example", { [INDEX_URL]: 5 }));
});

test("fetchPublisher is async and says what the publisher check says of a domain that can't publish", async () => {
  const p = opencrs.fetchPublisher("not a domain", { now: NOW });
  assert.ok(p instanceof Promise);
  const res = await p;
  assert.match(res.error, /isn't a publisher domain/);
  assert.equal(res.errors, 1);
  assert.deepEqual(res, viaWasm(wasm.publisher("not a domain", {}, NOW)));
  // Local names can't publish either, so a loopback server can't be pointed at.
  assert.match((await opencrs.fetchPublisher("localhost")).error, /isn't a publisher domain/);
  assert.match((await opencrs.fetchPublisher("127.0.0.1:1")).error, /isn't a publisher domain/);
});

test("fetchPublisher and discover reject, not throw, for a bad option", async () => {
  await assert.rejects(opencrs.fetchPublisher("harbourline.example", { now: "later" }), /not an RFC 3339 time/);
  await assert.rejects(opencrs.discover("harbourline.example", { now: "later" }), /not an RFC 3339 time/);
});

/** A publisher on a local port: its index lists two documents, and the paths under /bad misbehave. */
function localPublisher(handler) {
  return new Promise((resolve) => {
    const seen = [];
    const server = createServer((req, res) => {
      seen.push(req.url);
      const addr = `127.0.0.1:${server.address().port}`;
      handler(req, res, addr);
    });
    server.listen(0, "127.0.0.1", () => resolve({ server, seen, addr: `127.0.0.1:${server.address().port}` }));
  });
}

test("discover reads a local publisher and judges its documents, off the event loop", async () => {
  const doc = Buffer.from(spec());
  const { server, seen, addr } = await localPublisher((req, res, addr) => {
    if (req.url === "/.well-known/opencrs.json") {
      res.setHeader("Content-Type", "application/json");
      res.end(JSON.stringify({
        opencrs: "0.1",
        publisher: { id: "harbourline.example", name: "Harbourline" },
        documents: [
          { url: `http://${addr}/retail.screens.json`, network: "retail" },
          { url: `http://${addr}/slow.screens.json`, network: "retail" },
          { url: `http://${addr}/metadata.screens.json`, network: "retail" },
        ],
      }));
    } else if (req.url === "/retail.screens.json") {
      res.setHeader("Content-Encoding", "gzip");
      res.setHeader("Cache-Control", "max-age=3600");
      res.end(gzipSync(doc));
    } else if (req.url === "/slow.screens.json") {
      setTimeout(() => res.end(doc), 400);
    } else if (req.url === "/metadata.screens.json") {
      res.statusCode = 302;
      res.setHeader("Location", "https://169.254.169.254/latest/meta-data/");
      res.end();
    } else {
      res.statusCode = 404;
      res.end();
    }
  });
  try {
    let ticks = 0;
    const timer = setInterval(() => ticks++, 20);
    const res = await opencrs.discover(addr, { unsafeAllowLocalHttp: true, now: NOW });
    clearInterval(timer);
    assert.ok(ticks >= 8, `the event loop ran only ${ticks} times in 400 ms`);
    assert.equal(res.index.publisher.name, "Harbourline");
    const by = Object.fromEntries(res.documents.map((d) => [d.url.split("/").pop(), d]));
    assert.equal(by["retail.screens.json"].status, "trusted");
    assert.equal(by["retail.screens.json"].screens, 3, "the gzipped body was read");
    assert.equal(by["retail.screens.json"].cache_control, "max-age=3600");
    assert.equal(by["slow.screens.json"].status, "trusted");
    // A redirect to a cloud metadata address is refused, and nothing is sent there.
    assert.equal(by["metadata.screens.json"].status, "failed");
    assert.match(by["metadata.screens.json"].reason, /refusing to connect to 169\.254\.169\.254/);
    // Without local testing, the local address isn't a publisher at all.
    await assert.rejects(opencrs.discover(addr, { now: NOW }), /isn't a publisher domain/);
    // An index that can't be fetched rejects.
    await assert.rejects(opencrs.discover("127.0.0.1:1", { unsafeAllowLocalHttp: true }), /refused|Connection|connect/i);
    assert.ok(seen.includes("/.well-known/opencrs.json"));
  } finally {
    server.close();
  }
});

/** A publisher whose index is a hundred megabytes sent a byte at a time: as slow as it can be without stopping. */
function dripServer() {
  return new Promise((resolve) => {
    const server = createServer((req, res) => {
      res.writeHead(200, { "Content-Length": 1e8 });
      const t = setInterval(() => res.write(" "), 20);
      res.on("close", () => clearInterval(t));
    });
    server.listen(0, "127.0.0.1", () => resolve({ server, addr: `127.0.0.1:${server.address().port}` }));
  });
}

const slow = (extra = {}) => ({ unsafeAllowLocalHttp: true, now: NOW, ...extra });

test("the total time limit stops a publisher that never finishes", async () => {
  const { server, addr } = await dripServer();
  try {
    const t = Date.now();
    await assert.rejects(opencrs.discover(addr, slow({ totalTimeoutMs: 500 })), /time limit for the whole check/);
    const took = Date.now() - t;
    assert.ok(took >= 400 && took < 3000, `took ${took} ms`);
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

test("an AbortSignal stops a publisher that never finishes", async () => {
  const { server, addr } = await dripServer();
  try {
    const ctl = new AbortController();
    setTimeout(() => ctl.abort(), 300);
    const t = Date.now();
    await assert.rejects(opencrs.discover(addr, slow({ signal: ctl.signal })), (e) => e.name === "AbortError");
    assert.ok(Date.now() - t < 3000, `took ${Date.now() - t} ms`);
    // A reason given to abort() is what rejects.
    const ctl2 = new AbortController();
    setTimeout(() => ctl2.abort(new Error("enough")), 200);
    await assert.rejects(opencrs.discover(addr, slow({ signal: ctl2.signal })), /enough/);
    // Already aborted: nothing is started.
    const before = server.listenerCount("request");
    await assert.rejects(opencrs.discover(addr, slow({ signal: AbortSignal.abort() })), (e) => e.name === "AbortError");
    await assert.rejects(opencrs.fetchPublisher("harbourline.example", { signal: AbortSignal.abort() }), (e) => e.name === "AbortError");
    assert.equal(server.listenerCount("request"), before);
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

test("slow publishers don't hold up the rest of the process", async () => {
  const { server, addr } = await dripServer();
  try {
    // More calls than the libuv pool has threads, all waiting on a server that never finishes.
    const ctl = new AbortController();
    const calls = Array.from({ length: 8 }, () =>
      opencrs.discover(addr, slow({ signal: ctl.signal })).catch(() => {}),
    );
    await new Promise((r) => setTimeout(r, 100));
    const t = Date.now();
    await readFile(fileURLToPath(import.meta.url)); // uses the libuv pool
    await new Promise((r, j) => dnsLookup("localhost", (e) => (e ? j(e) : r())));
    assert.ok(Date.now() - t < 500, `fs and dns waited ${Date.now() - t} ms behind the fetches`);
    ctl.abort();
    await Promise.all(calls);
  } finally {
    server.closeAllConnections();
    server.close();
  }
});

test("huge or odd numbers in options are clamped, not wrapped or fatal", async () => {
  const odd = { maxDocumentBytes: Infinity, maxTotalBytes: 1e30, maxRounds: 1e30, maxPages: Infinity, timeoutMs: 1e300, totalTimeoutMs: Infinity };
  const res = await opencrs.fetchPublisher("not a domain", { ...odd, now: NOW });
  assert.match(res.error, /isn't a publisher domain/);
  for (const bad of [NaN, -1, 0, -Infinity]) {
    const r = await opencrs.fetchPublisher("not a domain", { maxTotalBytes: bad, timeoutMs: bad, totalTimeoutMs: bad, now: NOW });
    assert.match(r.error, /isn't a publisher domain/);
  }
  // And a real fetch with them still works.
  const doc = Buffer.from(spec());
  const { server, addr } = await localPublisher((req, res, a) => {
    if (req.url === "/.well-known/opencrs.json") {
      res.end(JSON.stringify({ opencrs: "0.1", publisher: { id: "harbourline.example", name: "H" }, documents: [{ url: `http://${a}/d.screens.json`, network: "retail" }] }));
    } else {
      res.end(doc);
    }
  });
  try {
    const d = await opencrs.discover(addr, slow({ timeoutMs: 1e300, totalTimeoutMs: Infinity }));
    assert.equal(d.documents[0].status, "trusted");
  } finally {
    server.close();
  }
});
