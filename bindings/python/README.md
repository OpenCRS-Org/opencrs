# opencrs

Python bindings for the OpenCRS core (Rust, via PyO3): lint documents, probe
creative files, and resolve screen requirements. OpenCRS is the Open Creative
Requirements Specification: https://opencrs.dev

Early work: version 0.0.0 and not yet published.

```python
import opencrs

opencrs.version()  # "0.1"
opencrs.lint(text)  # {"kind": ..., "schema_errors": [...], "document_checks": [...]}
opencrs.resolve(spec, "publisher.example/network/format")  # list of dicts
opencrs.resolve_strictest(spec, "publisher.example/network/format")  # one dict for every screen at once
opencrs.check([spec], manifest, {"a.png": probe})  # {"reports": [...], "notes": [...]}
opencrs.check_dir(dir, [spec], manifest)  # adds "missing": probes the manifest's files in dir
opencrs.probe("creative.png")  # {"sha256": ..., "bytes": ..., "media_type": ..., "pixels": [w, h]}
```

Errors from the core raise `opencrs.OpenCRSError`, a `ValueError`.

## Develop

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
