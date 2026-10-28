// Loader for the OpenCRS WebAssembly module (opencrs.wasm). A plain ES
// module for browsers and Node: no build step, no dependencies. The module
// has no imports; this file moves JSON in and out of its memory and does the
// two things it can't: hashing (Web Crypto) and reading file ranges.
//
//   import { load } from "./opencrs.js";
//   const crs = await load(new URL("./opencrs.wasm", import.meta.url));
//   crs.lint(text);                      // text or UTF-8 bytes; JSON string
//   await crs.check([specText], creativeText, files, now); // JSON string
//   crs.resolve(specText, ref);                       // JSON string
//   crs.legibility({ distance_m: [2, 6] });           // JSON string
//   crs.dwell({ speed_kmh: 80, distance_m: [20, 150] }); // JSON string
//
// Every result is a JSON string: the result, or {"error": "..."}.

// The most one request may carry: the module could never hold more, and its
// sizes are 32-bit. Matches the Go package's limit.
const MAX_REQUEST = 256 * 1024 * 1024;
const tooLarge = (n) => new RangeError(`a request of ${n} bytes is more than the module's ${MAX_REQUEST}-byte memory limit`);

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function hex(buf) {
  return Array.from(new Uint8Array(buf), (b) => b.toString(16).padStart(2, "0")).join("");
}

async function fetchModule(source) {
  if (source instanceof URL && source.protocol === "file:") {
    const { readFile } = await import("node:fs/promises");
    return readFile(source);
  }
  const res = await fetch(source);
  if (!res.ok) throw new Error(`fetching ${source}: ${res.status}`);
  return res.arrayBuffer();
}

