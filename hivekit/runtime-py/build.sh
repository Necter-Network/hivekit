#!/bin/sh
# Reproducible build of hivekit/runtime/hivekit-py-runtime.wasm (the prebuilt
# RustPython runtime shipped in the Python package, so module authors do not
# need Rust).
#
# Requirements: rustup (toolchain pinned in rust-toolchain.toml), binaryen's
# wasm-opt version 133, and a clang with the WebAssembly backend for the small
# C/asm parts of dependencies (psm): Homebrew `llvm`, or set CLANG=/path/to/clang;
# Node.js for the pre-initialization snapshot.
#
# The output is byte-reproducible: the repository and CARGO_HOME paths (which
# end up in panic locations) are remapped to fixed prefixes, the build time is
# pinned (build-env/git, TZ=UTC), and vendor/ fixes HashMap-order codegen.
# Expected sha256 of the shipped runtime: see README.md, "Rebuilding the runtime".
set -eu
cd "$(dirname "$0")"
want="wasm-opt version 133"
got="$(wasm-opt --version)"
case "$got" in "$want"*) ;; *) echo "need $want, found: $got" >&2; exit 1;; esac
CLANG="${CLANG:-$(brew --prefix llvm 2>/dev/null)/bin/clang}"
LLVM_AR="${LLVM_AR:-$(dirname "$CLANG")/llvm-ar}"
export CC_wasm32_unknown_unknown="$CLANG" AR_wasm32_unknown_unknown="$LLVM_AR"
repo="$(cd ../.. && pwd -P)"
cargo_home="$(cd "${CARGO_HOME:-$HOME/.cargo}" && pwd -P)"
# Extra rustflags are appended to the ones in .cargo/config.toml.
PATH="$PWD/build-env:$PATH" TZ=UTC cargo build --release --locked -j "${JOBS:-$(getconf _NPROCESSORS_ONLN)}" \
  --config "target.wasm32-unknown-unknown.rustflags=[\"--remap-path-prefix=$cargo_home=/cargo\", \"--remap-path-prefix=$repo=/necter-sdk\"]"
mkdir -p ../hivekit/runtime
wasm-opt -Oz --strip-debug --strip-producers \
  --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals \
  target/wasm32-unknown-unknown/release/hivekit_py_runtime.wasm \
  -o target/hivekit-py-runtime.opt.wasm
if [ "${SNAPSHOT:-1}" = 1 ]; then
  # Pre-initialize the interpreter and snapshot memory into the module (same
  # tool as the JavaScript runtime), so calls skip interpreter start-up. The
  # result is ~9.6 MiB, within the 12 MiB module.wasm limit (HBC_SPEC §2).
  # SNAPSHOT=0 builds the plain runtime (~7.6 MiB, start-up on every call).
  node ../../hivekit-js/runtime-js/snapshot.cjs target/hivekit-py-runtime.opt.wasm ../hivekit/runtime/hivekit-py-runtime.wasm
else
  cp target/hivekit-py-runtime.opt.wasm ../hivekit/runtime/hivekit-py-runtime.wasm
fi
ls -l ../hivekit/runtime/hivekit-py-runtime.wasm
shasum -a 256 ../hivekit/runtime/hivekit-py-runtime.wasm
