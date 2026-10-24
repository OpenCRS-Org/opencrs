# OpenCRS

The Open Creative Requirements Specification: one machine-readable file, on a publisher's own domain, that says what creative needs in order to work on each of its digital screens.

This repository holds the [0.1 specification](OpenCRS%200.1%20Specification.md), its JSON Schemas, worked examples, and a reference validator written in Rust, with a Go package that runs the validator's WebAssembly build.

| Path | What's in it |
| --- | --- |
| `schema/0.1/` | JSON Schemas (Draft 2020-12): `index`, `screens`, `creative`, `report`, `rule-pack`, plus `common` definitions |
| `examples/harbourline/` | The spec's complete example (verbatim), an index, a creative manifest with two PNGs, and the reports the validator writes for them |
| `crates/opencrs/` | Rust library: schema validation, inheritance resolution, file probing, checks, reports, and discovery and trust (fetching indexes and documents, delegation, partner merging) |
| `rulepacks/` | Regulatory rule packs (`<name>@<version>.json`), embedded in the validator |
| `conformance/` | The conformance suite: test cases any validator can run to show it meets Core, Layout or Full |
| `crates/opencrs-cli/` | The `opencrs` command-line tool |
| `crates/opencrs-wasm/` | The validator compiled to WebAssembly, for the online checker at [opencrs.dev/check](https://opencrs.dev/check/) |
| `bindings/` | Python and npm packages over the Rust library |
| `js/` | Loader, conformance adapter and smoke test for the WebAssembly build |
| `opencrs.go` | The Go package (`opencrs.dev/go`): the validator's WebAssembly build (`opencrs.wasm`, embedded) behind a Go API |

## Install

```sh
brew install opencrs-org/tap/opencrs                                  # macOS and Linux
scoop bucket add opencrs https://github.com/OpenCRS-Org/scoop-bucket  # Windows
scoop install opencrs
npx @opencrs/cli --help                                               # anywhere with Node.js
docker run --rm -v "$PWD:/work" ghcr.io/opencrs-org/opencrs --help    # container
```

Or the installer script, which puts `opencrs` in `~/.cargo/bin`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/OpenCRS-Org/opencrs/releases/latest/download/opencrs-cli-installer.sh | sh
```

Each [release](https://github.com/OpenCRS-Org/opencrs/releases) also has archives for macOS, Linux (glibc and static musl) and Windows, with checksums and build attestations.

## The validator

```sh
cargo build --release -p opencrs-cli    # target/release/opencrs: a single binary; no runtime needed
```

Run `opencrs --help` for the commands and `opencrs COMMAND --help` for a command's options. The single-dash spellings of earlier versions (`-spec`, `-json`, `-allow-local-http`) still work.

A man page (`man/opencrs.1`) and shell completions (`completions/`) are generated from the command definition and included in release archives; the Homebrew formula installs them. `opencrs completion bash|zsh|fish|powershell` prints a completion script. After changing the command line, run `cargo xtask generate` and commit the result; a test fails if they are out of date.

**Check documents against the schema** (add `--allow-local-http` to test an index that points at `http://localhost`). For a `screens.json`, this also reports screens that name a missing format or venue (`CRS-REFERENCE`) and rules that inheritance has made impossible to meet (`CRS-RULE-CONFLICT`).

```sh
opencrs schema examples/harbourline/retail.screens.json examples/harbourline/creative.json
```

**Check creative against a publisher's requirements**

```sh
opencrs check --spec examples/harbourline/retail.screens.json examples/harbourline/creative.json
```

```
lunch-portrait.png → harbourline.example/retail/portrait-totem-75: FAIL
  warning     CRS-LEGIBILITY         Legal text is 18 px; 16 px minimum at 2 m, ratio 150. Passes with little margin  [HC-L1-001]
  warning     CRS-LEGIBILITY         Headline text is 92 px; 77 px minimum at 10 m, ratio 150. Passes with little margin  [HC-L1-002]
  error       CRS-LEGIBILITY         Legal text is 18 px; 23 px minimum at 3 m, ratio 150  [HC-L1-002]
```

The same file passes on one totem and fails on its twin, because the second is seen from further away. That is the point of describing screens as people see them.

The validator reads each file's type, pixel size, byte size and SHA-256 (and duration, for MP4) from the file itself. Values the manifest declares are used only when the file isn't available (`--no-files`). Use `--json` to get a JSON array of reports, each in the spec's report format, and `--spec` more than once to check against several publishers. Exit status is 1 if any report fails.

**Discover a publisher's documents and check which to trust**

```sh
opencrs fetch harbourline.example                       # read the index, fetch every document, follow delegates
opencrs fetch https://specs.cms-a.example/harbourline/airports.screens.json   # does its publisher vouch for this exact URL?
opencrs fetch --token harbourline.example=TOKEN --output specs harbourline.example         # include partner documents and save
```

Each document is marked `trusted`, `expired`, `rejected`, `skipped` (a partner document without credentials) or `failed`, with the reason. `--output` saves trusted documents byte-for-byte as served, ready for `opencrs check -spec`. When partner documents are included, it also writes a `merged.screens.json` and lists any conflicts. Use `--allow-local-http` to test an index on `http://localhost` before deploying it.

`fetch` refuses to connect to loopback, private, link-local, shared (CGNAT), multicast, documentation and other internal addresses, checked after DNS resolution and again on every redirect, so an index can't aim it at a machine's own services or its network. `-allow-local-http` is the opt-in for local testing: it allows `http://` and connections to `localhost`, `127.0.0.1` and `[::1]` only. `fetch` connects directly and ignores `HTTP(S)_PROXY`.

Limits: each document is at most 50 MiB (the spec's limit), an index may list at most 1,000 documents and 1,000 delegates, and one run reads at most 512 MiB in all; documents past that are marked `failed`.

**See what a screen actually requires after inheritance**

```sh
opencrs resolve examples/harbourline/retail.screens.json harbourline.example/retail/screen/HC-L1-002
```

This prints the merged format, rules and derived values: minimum key and legal cap heights in mm and px, and the word limit.

**Checks implemented:** `CRS-PIXELS`, `CRS-MEDIA-TYPE`, `CRS-FILE-SIZE`, `CRS-DURATION`, `CRS-SAFE-AREA`, `CRS-BLOCKED-REGION`, `CRS-LEGIBILITY`, `CRS-MAX-WORDS`, `CRS-REQUIRED-ELEMENT`, `CRS-CATEGORY`, `CRS-MOTION` and `CRS-SPEC-STALE`. That covers all three conformance levels (Core, Layout and Full). Not yet implemented: segment checks for composite surfaces, and audio, codec and colour-space checks.

**Regulatory rule packs** are built into the validator (and the browser build) from `rulepacks/`, so a screen that references one, such as `"rule_packs": ["au-nsw-transport-corridor@2017-11"]`, is checked against it with no extra flags. Creative rules give `CRS-MOTION-PROHIBITED`, `CRS-TRAFFIC-DEVICE` (always needs visual review), `CRS-GLANCE-TEXT` and `CRS-SEQUENCING`; playback rules (dwell, transition, failure state, luminance, school zones) appear as `CRS-PLAYBACK` info lines chosen for the screen's speed and lighting zone, and never fail creative. A pack the validator doesn't know is reported `CRS-RULE-PACK` not checked. Packs for NSW, Queensland, WA, SA, Victoria and Tasmania are each checked against the state's own guidelines or planning rules, with page or section citations; all are `draft` except Tasmania's, which stays `unverified` until amendments made after its June 2024 edition are checked. Each pack's `notes` say what it leaves out. The NT and the ACT have no territory-wide rule for digital signs, so they have no pack. Validate a pack with `opencrs schema rulepacks/<pack>.json`.

**Run the conformance suite**

```sh
opencrs conformance                       # the Rust validator, at the Full level
opencrs conformance --cmd ./my-adapter.sh  # any other validator, through an adapter
```

See [`conformance/README.md`](conformance/README.md) for the case format, the matching rules and how to write an adapter.

## The browser validator

`crates/opencrs-wasm` compiles the validator to WebAssembly. The online checker at [opencrs.dev/check](https://opencrs.dev/check/) runs it in the browser, so creative files are never uploaded; its page lives in the site's repository, OpenCRS-Org/opencrs.dev.

```sh
scripts/build-wasm.sh /tmp/wasm      # needs binaryen's wasm-opt; writes opencrs.wasm and opencrs.js
node js/smoke.mjs /tmp/wasm          # the module must behave like the CLI
```

The committed `opencrs.wasm` at the repository root is the build of these sources and is what the Go package embeds. rustc's output differs from one host to another, so it is built in a pinned linux/amd64 container (Rust 1.95.0, binaryen 133) with `scripts/build-wasm-docker.sh OUT_DIR` (needs Docker), and CI rebuilds it there and compares byte for byte. A build with `scripts/build-wasm.sh` on your own machine is for testing; commit the container's.

## Development

```sh
cargo test --workspace     # the Rust library and CLI
go test -race .            # the Go package
```

The tests use the spec's own numbers as acceptance criteria: the worked legibility example (46 px and 16 px), text just below a minimum failing rather than rounding up to pass, restrictions that a lower level can't loosen, the 8-word limit from a 4-second dwell, larger text on the further totem, and every JSON example in the spec passing its schema (`crates/opencrs/tests/spec_examples.rs`), with the complete example identical to the copy in `examples/`.

The validator used to be written in Go. The last commit that had it is `6784da9`; `crates/opencrs-cli/tests/differential/README.md` explains how to build it and compare it with the Rust CLI.

## Licence

- **The specification** (`OpenCRS 0.1 Specification.md`): [CC BY 4.0](LICENSE-SPEC).
- **Everything else**, including the JSON Schemas, examples and code: [Apache 2.0](LICENSE).

Anyone can implement OpenCRS, including competitors, as the spec's governance section intends.

To contribute, see [CONTRIBUTING.md](CONTRIBUTING.md): commits are signed off under the [Developer Certificate of Origin](DCO). To report a security problem, see [SECURITY.md](SECURITY.md).

The Go module path is `opencrs.dev/go`, a vanity import path: `opencrs.dev` tells the `go` tool that the code lives at `github.com/OpenCRS-Org/opencrs`, so hosting can move without breaking imports. Add it with `go get opencrs.dev/go`.

**Checking a creative package from someone else.** Every host (the CLI, the npm and Python bindings, and the Go package's `CheckDir`) reads only the files the manifest names, refuses names with `..` or an absolute path, follows symlinks only while they stay inside the package directory, and probes only regular files. In Go, pass `root.FS()` from `os.OpenRoot(dir)` rather than `os.DirFS(dir)`, so the opens themselves are confined too. Two limits remain: a hard link inside the package can't be told from an ordinary file by any host, so unpack untrusted archives without hard links (most extractors have an option; Linux's `protected_hardlinks` also blocks links to other users' files); and a concurrent writer to the directory could swap in a symlink between the check and the open, so check a copy no one else can write to.
