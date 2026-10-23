# Security

## Reporting a problem

Please report security problems privately, not in a public issue or pull request.

Use GitHub's private reporting: on the repository's **Security** tab, choose **Report a vulnerability**. Only the maintainers can see the report.

Include what you found, how to reproduce it (the documents or files that trigger it help most), and what an attacker could do with it. We'll acknowledge your report, keep you updated while we work on a fix, and credit you when it's published, unless you'd rather not be named.

## What's in scope

- **The reference validator:** the `opencrs` command, the Rust crates in `crates/` (including discovery and fetching), the Go package that embeds the WebAssembly build, and the WebAssembly build used by the online checker. For example: a document or creative file that crashes the validator, makes it hang or use unbounded memory, or makes it read or fetch something it shouldn't.
- **Discovery and trust:** fetching indexes and documents from publishers' domains, redirects, delegation and partner merging. For example: a way to make a validator accept a `screens.json` that the publisher's own index doesn't list.
- **The specification itself:** a flaw in the trust model that any correct implementation would share, such as a way to impersonate a publisher. Report these privately too, so the fix can be agreed before it's public.
- **The JSON Schemas and rule packs,** where a mistake would let a document pass that the spec says must fail.

Problems with the opencrs.dev website can be reported the same way.

## Supported versions

OpenCRS is at 0.x. Fixes go into the latest release only; there are no backports to earlier 0.x versions.
