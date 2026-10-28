"use strict";

// The native addon, built by `napi build` (see package.json). This wrapper
// keeps the public surface (and its types in index.d.ts) in one place.
const native = require("./binding.js");

const abortReason = (signal) =>
  signal.reason ?? Object.assign(new Error("The operation was aborted"), { name: "AbortError" });

// The fetching calls take an AbortSignal in their options. The native side
// has a token it checks between pages and within each read; the signal
// flips it.
function abortable(fn) {
  return async (domain, options) => {
    const { signal, ...rest } = options ?? {};
    if (signal?.aborted) throw abortReason(signal);
    const token = new native.CancelToken();
    const onAbort = () => token.cancel();
    signal?.addEventListener("abort", onAbort, { once: true });
    try {
      return await fn(domain, rest, token);
    } catch (e) {
      if (signal?.aborted) throw abortReason(signal);
      throw e;
    } finally {
      signal?.removeEventListener("abort", onAbort);
    }
  };
}

module.exports = {
  version: native.version,
  lint: native.lint,
  probe: native.probe,
  probeBytes: native.probeBytes,
  check: native.check,
  checkDir: native.checkDir,
  resolve: native.resolve,
  resolveStrictest: native.resolveStrictest,
  legibility: native.legibility,
  dwell: native.dwell,
  rulePacks: native.rulePacks,
  publisher: native.publisher,
  fetchPublisher: abortable(native.fetchPublisher),
  discover: abortable(native.discover),
};
