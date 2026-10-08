# @necter/hivekit (JavaScript / TypeScript)

Build modules for the Necter network in TypeScript or JavaScript. `hivec build`
produces a `.hbc` artifact that conforms to `hive-wasm-v1`
([`docs/HBC_SPEC.md`](../docs/HBC_SPEC.md)): a single `module.wasm` with no
WASI imports, a manifest with only the allowed keys, sorted functions, and a
Keccak-256 content address over canonical JSON.

## Install

```bash
curl -fsSL https://necter.network/install.sh | sh -s -- typescript   # ndsr, checks Node >= 18
npm install --save-dev https://github.com/Necter-Network/hivekit/releases/download/v1.0.0/necter-hivekit-1.0.0.tgz
```

The package is distributed from GitHub releases, not the npm registry. The unrelated npm packages
named `hivekit` and `hivejs` are not part of Necter. Module sources still write
`import { hive, storage } from 'hivekit'` (JavaScript: `require('hivekit')`): that import is provided
by the compiler, not resolved from `node_modules`.

```bash
npx hivec build examples/counter.ts          # -> dist/counter.hbc
npx hivec run examples/counter.ts increment 5
npx hivec inspect dist/counter.hbc
```

## Two targets

| | `as` (AssemblyScript) | `js` (embedded JavaScript engine) |
|---|---|---|
| Default for | `.ts` | `.js`, `.mjs`, `.cjs` (use `--target js` for `.ts`) |
| Language | strict TypeScript subset | full JavaScript and TypeScript |
| `module.wasm` | a few KB | ~3.4 MB (engine + your script) |
| Gas per call (examples) | ~12k | ~2.3–3.2M (engine start-up is pre-initialized) |
| Handler signature | `(input: string): string` | `(input, ctx) => any` (JSON in/out) |
| Manifest `language` / `compiler` | `assemblyscript` / `hivekit-js/1.0.0+assemblyscript@0.28.20` | `javascript` or `typescript` / `hivekit-js/1.0.0+boa@0.22.0` |

### AssemblyScript target (`.ts`)

Same pipeline, prelude, glue and pinned compiler (AssemblyScript 0.28.20,
`--runtime stub -O3`) as `ndsr compile`; for the same source the produced
`module.wasm` is byte-identical (this is tested).

```ts
import { hive, storage } from 'hivekit'   // for editors; stripped at build time

function increment(input: string): string {
  const next = (storage.get("n").length == 0 ? 0 : I64.parseInt(storage.get("n"))) + 1
  storage.set("n", next.toString())
  hive.emit("incremented", "{\"n\":" + next.toString() + "}")
  return next.toString()
}
hive.define("increment", increment)
```

API (provided by the prelude): `hive.define(name, fn)`, `hive.call(address, fn, input)`,
`hive.tryCall(...)` (null on failure), `hive.tryCallRaw(...)` (error code),
`hive.emit(name, dataJson)`, `hive.log(msg)`, `hive.hash(data)` (keccak256 hex),
`hive.fail(msg)`, `storage.get/set/del`.

Constructs AssemblyScript cannot compile are rejected before compiling, with
`file:line` and a pointer to the `js` target: `async`/`await`/`Promise`, `any`,
`unknown`, `undefined`, `try`/`catch`, `JSON`, imports other than `hivekit`,
`require`, `Date.now`/`Math.random`/`new Date()`, `console`, `eval`,
`delete`, generators, `Symbol`. Compiler errors from `asc` are mapped back to
your file's line numbers. Closures that capture locals are not supported by
AssemblyScript (the error carries a hint).

### JavaScript engine target (`.js`, or `.ts --target js`)

Your source (TypeScript is transpiled with the TypeScript compiler) runs inside
a JavaScript engine (Boa 0.22) compiled to `wasm32-unknown-unknown`. The
prebuilt engine ships in `runtime/hivekit-js-runtime.wasm`; your script is
embedded into a copy of it as a data segment, so building needs no Rust.

