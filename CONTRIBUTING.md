# Contributing to OpenCRS

Thanks for helping. OpenCRS is an open standard, and it gets better from real spec sheets, real screens and real validators. Issues and proposals are open to anyone.

## Ways to contribute

- **Questions and problems with the spec.** Open an issue. Say what you were trying to describe and where the spec got in the way; a real (or realistically anonymised) spec sheet helps most. Only share what you're allowed to: a media owner's spec sheet may be confidential, or list screen locations they don't publish. If in doubt, anonymise it or describe the problem instead. Issues and pull requests are public, and signing off a commit (see below) says you have the right to submit what's in it.
- **Changes to the spec.** Open an issue first, so the idea can be discussed before anyone writes the wording. `OpenCRS 0.1 Specification.md` is the master copy of the spec; change it there, along with the schemas, examples and tests it affects.
- **Rule packs.** Each pack encodes one regulation or policy from its primary source. A new or changed pack needs a page citation for every value, and stays `draft` until someone independent of its author has reviewed it (see "Rule packs" in the spec).
- **Conformance cases.** A case in `conformance/` that pins down a rule the suite doesn't yet test is one of the most useful contributions. See [`conformance/README.md`](conformance/README.md).
- **The reference validator.** Bug reports with the documents that trigger them, and fixes.

Security problems are different: please don't open a public issue. See [SECURITY.md](SECURITY.md).

## Contribution terms

You keep the copyright in your contribution. By contributing, you agree that it's licensed under the same terms as the part of the repository it changes:

- changes to the specification (`OpenCRS 0.1 Specification.md`) under [CC BY 4.0](LICENSE-SPEC)
- everything else, including the JSON Schemas, rule packs, examples, conformance cases and Go code, under [Apache 2.0](LICENSE)

That means anyone can use and implement your contribution, without asking and without paying, now and after OpenCRS moves to a neutral body.

### Sign off your commits

Every commit must be signed off under the [Developer Certificate of Origin](DCO) (DCO). The DCO is a short statement that you wrote the contribution, or otherwise have the right to submit it under the project's licences. Read it before your first contribution.

To sign off, add a `Signed-off-by` line to each commit message, using the name and email you're known by:

```
Signed-off-by: Alex Example <alex@example.com>
```

Git adds it for you with `-s`:

```sh
git commit -s -m "Add the 2025 edition of the WA rule pack"
```

If you forgot, `git commit --amend -s` fixes the last commit, and `git rebase --signoff main` fixes every commit on your branch. A check on each pull request fails until every commit is signed off.

There's no contributor licence agreement (CLA) to sign. The sign-off is the whole agreement.

## Making a change

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace                       # includes every JSON example in the spec
cargo run -p opencrs-cli -- conformance      # the reference validator must stay at Full
gofmt -l .                                   # the Go package: must print nothing
go vet .
go test -race .
scripts/build-wasm.sh /tmp/wasm && node js/smoke.mjs /tmp/wasm   # the WebAssembly build (for testing)
scripts/build-wasm-docker.sh target/wasm && cp target/wasm/opencrs.wasm .   # the committed module, if you changed the core
```

CI runs these on every pull request, plus a check that the committed `opencrs.wasm` is what the sources build and gives the same reports as the CLI.

- **Keep examples fictional.** Use made-up organisations on `.example` domains (such as Harbourline and Baycrest), never real companies or people. A test fails on any example host that isn't a `.example` domain.
- **Write for both readers.** Plain words first, technical detail underneath, in the spec and in error messages alike.
- **One change per pull request,** with commit messages in the imperative ("Add X", not "Added X") that explain why when it isn't obvious.

## Who decides

While OpenCRS is at 0.x, Mestra maintains it: maintainers review proposals and pull requests in the open and make the final call. The plan is to hand the standard to a neutral industry body once several publishers and validators use it (see "Governance" in the spec). The contribution terms above are chosen so that handover needs nothing more from contributors.

## Releases (maintainers)

Pushing a version tag runs `.github/workflows/release.yml`, which cargo-dist generates from `dist-workspace.toml` (run `dist generate`; don't edit it by hand), and `.github/workflows/release-extra.yml` after it. The repository needs:

- `HOMEBREW_TAP_TOKEN`: pushes the formula to `OpenCRS-Org/homebrew-tap`.
- `SCOOP_BUCKET_TOKEN`: pushes the manifest to `OpenCRS-Org/scoop-bucket`.
- `WINGET_TOKEN`: opens the update PR in `microsoft/winget-pkgs`.
- An npm Trusted Publisher configured for `@opencrs/cli`: repository `OpenCRS-Org/opencrs`, workflow `release.yml`. npm is published by OIDC with provenance; there is no npm token.

The container image goes to GHCR with the workflow's own `GITHUB_TOKEN`.