/** Loads the module from a URL or the bytes of opencrs.wasm. */
export async function load(source) {
  const bytes = source instanceof URL || typeof source === "string" ? await fetchModule(source) : source;
  const module = await WebAssembly.compile(bytes);
  // Anything thrown during a call (a trap when the module aborts on a panic
  // or runs out of memory, or a RangeError when its stack overflows) can
  // leave it in a bad state, so the instance is dropped and the next call
  // makes a fresh one from the same compiled module.
  let current = null;
  let instances = 0;
  const exports_ = () => {
    if (!current) {
      current = new WebAssembly.Instance(module, {}).exports;
      instances++;
    }
    return current;
  };
  const guard = (fn) => {
    try {
      return fn();
    } catch (e) {
      current = null;
      throw e;
    }
  };

  // Linear memory can grow during a call, so take a fresh view every time.
  const view = (ptr, len) => new Uint8Array(exports_().memory.buffer, ptr, len);
  const put = (data) => {
    if (data.length >= MAX_REQUEST) throw tooLarge(data.length);
    const ptr = exports_().opencrs_alloc(data.length);
    view(ptr, data.length).set(data);
    return ptr;
  };
  const take = (packed) => {
    // wasm i64 results arrive as signed BigInts; the pointer is unsigned.
    packed = BigInt.asUintN(64, packed);
    const ptr = Number(packed >> 32n);
    const len = Number(packed & 0xffffffffn);
    const out = decoder.decode(view(ptr, len));
    exports_().opencrs_free(ptr, len);
    return out;
  };
  const call = (name, request) =>
    guard(() => {
      const x = exports_();
      const data = encoder.encode(JSON.stringify(request));
      const ptr = put(data);
      try {
        return take(x[name](ptr, data.length));
      } finally {
        if (current === x) x.opencrs_free(ptr, data.length);
      }
    });
  // The calls that carry documents hand the module the raw bytes and give up
  // the allocation: the module frees it as soon as it has decoded them.
  const callOwned = (name, parts) =>
    guard(() => {
      const x = exports_();
      let total = 0;
      for (const p of parts) total += p.length;
      if (total >= MAX_REQUEST) throw tooLarge(total);
      const ptr = x.opencrs_alloc(total);
      let at = ptr;
      for (const p of parts) {
        view(at, p.length).set(p);
        at += p.length;
      }
      return take(x[name](ptr, total));
    });
  // header is a request object; blobs are the documents that follow it in a
  // frame: a little-endian u32 header length, the header JSON, then the blobs.
  const callFramed = (name, header, blobs) => {
    const bytes = blobs.map(toBytes);
    const head = encoder.encode(JSON.stringify({ ...header, blobs: bytes.map((b) => b.length) }));
    const len = new Uint8Array(4);
    new DataView(len.buffer).setUint32(0, head.length, true);
    return callOwned(name, [len, head, ...bytes]);
  };
  const toBytes = (doc) => (typeof doc === "string" ? encoder.encode(doc) : doc);
  const feed = (session, chunk) =>
    guard(() => {
      const x = exports_();
      const ptr = put(chunk);
      try {
        return take(x.opencrs_probe_feed(session, ptr, chunk.length));
      } finally {
        if (current === x) x.opencrs_free(ptr, chunk.length);
      }
    });

  /** Probes one file's bytes: hashes it, then answers the module's range requests. */
  async function probe(data) {
    const sha256 = hex(await crypto.subtle.digest("SHA-256", data));
    let step = JSON.parse(call("opencrs_probe_start", { size: data.length, sha256 }));
    while (!step.error && step.need) {
      const { offset, len } = step.need;
      step = JSON.parse(feed(step.session, data.subarray(offset, Math.min(offset + len, data.length))));
    }
    if (step.error) throw new Error(step.error);
    return step.probe;
  }

  const errorJSON = (e) => JSON.stringify({ error: String(e?.message ?? e) });

  return {
    version: JSON.parse(call("opencrs_version", {})).version,
    /** lint(doc) checks one document of any kind, given as text or UTF-8 bytes. */
    lint: (doc) =>
      guard(() => {
        const x = exports_();
        const data = toBytes(doc);
        const ptr = put(data);
        try {
          return take(x.opencrs_lint(ptr, data.length));
        } finally {
          if (current === x) x.opencrs_free(ptr, data.length);
        }
      }),
    /**
     * check(specs, manifest, files, now) checks a creative.json against one
     * or more screens.json documents (text or UTF-8 bytes). files maps manifest paths to Uint8Arrays;
     * now is an RFC 3339 time or "".
     */
    async check(specs, manifest, files, now = "") {
      try {
        // The module has no clock: an empty time means now.
        if (!now) now = new Date().toISOString();
        const probes = {};
        if (!globalThis.crypto?.subtle) throw new Error("Web Crypto (crypto.subtle) is needed to hash files; it needs a secure context");
        for (const [name, data] of Object.entries(files ?? {})) {
          probes[name] = await probe(data instanceof Uint8Array ? data : new Uint8Array(data));
        }
        return callFramed("opencrs_check", { probes, now }, [manifest, ...specs]);
      } catch (e) {
        return errorJSON(e);
      }
    },
    /** resolve(spec, ref) returns the requirements for a format or screen reference. */
    resolve(spec, ref) {
      try {
        return callFramed("opencrs_resolve", { ref }, [spec]);
      } catch (e) {
        return errorJSON(e);
      }
    },
    /**
     * legibility({distance_m: [near, far], ratio?, screen?: {width_mm, width_px},
     * dwell_s?, rate?}) gives minimum text sizes and the word limit, worked out
     * as the legibility check does.
     */
    legibility: (request) => call("opencrs_legibility", request),
    /**
     * dwell({speed_kmh, distance_m: [near, far], ratio?, rate?, screen?,
     * rule_packs?}) gives what a driver can read and each built-in pack's
     * limits at that speed, as the glance and pack checks work them out.
     */
    dwell: (request) => call("opencrs_dwell", request),
    /**
     * publisher(domain, pages, now?) checks a publisher's published specs from
     * pages already fetched, keyed by URL: {status, body, error?, etag?,
     * cache_control?, last_modified?, content_type?, content_encoding?,
     * allow_origin?}. Discovery, trust and linting are the core's.
     */
    publisher: (domain, pages, now = "", missing = "") =>
      call("opencrs_publisher", { domain, pages, now: now || new Date().toISOString(), missing }),
    /** rulePacks() lists the rule packs built into the module. */
    rulePacks: () => call("opencrs_rule_packs", {}),
    /** probe(bytes) is the probe object for one file (throws if it can't be read). */
    probe,
    /** The module's current memory size, in bytes. */
    memoryBytes: () => exports_().memory.buffer.byteLength,
    /** Test hook: makes the module trap (an impossible allocation). */
    _trap: () => guard(() => exports_().opencrs_alloc(0xffffffff)),
    /** Test hook: throws err as if from inside a call. */
    _throw: (err) => guard(() => {
      exports_();
      throw err;
    }),
    /** Test hook: how many instances have been made. */
    _instances: () => instances,
  };
}
