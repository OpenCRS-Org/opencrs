import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

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
        within(want.expect, got, `${name} ${want.ref}`);
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
