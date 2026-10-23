# @opencrs/opencrs

Node.js bindings for the OpenCRS core (Rust, via napi-rs): lint documents,
probe creative files, and resolve screen requirements. OpenCRS is the Open
Creative Requirements Specification: https://opencrs.dev

Early work: version 0.0.0 and not yet published.

```js
const opencrs = require("@opencrs/opencrs");

opencrs.version(); // "0.1"
opencrs.lint(text); // { kind, schema_errors?, document_checks? }
opencrs.resolve(spec, "publisher.example/network/format"); // Requirements[]
opencrs.check([spec], manifest, { "a.png": probe }); // { reports, notes }
opencrs.checkDir(dir, [spec], manifest); // { reports, notes, missing }: probes the manifest's files in dir
opencrs.probe("creative.png"); // { sha256, bytes, media_type, pixels? }
```

Errors from the core are thrown as `Error` with the core's message.

## Develop

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
