# hivekit (Rust SDK)

Write modules for the Necter network's NDSR runtime in Rust. A module is a
`cdylib` compiled for `wasm32-unknown-unknown` that implements the
`hive-wasm-v1` guest ABI ([`docs/HBC_SPEC.md`](../docs/HBC_SPEC.md)), packaged as a `.hbc`
(`manifest.json` + `module.wasm`) identified by its Keccak-256 content address.

## Quick start

The installer puts prebuilt `ndsr` and `hivec` binaries in `~/.necter/bin` and adds the
`wasm32-unknown-unknown` target. Without it: `rustup target add wasm32-unknown-unknown` and
`cargo install --git https://github.com/Necter-Network/hivekit --tag v1.0.0 hivekit --bin hivec`.

```bash
curl -fsSL https://necter.network/install.sh | sh -s -- rust   # ndsr + hivec, checks rustup
cargo new --lib my_module
```

`Cargo.toml`:

```toml
[lib]
crate-type = ["cdylib", "rlib"]   # rlib lets `cargo test` use the module natively

[dependencies]
hivekit = { git = "https://github.com/Necter-Network/hivekit", tag = "v1.0.0" }
serde = { version = "1", features = ["derive"] }

[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

`src/lib.rs`:

```rust
use hivekit::prelude::*;

/// Exported as "addNumbers" (camelCase of the identifier).
#[hive_export]
fn add_numbers(input: Value) -> Result<Value, String> {
    let a = input["a"].as_i64().ok_or("`a` must be an integer")?;
    let b = input["b"].as_i64().ok_or("`b` must be an integer")?;
    Ok(json!({ "total": a + b }))
}

/// Exported under an explicit name.
#[hive_export("increment")]
fn bump(input: Value) -> Result<Value, String> {
    let by = input["by"].as_i64().unwrap_or(1);
    let n = storage::get_json::<i64>("n").unwrap_or(0) + by;
    storage::set_json("n", &n);
    emit("incremented", &json!({ "value": n }));
    Ok(json!({ "value": n }))
}

// Every exported function, once per crate.
hive_module!(add_numbers, bump);
```

Build, inspect, run:

```bash
hivec build                                   # -> dist/my_module.hbc
hivec inspect dist/my_module.hbc
hivec run dist/my_module.hbc increment '{"by":2}' --data-dir .node   # state persists in .node
```

## Writing functions

`#[hive_export]` accepts `fn(T) -> R` or `fn() -> R`:

- `T` is any `serde::de::DeserializeOwned` type; the input bytes are parsed as
  JSON (empty input is `null`). Invalid input fails the call.
- `R` is `serde_json::Value`, `hivekit::Json<impl Serialize>`, `String` /
  `&'static str` (returned as raw text, not JSON-quoted), `()` (empty output),
  or `Result<any of those, impl Display>`; `Err` aborts the call with the message.

A failed call (error return, `abort`, panic) commits nothing: storage writes
and events of that call are discarded. Panics are reported through `hive.abort`
with their message.

`hive_module!(f, g, ...)` sorts the exports bytewise by name at compile time
(duplicate names are a compile error). The index in that order is the
`func_id` NDSR passes to `__hive_entry`, and it equals the index into the
manifest's sorted `functions` list. The macro also writes the sorted names into
a `hivekit.functions` custom section, which `hivec build` uses for the manifest,
so the two can never disagree. It generates `pub static HIVE_MODULE`.

## Host API

| Function | Host import | Notes |
|---|---|---|
| `storage::get / get_string / get_json` | `storage.get` | `None` if absent; keys are private to the module |
| `storage::set / set_json` | `storage.set` | empty value deletes; key 1..=256 bytes, value ≤ 64 KiB |
| `storage::del` | `storage.del` | |
| `emit(name, &data)` | `hive.emit` | name `[A-Za-z0-9_.:-]{1,64}`; data must be canonical JSON: **no floats**, ints within ±(2^53−1) |
| `call(addr, fn, &[u8])`, `call_json(addr, fn, &Value)` | `hive.call` | `Err(CallError)` for codes −1 not found, −2 no such function, −3 callee failed, −5 depth, −6 bad address |
| `hash(&[u8])` | `crypto.hash` | Keccak-256 as `0x…` |
| `log(&str)` | `console.log` | debug only |
| `abort(&str) -> !` | `hive.abort` | fails the call |

The output wasm imports only these (no WASI); `hivec inspect` lists the imports.

## Testing natively

Off wasm, the same API runs against an in-process mock host with NDSR
semantics (per-module storage namespaces, rollback of failed calls, `hive.call`
to registered modules, event validation):

```rust,ignore
use hivekit::testing;

#[test]
fn increments() {
    testing::reset();
    assert_eq!(HIVE_MODULE.invoke("increment", json!({"by": 2})).unwrap()["value"], 2);
    assert_eq!(testing::events()[0].name, "incremented");
    // testing::register_module("0x…", &other::HIVE_MODULE) makes hive.call targets available.
}
```

The crate's own `tests/ndsr_integration.rs` builds the examples with `hivec`
and executes them on the real `ndsr` binary (inspect, storage persistence,
events, error paths, `hive.call` between two modules). It looks for `$NDSR_BIN`
or a `tools/ndsr` in a parent directory and is skipped only if neither exists.

## hivec

| Command | |
|---|---|
| `hivec build [PATH] [--example NAME] [--out dist] [--name N] [--module-version V] [--description D]` | `cargo build --target wasm32-unknown-unknown --release`, ABI check, `.hbc` |
| `hivec build file.wasm [--functions a,b] [--language L]` | package an existing hive-wasm-v1 module |
| `hivec inspect FILE.hbc [--json]` | verify like NDSR's loader: container, strict manifest, address, imports/exports |
| `hivec functions FILE` | functions with `func_id` from `.hbc`, `.wasm` (custom section) or source |
| `hivec run FILE.hbc FN [INPUT] [--data-dir D] [--gas G] [--module OTHER.hbc]...` | executes on the real runtime by invoking `ndsr run` (`--ndsr`, `$NDSR_BIN`, `ndsr` on `PATH`, or `tools/ndsr` in a parent directory). `--module` places another artifact in `<data-dir>/modules/<address>.hbc` so `hive.call` can reach it |

`hivec run` has no built-in interpreter: results are exactly what an NDSR node
produces, including the signed receipt.

## Examples

`examples/` holds complete modules (`cargo` examples with `crate-type = ["cdylib"]`):

- `math_module`: pure functions, `crypto.hash`, a storage-writing `record`.
- `counter`: storage, events, logging, a failing function, and `hive.call` into `math_module`.
- `price_oracle`: typed input (`#[derive(Deserialize)]`) and `Json<T>` output.

```bash
hivec build --example counter
hivec build --example math_module
hivec run dist/counter.hbc callAdd '{"module":"<math address>","a":1,"b":2}' --module dist/math_module.hbc
```

## License

Apache-2.0. See [LICENSE](../LICENSE) and [NOTICE](../NOTICE).
