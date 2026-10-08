#!/bin/sh
# Reproducible build of runtime/hivekit-js-runtime.wasm (the prebuilt Boa runtime
# shipped in the npm package, so module authors do not need Rust).
#
# Requirements: rustup (toolchain pinned in rust-toolchain.toml), binaryen's
# wasm-opt version 133 (`brew install binaryen` / `apt install binaryen`) and Node.js.
set -eu
cd "$(dirname "$0")"
want="wasm-opt version 133"
got="$(wasm-opt --version)"
case "$got" in "$want"*) ;; *) echo "need $want, found: $got" >&2; exit 1;; esac
# Source paths (this checkout and the cargo registry) end up in panic locations;
# remap them to fixed prefixes so the output does not depend on where it is built.
repo="$(cd ../.. && pwd -P)"
cargo_home="$(cd "${CARGO_HOME:-$HOME/.cargo}" && pwd -P)"
cargo build --release --locked -j "${JOBS:-$(getconf _NPROCESSORS_ONLN)}" \
  --config "target.wasm32-unknown-unknown.rustflags=[\"--remap-path-prefix=$cargo_home=/cargo\", \"--remap-path-prefix=$repo=/necter-sdk\"]"
mkdir -p ../runtime
wasm-opt -Oz --strip-debug --strip-producers \
  --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals \
  target/wasm32-unknown-unknown/release/hivekit_js_runtime.wasm \
  -o target/hivekit-js-runtime.opt.wasm
# Pre-initialize the engine (context + prelude) and snapshot memory into the module.
node snapshot.cjs target/hivekit-js-runtime.opt.wasm ../runtime/hivekit-js-runtime.wasm
ls -l ../runtime/hivekit-js-runtime.wasm
shasum -a 256 ../runtime/hivekit-js-runtime.wasm
