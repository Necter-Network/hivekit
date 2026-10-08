# hivekit-core

The language-neutral core shared by every HiveKit SDK. It implements the
artifact side of NDSR's `hive-wasm-v1` specification (`docs/HBC_SPEC.md`):

- **Manifest** (`manifest`): the strict schema. Required `name`, `language`,
  `compiler`, `runtime` (`"hive-wasm-v1"`), `functions` (sorted bytewise,
  unique); optional `version`, `description`, `manifest_address`. Any other key
  is rejected, so are duplicate keys and floats.
- **Content address** (`manifest`, `canonical`):
  `0x` + hex(keccak256(canonical_json(manifest − manifest_address) ‖ module.wasm)),
  with Ethereum Keccak-256 and HBC_SPEC canonical JSON (sorted keys, no
  whitespace, non-ASCII unescaped, integers only).
- **ABI checks** (`abi`): the wasm may import only the hive-wasm-v1 host
  functions (no WASI) and must export `memory`, `__alloc(i32)->i32` and
  `__hive_entry(i32,i32,i32)->i64`.
- **`.hbc` container** (`hbc`): exactly `manifest.json` + `module.wasm`,
  written deterministically (stored, fixed timestamps) and verified on load.
- **Packaging** (`compiler`): `package_wasm` turns a compiled module into a
  `.hbc`. Source text is never packaged; `compile` only uses source to discover
  function names (`registry`).

`tests/vectors.rs` reproduces NDSR's `test-vectors.json` (canonical JSON,
Keccak-256, the `vector_echo` manifest address and its exact `.hbc` bytes).

```rust,no_run
use hivekit_core::{package_wasm, load_hbc_strict, PackageWasmOptions};

let wasm = std::fs::read("target/wasm32-unknown-unknown/release/my_module.wasm").unwrap();
let r = package_wasm(PackageWasmOptions {
    name: "my_module".into(),
    language: "rust".into(),
    wasm_bytes: wasm,
    functions: None, // read from the module's `hivekit.functions` section
    ..Default::default()
})
.unwrap();
std::fs::write("dist/my_module.hbc", &r.hbc_bytes).unwrap();
assert_eq!(load_hbc_strict(&r.hbc_bytes).unwrap().manifest_address, r.manifest_address);
```

Bindings built from this workspace:

- `crates/hivekit-py`: Python extension module `hivekit_core` (maturin).
- `crates/hivekit-js`: wasm-bindgen package for JavaScript.

## License

Apache-2.0. See [LICENSE](../../../LICENSE) and [NOTICE](../../../NOTICE).
