# Differential test

`differential.rs` runs the Go `opencrs` binary (the oracle) and this crate's
`opencrs` on the same inputs and compares exit code, stdout and stderr.

The Go validator has been removed from the tree, so CI no longer runs this
test. The last commit that contained it is `6784da9`. To rerun the comparison
against it, build the oracle from that commit in a separate worktree:

```sh
git worktree add /tmp/opencrs-oracle 6784da9
(cd /tmp/opencrs-oracle && go build -o /tmp/opencrs-go ./cmd/opencrs)
OPENCRS_ORACLE=/tmp/opencrs-go cargo test -p opencrs-cli --test differential -- --nocapture
```

The last full run, immediately before the Go validator was removed, compared
every invocation (all 13 tests, including the 20000-screen bench network and
about 3000 mutated documents) with no differences.

Without `OPENCRS_ORACLE` the tests print "skipped" and pass. `uv` is needed
for the bench network (`bench/gen-network.py`); without it that test skips.
Set `OPENCRS_DIFF_LOG=FILE` to write every difference to a file.

Inputs: `examples/`, every conformance case (documents, and the adapter's
`check` invocation), `crates/opencrs/tests/data`, the bench network
(20000 screens, 400 formats), usage and flag errors, and deterministic
mutations of the Harbourline screens.json and creative.json (each field
removed, each number set to -1, 0, 1e9 and a string, each id reference set to
`missing-id`, text/id/name set to non-ASCII text). The `check_*` tests cover `check`.

## What is not compared byte for byte

- Help and usage errors. The CLI now parses with clap, so its help text and
  usage errors differ from Go's: for those cases only the exit status is
  compared (2 for a usage error with no stdout; 0 and help on stdout when
  help is asked for, where Go exited 2 for a command's `-h`).

- (a) Schema errors: lines quoting one are reduced to their instance paths and
  the count is dropped. Paths under a node Go reports as "validation failed"
  fold into that node.
- (b) `-json` output: compared as JSON after removing `validator` and
  `checked_at`.
- (c) A document that matches the schema but can't be decoded (for example
  `1e9` for an integer field): only the fact is compared, not Go's or serde's
  wording.

## Known, accepted differences

- Go's `encoding/json` matches keys case-insensitively (`"Pixels"`); Rust
  doesn't. The corpus excludes such inputs.
- `fetch` is covered against a local HTTP server (`-allow-local-http`); network-level error text (connection refused, timeouts) and JSON syntax errors are excluded because their wording differs.
- `conformance` is covered including `-cmd`, with each binary running the README's adapter against the other (needs `jq`). The wording of a JSON syntax error in a validator's output differs and is compared only as "output isn't a JSON array of reports".
- `-margin` does not accept hexadecimal floats or underscores.

## Intentional differences since the oracle (security fixes, 2026-10-24)

These change behaviour on purpose; a rerun against `6784da9` will show them.

- `fetch DOMAIN` and a document's `publisher` must be a bare domain (the
  schema's form), or a local host in `-allow-local-http` mode: anything else
  is "isn't a publisher domain" before any request.
- `fetch` refuses loopback, private, link-local and other internal addresses
  after resolving each host, redirects included ("refusing to connect to
  ..."); `-allow-local-http` lets localhost, 127.0.0.1 and [::1] through. Its
  help text says so, and proxies from the environment are ignored.
- `same_url` also compares the parsed URLs, and never matches text with `\`.
- A listed document on another host than the listing domain's is rejected
  with the path of its first schema error, not the error's text, and with no
  value from a type error.
- An index may list at most 1,000 documents and 1,000 delegates, and one run
  reads at most 512 MiB; later documents are `failed`.
- `fetch -o` names files from the parsed URL's last segment, allowing only
  `[A-Za-z0-9._-]` and no reserved Windows names; anything else is
  `document.json`.
- The probe reads JPEG and MP4 headers in 4 KiB windows and stops after
  10,000 reads. Results match the old walk (a randomised test compares them)
  except for files built to need more reads than that.
