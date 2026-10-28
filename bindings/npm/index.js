"use strict";

// The native addon, built by `napi build` (see package.json). This wrapper
// keeps the public surface (and its types in index.d.ts) in one place.
const native = require("./binding.js");

module.exports = {
  version: native.version,
  lint: native.lint,
  probe: native.probe,
  probeBytes: native.probeBytes,
  check: native.check,
  checkDir: native.checkDir,
  resolve: native.resolve,
  resolveStrictest: native.resolveStrictest,
};
