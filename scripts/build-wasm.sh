#!/usr/bin/env bash
# Builds the WebAssembly module and its browser loader into OUT_DIR:
#
#   scripts/build-wasm.sh OUT_DIR   ->  OUT_DIR/opencrs.wasm, OUT_DIR/opencrs.js
#
# Deterministic on one host: paths are remapped, codegen-units is 1, and
# wasm-opt is pinned. rustc's output still differs between hosts, so the
# committed opencrs.wasm comes from scripts/build-wasm-docker.sh, which runs
# this script in a pinned container. Needs the wasm32-unknown-unknown target
# (rust-toolchain.toml adds it).
set -euo pipefail

# wasm-opt (binaryen) version this build is verified with. A different
# version can produce different bytes, so CI installs exactly this one.
WASM_OPT_VERSION=133

out=${1:?usage: scripts/build-wasm.sh OUT_DIR}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

have=$(wasm-opt --version | awk '{print $3}')
if [ "$have" != "$WASM_OPT_VERSION" ]; then
  echo "warning: wasm-opt version $have, expected $WASM_OPT_VERSION; the output may differ" >&2
fi

cargo_home=${CARGO_HOME:-$HOME/.cargo}
sysroot=$(rustc --print sysroot)
export RUSTFLAGS="-C codegen-units=1 --cfg getrandom_backend=\"custom\" --remap-path-prefix=$root=/opencrs --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$sysroot=/rustc"
# Profile settings live here, not in Cargo.toml, so they only affect this build.
# Built for speed: about 40% faster than opt-level z, at twice the size.
export CARGO_PROFILE_RELEASE_OPT_LEVEL=3
export CARGO_PROFILE_RELEASE_LTO=true
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
export CARGO_PROFILE_RELEASE_PANIC=abort
export CARGO_PROFILE_RELEASE_STRIP=true

cargo build -p opencrs-wasm --release --target wasm32-unknown-unknown --locked

mkdir -p "$out"
wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-simd \
  --strip-debug --strip-producers \
  -o "$out/opencrs.wasm" "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/opencrs_wasm.wasm"
cp js/opencrs.js "$out/opencrs.js"

raw=$(wc -c < "$out/opencrs.wasm" | tr -d ' ')
gz=$(gzip -9 -c "$out/opencrs.wasm" | wc -c | tr -d ' ')
echo "opencrs.wasm: $raw bytes, $gz gzipped"