```js
const { hive, storage, db } = require('hivekit')   // or: import { hive } from 'hivekit'

hive.define('increment', (input) => {
  const count = (db.get('count') || 0) + (input.by ?? 1)
  db.set('count', count)
  hive.emit('incremented', { count })
  return { count }
})

hive.define('relay', (input) => hive.call(input.address, input.function, input.input))

hive.define('note', (ctx) => {           // first parameter named ctx -> context object
  ctx.storage.set('note', ctx.input.text)
  return { hash: hive.hash(ctx.input.text) }
})
```

Conventions (identical in the Python SDK):

- **Input**: the call input parsed as JSON; the raw string if it is not JSON; `{}` if empty.
- **Output**: a returned string is the output as-is; `undefined` → `""`; anything else → `JSON.stringify`.
- **Errors**: a thrown error or `hive.fail(msg)` fails the call (state changes and events are discarded) with the message in the receipt.
- **Async**: handlers may be `async`; only `hive.*` work can be awaited (there is no I/O).
- `hive.call` returns the callee output decoded like input and throws `HiveCallError` (with `.code`, HBC_SPEC §6.5) on failure; `hive.callRaw` returns the raw string; `hive.tryCall` returns `null`.
- `storage` holds raw strings; `db` stores JSON values in the same state.
- `require('hivekit')` is the only module available; bundle other dependencies into the source.

**Determinism.** The engine has no clock, randomness, filesystem or network.
`Math.random()`, `Date.now()`, `Date()` and argument-less `new Date()` throw;
`new Date(timestamp)` works. Event data must be JSON without floats
(node rule). The engine's internal clock is fixed at the Unix epoch.

**Gas.** Engine start-up and the prelude are pre-initialized at build time
(`runtime-js/snapshot.cjs`), so a small call costs ~2–3M gas, within the CCS
default cap (50M). Heavy scripts cost more; pass a higher `gas_limit` if needed.

## Running and testing

`hivec run <file|.hbc> <fn> [input]` builds the module and executes it with the
NDSR binary when one is found (`$NDSR_BIN`, `ndsr` on `PATH`, or a `tools/ndsr`
in a parent directory), printing the signed receipt summary. `--data-dir`
persists state between runs; `hive.call` targets are resolved from
`<data-dir>/modules/<address>.hbc`. Without NDSR (or with `--local`) an
in-process WebAssembly host runs the module (no gas, no receipts).

In Node, `hive.invoke(name, input)` runs a handler against an in-memory store
with the same input/output conventions, for unit tests.

```bash
npm test          # vitest: test vectors, schema, embedding, end-to-end under ndsr
npm run typecheck
npm run lint
```

End-to-end tests use NDSR when it is found and are skipped otherwise.

## Library API

```ts
import { compile, compileFile, readHbc, manifestAddress, canonicalJson, keccak256 } from '@necter/hivekit'

const r = compile(source, 'counter.ts')           // { hbc, wasm, manifest, manifestAddress, functions, target }
readHbc(bytes)                                    // validates container + manifest, verifies the address
```

`HiveClient` is re-exported from [`@necter/hivejs`](../hivejs) for calling deployed modules.

## Rebuilding the JavaScript runtime

```bash
npm run build:runtime   # runtime-js/build.sh: pinned Rust 1.98.0, wasm-opt 133, Node
```

The build is reproducible: a clean build, from any checkout path (source and
cargo registry paths are remapped), yields the committed
`runtime/hivekit-js-runtime.wasm` byte for byte —
sha256 `db6c05e00bfcd0dfd5fc9722d3d7e8a126bd7e8ce8c321396954b271ce23bf01`. `runtime-js/guest` holds the
hive-wasm-v1 guest glue shared with the Python runtime; `runtime-js/src/prelude.js`
is the in-engine `hivekit` API.

## License

Apache-2.0. See [LICENSE](../LICENSE) and [NOTICE](../NOTICE).
