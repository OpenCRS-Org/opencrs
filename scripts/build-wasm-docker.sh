#!/usr/bin/env bash
# Builds the WebAssembly module in the canonical environment, a pinned
# linux/amd64 Rust 1.95.0 container with binaryen 133, so every machine
# (a Mac, a Linux laptop, CI) produces the same bytes:
#
#   scripts/build-wasm-docker.sh OUT_DIR   ->  OUT_DIR/opencrs.wasm, OUT_DIR/opencrs.js
#
# The committed opencrs.wasm must come from this script; CI rebuilds it here
# and compares. rustc's output differs between hosts (macOS and Linux, arm64
# and x86_64), so scripts/build-wasm.sh run directly on a workstation is for
# testing only. Needs Docker; on Apple silicon the container runs emulated.
set -euo pipefail

# rust:1.95.0-bookworm, linux/amd64 manifest.
IMAGE=rust:1.95.0-bookworm@sha256:4c2fd73ef19c5ef9d54bee03b06b2839a392604fbfcd578ed948b71b37c1d7fb
BINARYEN_URL=https://github.com/WebAssembly/binaryen/releases/download/version_133/binaryen-version_133-x86_64-linux.tar.gz
BINARYEN_SHA256=2dc9c7813f5375db93d96ead4b78222fcc3e2677bbb832297af4797782a37489

out=${1:?usage: scripts/build-wasm-docker.sh OUT_DIR}
root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)

# The sources are mounted read-only; the build happens in the container's
# own target directory. RUSTUP_TOOLCHAIN uses the image's toolchain rather
# than letting rust-toolchain.toml fetch another.
docker run --rm --platform linux/amd64 \
  -v "$root:/opencrs:ro" -v "$out:/out" \
  -e RUSTUP_TOOLCHAIN=1.95.0 -e CARGO_TARGET_DIR=/tmp/target \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  -w /opencrs "$IMAGE" bash -euo pipefail -c '
    curl -fsSL "$1" -o /tmp/binaryen.tar.gz
    echo "$2  /tmp/binaryen.tar.gz" | sha256sum -c - >/dev/null
    tar -xzf /tmp/binaryen.tar.gz -C /tmp
    export PATH="/tmp/binaryen-version_133/bin:$PATH"
    rustup target add wasm32-unknown-unknown >/dev/null 2>&1
    scripts/build-wasm.sh /out
    chown "$HOST_UID:$HOST_GID" /out/opencrs.wasm /out/opencrs.js
  ' build "$BINARYEN_URL" "$BINARYEN_SHA256"

if [ ! -s "$out/opencrs.wasm" ]; then
  echo "error: $out/opencrs.wasm isn't there: Docker can't share $out with the container (with Colima, use a directory under your home)" >&2
  exit 1
fi
