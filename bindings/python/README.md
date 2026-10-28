# opencrs

Python bindings for the OpenCRS core (Rust, via PyO3): lint documents, probe
creative files, resolve screen requirements, work out legibility and dwell, and
check a publisher's published specs. OpenCRS is the Open Creative
Requirements Specification: https://opencrs.dev

Early work: the API may change before 1.0.

## Install

```sh
pip install opencrs
```

Prebuilt wheels for macOS (arm64, x86-64), Linux (glibc and musl, arm64 and
x86-64) and Windows (x86-64), for Python 3.9 or later. Documentation:
https://opencrs.dev

```python
import opencrs

opencrs.version()  # "0.1"
opencrs.lint(text)  # {"kind": ..., "schema_errors": [...], "document_checks": [...]}
opencrs.resolve(spec, "publisher.example/network/format")  # list of dicts
opencrs.resolve_strictest(spec, "publisher.example/network/format")  # one dict for every screen at once
opencrs.check([spec], manifest, {"a.png": probe})  # {"reports": [...], "notes": [...]}
opencrs.check_dir(dir, [spec], manifest)  # adds "missing": probes the manifest's files in dir
opencrs.probe("creative.png")  # {"sha256": ..., "bytes": ..., "media_type": ..., "pixels": [w, h]}

# Minimum text sizes, and what a driver can read, as the checks work them out.
opencrs.legibility([2, 6], screen={"width_mm": 940, "width_px": 1080})["key"]["px"]  # 46
opencrs.dwell(80, [20, 150], rule_packs=["au-nsw-transport-corridor@2017-11"])["glance"]["words"]  # 4
opencrs.rule_packs()  # {"rule_packs": [{"ref": ..., "title": ..., "status": ...}, ...]}

# Check a publisher's specs from pages you fetched, or let it fetch them.
opencrs.publisher("harbourline.example", pages)  # {"errors": ..., "advice": [...], "wanted": [...], ...}
opencrs.fetch_publisher("harbourline.example")  # the same result, after fetching
opencrs.discover("harbourline.example")  # which documents to trust, as `opencrs fetch` says
```

Errors from the core raise `opencrs.OpenCRSError`, a `ValueError`. The
calculators, `rule_packs` and `publisher` give the same data as the browser
build (`js/opencrs.js`) and Go for the same input, so `rule_packs()` has its
list under `"rule_packs"`.

## Fetching a publisher

`publisher(domain, pages)` takes pages you have fetched, keyed by URL
(`{"status": 200, "body": ..., "etag": ..., "cache_control": ...}`, with a `str`
or `bytes` body); its `"wanted"` lists the URLs it still needs.
`fetch_publisher(domain)` runs that loop for you, releasing the GIL while it
waits on the network. The publisher chose every URL, so it fetches as carefully
as the `opencrs` command line does: HTTPS only, no connection to a loopback,
private, link-local or other non-public address (checked after DNS and on every
redirect), no proxy, and caps on the size and number of pages
(`max_document_bytes`, `max_total_bytes`, `max_rounds`, `max_pages`, `timeout`).
`discover(domain)` gives only the trust decisions; `allow_local_http=True` lets
it reach `localhost` over `http://` for testing.

## Develop

From a clone of https://github.com/OpenCRS-Org/opencrs, in `bindings/python`:

```sh
uv run --with maturin maturin develop
uv run pytest
```

## File names in manifests

`check_dir` treats the manifest as untrusted. A file name that is absolute or
contains `..` is an error, as is a symlink that leaves `dir` or anything that
isn't a regular file (a FIFO, a device, a directory). A file that doesn't
exist is judged on the manifest's declared values instead and listed in the
result's `"missing"` (every file is, if `dir` itself doesn't exist): check it
before trusting a pass. `probe` also refuses non-regular files.
