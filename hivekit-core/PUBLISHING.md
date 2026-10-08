# Publishing the HiveKit core crates

Releases are currently distributed from GitHub releases only (see the top-level README).
These are the steps for the planned registry publishing.

All crates in this workspace share `workspace.package.version` (root `Cargo.toml`).
`hivekit` (in `hivekit-rs/`) and `hivekit-macro` carry their own version and
depend on `hivekit-core` by `version` + `path`, so bump them together.

## crates.io

Order matters (dependents need their dependencies on the index):

```bash
(cd hivekit-core/crates/hivekit-core && cargo publish)
(cd hivekit-rs/hivekit-macro && cargo publish)
(cd hivekit-rs && cargo publish)
```

## PyPI: `hivekit-core` (import name `hivekit_core`)

The pyo3 extension is deliberately **not** called `hivekit`: that name belongs
to the pure-Python SDK.

```bash
pip install maturin
maturin develop -m hivekit-core/crates/hivekit-py/Cargo.toml   # local install
python hivekit-core/crates/hivekit-py/tests/test_hivekit_core.py
maturin build --release -m hivekit-core/crates/hivekit-py/Cargo.toml
maturin publish -m hivekit-core/crates/hivekit-py/Cargo.toml
```

The wheel is abi3 (CPython ≥ 3.9). `pyproject.toml` enables the
`extension-module` feature; plain `cargo build`/`cargo test` link libpython instead.

## npm: wasm-bindgen package

```bash
rustup target add wasm32-unknown-unknown
cargo build -p hivekit-js --target wasm32-unknown-unknown --release
wasm-bindgen --target nodejs --out-dir pkg \
  target/wasm32-unknown-unknown/release/hivekit_js.wasm      # or: wasm-pack build crates/hivekit-js
node crates/hivekit-js/tests/smoke.cjs pkg                   # checks the test vectors
```

Exports: `packageWasm`, `compile`, `inspect`, `inspectWasm`, `computeAddress`,
`canonicalJson`, `keccak256Hex`, `normalizeAddress`, `detectFunctions`, `runtimeId`.
