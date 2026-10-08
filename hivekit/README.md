# hivekit (Python)

Write Necter modules in Python. `hivec build` produces a `.hbc` artifact that
conforms to `hive-wasm-v1` ([`docs/HBC_SPEC.md`](../docs/HBC_SPEC.md)): one
`module.wasm` with no WASI imports, a manifest with only the allowed keys,
sorted functions, and a Keccak-256 content address over canonical JSON.

## Install

```bash
curl -fsSL https://necter.network/install.sh | sh -s -- python    # ndsr, checks Python >= 3.10
python3 -m venv .venv && . .venv/bin/activate
pip install https://github.com/Necter-Network/hivekit/releases/download/v1.0.0/necter_hivekit-1.0.0-py3-none-any.whl
```

The distribution is named `necter-hivekit` and is installed from GitHub releases; the import name is
`hivekit`. The unrelated PyPI project called `hivekit` uses the same import name, so do not install both
in one environment. Dependencies: `click` and `pycryptodome`.

From a checkout: `pip install -e './hivekit[test]'`.

## Write a module

```python
# counter.py
from hivekit import hive

@hive.define("increment")
def increment(input):
    count = hive.db.get("count", 0) + input.get("by", 1)
    hive.db.set("count", count)
    hive.emit("incremented", {"count": count})
    return {"count": count}

@hive.define("relay")
def relay(input):
    return hive.call(input["address"], input["function"], input.get("input", ""))

@hive.define("note")
def note(ctx):                   # first parameter named ctx -> context object
    ctx.storage.set("note", ctx.input["text"])
    ctx.log.info("saved")
    return {"hash": hive.hash(ctx.input["text"])}
```

Handler styles: `fn(input)`, `fn(ctx)` (first parameter named `ctx`),
`fn(input, ctx)`, or `fn()`.

Conventions (identical in the JavaScript SDK):

- **Input**: the call input parsed as JSON; the raw string if it is not JSON; `{}` if empty.
- **Output**: a returned `str` is the output as-is; `None` → `""`; anything else → compact JSON (`ensure_ascii=False`).
- **Errors**: an exception or `hive.fail(msg)` fails the call; its state changes and events are discarded and the message is recorded in the receipt.
- `hive.call(address, fn, input)` returns the callee output decoded like input and raises `HiveCallError` (`.code`, HBC_SPEC §6.5); `hive.call_raw` returns the raw string; `hive.try_call` returns `None`.
- `hive.storage` holds raw strings; `hive.db` (and `ctx.db`) stores JSON values in the same state.
  Key listing (`db.keys()`, `db.all()`) and `hive.files` work only in local runs: the node ABI has no key enumeration and no file access.
- `hive.config({...})["environment"]` is available as `ctx.env`. Nothing from `hive.config`,
  `hive.schedule` or `NRC1Config` is written to the manifest (HBC_SPEC §3 forbids extra keys).
- `@hive.consensus(NRC1Config(...))` registers the `__consensus` function.

## Build, inspect, run

```bash
hivec build counter.py                      # -> dist/counter.hbc (+ counter.manifest.json)
hivec inspect dist/counter.hbc              # uses `ndsr inspect` when available
hivec run counter.py increment '{"by": 2}'  # builds, then `ndsr run`
hivec run counter.py increment '{"by": 2}' --data-dir .state   # persist state
hivec run --local counter.py increment '{"by": 2}'            # in-process, no node
hivec functions counter.py                  # func_id -> name
```

`hivec run` delegates to the NDSR binary when it finds one (`$NDSR_BIN`,
`ndsr` on `PATH`, or `tools/ndsr` in a parent directory): real gas, receipts,
storage, events and `hive.call` (targets are resolved from
`<data-dir>/modules/<address>.hbc`). Otherwise it imports the source in this
process with an in-memory store.

From Python: `compile_file(path)`, `compile_source(text, name)`,
`read_hbc(bytes)`, `manifest_address(manifest, wasm)`, `canonical_json(obj)`,
`keccak256(data)`; `hive.invoke_local(name, input)` for unit tests.

## How it runs on a node

The module executes in a Python interpreter (RustPython 0.6.0, without the
CPython standard library) compiled to `wasm32-unknown-unknown`. It imports only
the hive-wasm-v1 host functions; there is no WASI. The prebuilt interpreter
ships as `hivekit/runtime/hivekit-py-runtime.wasm`; `hivec build` embeds your
source into a copy of it as a data segment, so no Rust toolchain is needed.

Inside a module:

- `from hivekit import hive` resolves to the built-in runtime module.
- `import json` works (`dumps`/`loads` with the usual arguments).
- Only deterministic built-in modules can be imported (`itertools`,
  `_functools`, `_collections`, `_operator`, …). `time`, `random`, `os`,
  `socket`, threads and third-party packages are not available, by design:
  modules must be deterministic. Pass timestamps and randomness in the input,
  or derive values with `hive.hash()`.
- String hashing uses a fixed seed; the interpreter sees no clock or entropy.
- Event data must be JSON without floats (node rule).

### Gas and size

| | |
|---|---|
| `module.wasm` | ~9.6 MiB (pre-initialized interpreter) + your source (≤ 1 MiB); the limit is 12 MiB |
| Gas per call | ~10–12M for a small handler on execution revision 2 (the interpreter's data segments are copied in at 1 gas per byte on every call); a `hive.call` into another Python module adds that module's ~10–12M |

Interpreter start-up (creating the VM and installing the `hivekit`/`json`
modules) happens once, at build time: `runtime-py/build.sh` runs the runtime's
`__hive_preinit` and snapshots the initialized linear memory into the shipped
module, so each call starts from a ready interpreter instead of paying ~170M
gas for start-up. Use a gas limit of 50,000,000 for Python calls (the CCS
default gas cap, and the `max_gas_limit` to give a Python project): it covers
ordinary calls and Python-to-Python `hive.call`s; heavy handlers cost more, so
pass a higher `gas_limit` if needed. Nodes need the 12 MiB `module.wasm` limit (HBC_SPEC §2);
older nodes, limited to 8 MiB, reject Python modules.

## Tests

```bash
pytest     # vectors, schema, local API, CLI, end-to-end under ndsr (skipped if ndsr is absent)
```

## Rebuilding the runtime

```bash
runtime-py/build.sh     # pinned Rust 1.98.0, wasm-opt 133, Homebrew llvm (clang for wasm32), Node.js
```

The build is byte-reproducible (same output from clean builds in different
directories). Expected result:

```
hivekit/runtime/hivekit-py-runtime.wasm  10072015 bytes
sha256 6523d011b0016d0de8998c67aef3b4e8bc9ee9d6d4171ea1d8715da8cb89ac2e
```

`SNAPSHOT=0 runtime-py/build.sh` builds the plain runtime without the
pre-initialized interpreter (~7.6 MiB, ~170M gas per call), for debugging.

`runtime-py/src/hivekit_rt.py` is the in-interpreter `hivekit` module and
`runtime-py/src/json_rt.py` the `json` module; the guest ABI glue is shared
with the JavaScript runtime (`../hivekit-js/runtime-js/guest`).

## License

Apache-2.0. See [LICENSE](../LICENSE) and [NOTICE](../NOTICE).
