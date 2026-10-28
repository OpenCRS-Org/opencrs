# @opencrs/opencrs

Node.js bindings for the OpenCRS core (Rust, via napi-rs): lint documents,
probe creative files, resolve screen requirements, work out legibility and
dwell, and check a publisher's published specs. OpenCRS is the Open
Creative Requirements Specification: https://opencrs.dev

Early work: the API may change before 1.0.

## Install

```sh
npm install @opencrs/opencrs
```

One package carries prebuilt binaries for macOS (arm64, x64), Linux (glibc and
musl, arm64 and x64) and Windows (x64). Node 18 or later. Documentation:
https://opencrs.dev

```js
const opencrs = require("@opencrs/opencrs");

opencrs.version(); // "0.1"
opencrs.lint(text); // { kind, schema_errors?, document_checks? }
opencrs.resolve(spec, "publisher.example/network/format"); // Requirements[]
opencrs.resolveStrictest(spec, "publisher.example/network/format"); // one StrictestRequirements for every screen at once
opencrs.check([spec], manifest, { "a.png": probe }); // { reports, notes }
opencrs.checkDir(dir, [spec], manifest); // { reports, notes, missing }: probes the manifest's files in dir
opencrs.probe("creative.png"); // { sha256, bytes, media_type, pixels? }

// Minimum text sizes, and what a driver can read, as the checks work them out.
opencrs.legibility({ distance_m: [2, 6], screen: { width_mm: 940, width_px: 1080 } }).key.px; // 46
opencrs.dwell({ speed_kmh: 80, distance_m: [20, 150], rule_packs: ["au-nsw-transport-corridor@2017-11"] }).glance.words; // 4
opencrs.rulePacks(); // { rule_packs: [{ ref, title, status, source_url? }, ...] }

// Check a publisher's specs from pages you fetched, or let it fetch them.
opencrs.publisher("harbourline.example", pages); // { errors, warnings, advice, wanted, ... }
await opencrs.fetchPublisher("harbourline.example"); // the same result, after fetching
await opencrs.discover("harbourline.example"); // which documents to trust, as `opencrs fetch` says
```

Errors from the core are thrown as `Error` with the core's message. The
calculators, `rulePacks` and `publisher` give the same objects as the browser
build (`js/opencrs.js`) and Go for the same input, so `rulePacks()` has its
list under `rule_packs`.

## Fetching a publisher

`publisher(domain, pages)` takes pages you have fetched, keyed by URL (`{ status,
body, etag?, cache_control?, ... }`, with a string or bytes `body`); its `wanted`
lists the URLs it still needs. `fetchPublisher(domain, options?)` runs that loop
for you and returns a Promise, off the event loop. The publisher chose every URL,
so it fetches as carefully as the `opencrs` command line does: HTTPS only, no
connection to a loopback, private, link-local or other non-public address
(checked after DNS and on every redirect), no proxy, and caps on the size and
number of pages (`maxDocumentBytes`, `maxTotalBytes`, `maxRounds`, `maxPages`,
`timeoutMs`). `discover(domain, options?)` gives only the trust decisions;
`allowLocalHttp: true` lets it reach `localhost` over `http://` for testing.

## Develop

From a clone of https://github.com/OpenCRS-Org/opencrs, in `bindings/npm`:

```sh
npm install
npm run build   # builds the native addon for this machine
npm test
```

## File names in manifests

`checkDir` treats the manifest as untrusted. A file name that is absolute or
contains `..` is an error, as is a symlink that leaves `dir` or anything that
isn't a regular file (a FIFO, a device, a directory). A file that doesn't
exist is judged on the manifest's declared values instead and listed in the
result's `missing` (every file is, if `dir` itself doesn't exist): check it
before trusting a pass. `probe` also refuses non-regular files.
