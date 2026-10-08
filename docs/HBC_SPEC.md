# HBC_SPEC — Hive Bytecode artifacts and the `hive-wasm-v1` guest ABI

Status: **normative, v1**. NDSR (`ndsr` ≥ 1.0.0) implements exactly this document.
The HiveKit SDKs (Rust, Go, JS/TS, Python) and the network's coordinator MUST produce / verify
artifacts against it. Canonical JSON and the receipt hash are defined in §4 and §11 below.
Known-answer vectors: [`test-vectors.json`](test-vectors.json).

The key words MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

---

## 1. Overview

A module is a single WebAssembly binary plus a manifest, packaged as a `.hbc` (ZIP) file and
identified by its content address `manifest_address`. A node executes one exported function per
call: the input is a byte string (UTF-8 text by convention, usually JSON), the output is a UTF-8
string, and the node records a receipt whose hash is identical on every honest node.

```
source ──(SDK compiler)──► module.wasm + manifest.json ──zip──► module.hbc
                                                 │
            manifest_address = keccak256(canonical(manifest − manifest_address) ‖ wasm)
```

## 2. The `.hbc` container

* A ZIP archive (stored or deflate) containing **exactly** two entries:
  * `manifest.json`
  * `module.wasm`
* Any other entry (directories, `module.js`, `module.py`, `module.go`, …) → the artifact is
  **rejected**. Source/WASI artifacts are not executable on NDSR (see §9).
* Duplicate entry names → rejected.
* Size limits (uncompressed, enforced while reading, independent of the sizes declared in the ZIP):

| Item                | Limit   |
|---------------------|---------|
| whole `.hbc` file   | 16 MiB  |
| `manifest.json`     | 64 KiB  |
| `module.wasm`       | 12 MiB  |

* These sizes are **consensus parameters**: a node that rejects an artifact cannot execute it
  while a node that accepts it can, so every node on a network MUST enforce the same values.
* `module.wasm` limit history: the initial v1 text set 8 MiB; it is now **12 MiB**, so that
  interpreter runtimes shipped pre-initialized (the interpreter's start-up state captured in
  data segments at build time — HiveKit's Python runtime is ~10 MB, JavaScript ~3.4 MB) fit
  together with their embedded source. Rationale for the bound: per-call memory is still capped
  by the linear-memory limit (§8, 64 MiB) because data segments must fit in the initial memory;
  compile time is driven by the code section, which pre-initialization does not grow; and a
  12 MiB `module.wasm` plus a 64 KiB manifest still fits a stored (uncompressed) `.hbc` under the
  unchanged 16 MiB file limit.
* Compatibility: the runtime id stays `hive-wasm-v1` (the guest ABI is unchanged) and every
  artifact ≤ 8 MiB keeps its address, gas and receipts. Artifacts with an 8–12 MiB `module.wasm`
  are rejected by older nodes, so a network MUST upgrade all of its nodes before such modules
  are deployed (or keep the dispatcher's upload cap at 8 MiB until it has).
* Packagers SHOULD write deterministic archives (fixed entry order `manifest.json`,
  `module.wasm`; stored; zero timestamps). Archive bytes are **not** part of the address,
  so this is a convenience, not a consensus rule.

## 3. Manifest schema

`manifest.json` is a UTF-8 JSON object. Rules:

* No duplicate keys (anywhere). No floats; integers only, |n| ≤ 2^53−1.
* **Only** the keys below are allowed. Unknown keys (e.g. `created_at`, `timestamp`, `wasm_ready`,
  `nrc1`, `consensus`) → rejected. Timestamps are forbidden: the same build must yield the same address.

| Key                | Type            | Required | Rule |
|--------------------|-----------------|----------|------|
| `name`             | string          | yes      | 1–128 bytes |
| `language`         | string          | yes      | `[a-z0-9_+-]{1,32}`, informational (e.g. `assemblyscript`, `rust`, `go`, `c`, `wat`) |
| `compiler`         | string          | yes      | 1–128 bytes, informational (e.g. `hivec-rs/1.2.0`) |
| `runtime`          | string          | yes      | MUST equal `"hive-wasm-v1"` |
| `functions`        | array of string | yes      | 1–256 entries; each `[A-Za-z_][A-Za-z0-9_]{0,63}`; **sorted ascending by byte value and unique** |
| `version`          | string          | no       | ≤ 1024 bytes |
| `description`      | string          | no       | ≤ 1024 bytes |
| `manifest_address` | string          | no       | if present MUST equal the computed address (§4) |

A loader MUST reject an unsorted or duplicated `functions` list rather than sort it.

## 4. Content address

```
manifest_address = "0x" + hex( keccak256( canonical_json(M) || module.wasm ) )
```

* `M` is the parsed manifest object **with the `manifest_address` key removed** (if present).
* `canonical_json` is NDSR canonical JSON: keys sorted by codepoint, no whitespace,
  UTF-8 without `\u` escaping of non-ASCII, integers only.
  Python: `json.dumps(M, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()`.
* `keccak256` is original Keccak-256 (Ethereum), not SHA3-256.
* Format: `0x` + 64 **lowercase** hex. APIs MAY accept upper case or a `hive:` prefix and MUST
  normalize before comparing; URLs and storage keys always use the canonical form.
* Verification is **mandatory** everywhere an artifact enters a trust boundary
  (upload, node fetch, cache read). A node fetching address `A` MUST reject bytes whose
  computed address ≠ `A`.

Example (from `test-vectors.json`):
`{"compiler":"ndsr-test-vectors/1","functions":["echo"],"language":"wat","name":"vector_echo","runtime":"hive-wasm-v1","version":"1.0.0"}`.

## 5. Function ids

`func_id` of a function is its **zero-based index in `manifest.functions`** (which is sorted).
SDK dispatch glue MUST map ids using the same sorted order — never registration order.

## 6. Guest ABI (`hive-wasm-v1`)

All integers are wasm `i32`/`i64`. Pointers and lengths are interpreted as **unsigned** 32-bit.

### 6.1 Required exports

| Export          | Kind / signature                          | Semantics |
|-----------------|-------------------------------------------|-----------|
| `memory`        | memory (32-bit, non-shared)               | the module's linear memory |
| `__alloc`       | `(len: i32) -> i32`                       | returns a pointer to `len` writable bytes **in `memory`** (a real linear-memory address, not an offset into a private heap). Called only with `len > 0`. |
| `__hive_entry`  | `(func_id: i32, ptr: i32, len: i32) -> i64` | run function `func_id` on input bytes `memory[ptr..ptr+len]`; return packed output (§6.2) |

No other exports are required; extra exports are ignored. The legacy
`__hive_entry_str(i32,i32,i32)->i32` + `__str_len` (NUL-terminated output) ABI is **not** supported
and is rejected with an explicit error.

### 6.2 Packed (ptr, len) return convention

Every byte string returned across the boundary (entry output, host results) is an `i64`:

```
packed = (ptr << 32) | len        ptr, len: u32
```

* `packed >= 0` → bytes `memory[ptr .. ptr+len]`. `0` means the empty string.
* `packed < 0` from `__hive_entry` → the call **fails** with "guest returned error code N".
* Because linear memory is capped at 1 GiB (§8), `ptr < 2^31`, so a valid packed value is never negative.

Why packed i64 rather than pointer + `__str_len`: outputs are binary-safe (may contain NUL),
the host needs no second export call, there is no scan of guest memory, and the length is
bounds-checked before any copy.

### 6.3 Call sequence (host side)

1. Validate the module (imports §6.4, exports §6.1) and apply the stack-limit instrumentation
   (§9.1). Failure → failed receipt, `gas_used = 0`.
2. Instantiate with fuel = `gas_limit`, then run the start function, if any (metered; after
   instrumentation it is called through the `__ndsr_start` export, at the same point).
3. If the input is non-empty: `p = __alloc(len)`; check `p + len ≤ memory size`; copy input to `p`.
   Empty input is passed as `(0, 0)` without calling `__alloc`.
4. `r = __hive_entry(func_id, p, len)`.
5. `r < 0` → failure. Else unpack; check range in bounds and `len ≤ max_output_bytes`;
   output MUST be valid UTF-8 (else failure).

### 6.4 Host imports

A module MAY import only these functions (exact signatures). Any other import — including
any `wasi_*` module, imported memories/tables/globals, or AssemblyScript's `env.seed` /
`env.trace` — makes the module invalid (failed receipt with a clear error).

| Import          | Signature                                        | Semantics |
|-----------------|--------------------------------------------------|-----------|
| `hive.call`     | `(addr_ptr, addr_len, fn_ptr, fn_len, in_ptr, in_len: i32) -> i64` | synchronous cross-module call (§6.5) |
| `hive.emit`     | `(name_ptr, name_len, data_ptr, data_len: i32)`  | append event `{name, data}`; name `[A-Za-z0-9_.:-]{1,64}`; data MUST be JSON acceptable to canonical JSON (no floats, |int| ≤ 2^53−1); otherwise trap |
| `hive.abort`    | `(msg_ptr, msg_len: i32)`                        | fail the call with a message (UTF-8, ≤ 1 KiB used) |
| `storage.get`   | `(key_ptr, key_len: i32) -> i64`                 | value for key in this module's state, packed; `0` if absent |
| `storage.set`   | `(key_ptr, key_len, val_ptr, val_len: i32)`      | set key; an empty value deletes the key; empty key → trap |
| `storage.del`   | `(key_ptr, key_len: i32)`                        | delete key |
| `console.log`   | `(ptr, len: i32)`                                | debug log on the node; no consensus effect |
| `crypto.hash`   | `(ptr, len: i32) -> i64`                         | keccak256 of the bytes as the 66-byte ASCII string `0x…` (packed) |
| `env.abort`     | `(msg, file, line, col: i32)`                    | AssemblyScript runtime abort → failure |

Host functions that return data allocate it in guest memory by calling the guest's `__alloc`
(the guest's own fuel is spent), validate the returned pointer, copy, and return the packed value.
Storage is **namespaced by the calling module's address**: a module can only read/write its own keys.

Any out-of-bounds pointer, over-limit length, or invalid UTF-8 where text is required is a
**trap** of the current call (never a host crash, never an allocation of the requested size).

### 6.5 `hive.call`

* `addr` MUST be a canonical module address (else returns `-6`).
* Depth: the top-level call is depth 0; a call that would exceed `max_call_depth` returns `-5`.
* The callee is resolved (in-memory registry → local cache → network fetch with address verification).
  Not found → `-1`. Function not in the callee's manifest → `-2`. If the node cannot determine
  existence (network failure), the whole execution is aborted as a **node error**: no receipt is
  produced or signed.
* The callee runs in a fresh instance with **all** of the caller's remaining gas; whatever it
  consumes is charged to the caller on every path (success, trap, error).
* Callee success → its state writes and events are merged into the caller's journal/event list;
  returns its output (packed, ≥ 0).
* Callee trap/abort/negative return → its writes and events are discarded; returns `-3`.
* Callee out of gas → the caller has no gas left: the **whole** call fails out of gas
  (code `-4` is reserved and never observed).

| Code | Meaning |
|------|---------|
| `-1` | module not found |
| `-2` | function not found |
| `-3` | callee failed (trap / abort / error code) |
| `-4` | reserved (out of gas propagates) |
| `-5` | call depth exceeded |
| `-6` | malformed address |

## 7. Gas

* `gas_limit` is in **fuel units** of wasmtime 38.0.3 with `consume_fuel` (≈ 1 per wasm operator;
  structural operators such as `nop`, `block`, `end` are free). Wasm fuel accounting is defined by
  that implementation; nodes MUST run the pinned version. Upgrading wasmtime is a network
  protocol change. `test-vectors.json → execution` pins one concrete value.
* `1 ≤ gas_limit ≤ max_gas_limit` (node config, default 10^10, hard cap 2^53−1), else the request is rejected.
* Host functions charge **fixed** costs before doing work (insufficient gas → out of gas):

| Host function | Cost |
|---------------|------|
| `console.log` | 100 + 1/byte |
| `hive.emit`   | 500 + 2/byte (name + data) |
| `crypto.hash` | 200 + 1/byte |
| `storage.get` | 1 000 + 1/byte of key + 1/byte of value returned |
| `storage.set` | 5 000 + 10/byte of key + value |
| `storage.del` | 2 000 + 1/byte of key |
| `hive.call`   | 10 000 + 1/byte (address + function + input) + callee gas + 1/byte of output |

* `gas_used = gas_limit − remaining` on success or trap; **`gas_used = gas_limit` on out of gas**;
  `0` for modules rejected by validation.
* Gas is metered on the **instrumented** module (§9.1): the stack-limit prologue costs 8 and the
  epilogue 4, i.e. **12 gas per executed wasm function call** (including the host's calls of
  `__alloc`, `__hive_entry` and the start function).

### 7.1 Where out of gas is detected (fuel-check granularity)

Gas is charged per operator, but wasmtime only *checks* it at fixed points. This is part of the
pinned semantics (identical on every engine, since every engine runs the same fuel-instrumented
IR), stated exactly:

* **Accounting.** Operator costs are added to the consumed total at basic-block boundaries:
  before `call`, `call_indirect`, `return_call*`, `return`, `unreachable`, `br`, `br_if`,
  `br_table`, `if`, `else`, `end` and `loop`. A trap raised by any other operator (division by
  zero, out-of-bounds access, …) does not count the operators of its block that precede it since
  the last boundary.
* **Checks.** Execution stops with out of gas at the first check point at which
  `consumed ≥ gas_limit`. The check points are: the entry of every wasm function (before its
  first operator), every arrival at a `loop` header (each iteration), and every host function,
  which needs `remaining ≥ cost` (§7 table) before doing any work.
* **Overdraft.** Between two check points execution is not interrupted, and `remaining` reported
  afterwards floors at 0. So a call may finish — succeed, or trap — after consuming more than
  `gas_limit`, and then reports `gas_used = gas_limit`. In particular a call that needs `G` gas
  may still succeed with `gas_limit = G − 1` (reporting `gas_used = G − 1`). The free overdraft
  is bounded by one loop-free, call-free stretch of a single function body.
* This is accepted as is: it is deterministic across engines and builds, bounded, and cheaper
  than per-operator checks. Callers that need "succeeds iff `gas_limit ≥ G`" must not assume it;
  the guarantee is only that the same `(module, function, input, gas_limit, state)` yields the
  same receipt on every node.

## 8. Limits (consensus parameters, defaults)

All nodes on a network MUST use the same values. The engine's own (native or Pulley) stack size
is *not* a consensus parameter: it only has to leave headroom above the logical limit (§9.1).

| Limit | Default |
|-------|---------|
| linear memory per instance | 64 MiB (≤ 1 GiB) — `memory.grow` beyond returns −1 |
| table elements | 100 000 |
| instances / memories / tables per store | 1 / 1 / 4 |
| input bytes | 1 MiB |
| output bytes | 1 MiB |
| events per call | 64 |
| event name / data bytes | 64 / 16 KiB |
| storage key / value bytes | 256 / 64 KiB |
| state bytes written per top-level call (keys + values) | 1 MiB |
| `console.log` bytes | 4 KiB |
| `hive.call` depth | 8 |
| logical stack (§9.1) | 65 536 units per instance |

## 9. Determinism rules

* Enabled: MVP, multi-value, bulk memory, reference types, sign-extension, saturating
  float→int, SIMD (with NaN canonicalization), tail calls.
* Disabled: threads / shared memory, relaxed SIMD, memory64, multi-memory, GC, typed function
  references, exceptions, stack switching, wide arithmetic, custom page sizes.
* All float NaN results are canonicalized (`f64` 0/0 → `0x7ff8000000000000`).
* No clocks, randomness, filesystem, network or environment: there are **no WASI imports**.
  JS (Javy/QuickJS stdio) and Python (py2wasm) artifacts built for WASI are rejected; such
  languages must target `hive-wasm-v1` (e.g. a QuickJS build that exports `__alloc`/`__hive_entry`).
* State reads see committed state plus the current call's own writes; top-level executions on a
  node are serializable, so each module's state transitions are totally ordered by the node's commit
  order. (Executions that touch disjoint module state may run concurrently; an execution holds every
  module whose state it reads or writes until it commits. This is invisible to modules.)
  Ordering of calls across nodes is the network dispatcher's responsibility.
* Execution engine: guest code is compiled by the pinned Cranelift and runs either as native code
  (x86_64, aarch64, riscv64, s390x) or on wasmtime's Pulley interpreter (`pulley32` on 32-bit
  CPUs such as armv7; `pulley64` when forced on a 64-bit host). Fuel instrumentation and NaN
  canonicalization are applied to the same IR before the backend runs, and Pulley implements
  the wasm trap semantics itself, so output, events, `gas_used`, success and receipt hash are
  identical on both. The engine is not a consensus parameter.
* Stack exhaustion is deterministic: see §9.1.

### 9.1 Deterministic stack limit

How deep native code or Pulley can recurse depends on frame sizes, which differ between CPU
architectures, engines and even builds of the node (before this rule, one runaway-recursion module
trapped after 98 355 gas on native aarch64 release, 98 220 native debug, 98 409 `pulley64` and
118 065 `pulley32` — four different receipts). Every node therefore rewrites each module before
compiling it, so that the module tracks its own logical stack height and traps at a fixed limit
that is always reached long before any engine stack is. The rewrite is consensus-critical and
MUST be reproduced exactly (reference: the NDSR node's stack-limit pass).

**Analysis.** The module is validated with wasmparser 0.239.0 (pinned; same as wasmtime 38.0.3).
For every *defined* function `f` (imports cost nothing):

```
cost(f) = FRAME_COST + L(f) + H(f)
FRAME_COST = 4
L(f) = number of parameters + number of declared locals          (each value = 1, any type)
H(f) = maximum, over every operator of the body, of the validator's operand stack height
       (`FuncValidator::operand_stack_height`, whole function, all control frames) right after
       that operator is validated
```

`cost(f)` saturates at `STACK_LIMIT + 1` (such a function traps whenever it is called).

**Rewrite.** With `STACK_LIMIT = 65 536` and `G` = the index of a new global:

1. Append one global `(global $sh (mut i32) (i32.const 65536))`; `G` = number of imported globals
   + number of defined globals (no existing index moves). If the module has no global section,
   one is created in canonical section order.
2. Export it as `"__ndsr_stack"`. If the module has a start section, remove the section and export
   its function as `"__ndsr_start"` (the host calls it right after instantiation). Export names
   starting with `__ndsr_` in the original module are reserved: such a module is rejected
   (failed receipt, `gas_used = 0`). If the module has no export section, one is created.
3. For each function whose results `R` have 2 or more values, a block type is needed: append
   one function type `[] -> R` per distinct `R`, in order of first use by defined functions, at
   the end of the type section (created if absent). No existing type index moves.
4. Each defined function body keeps its locals declaration and becomes

```
global.get G  i32.const cost(f)  i32.sub  global.set G        ;; charge on entry
global.get G  i32.const 0  i32.lt_s  if  unreachable  end     ;; budget negative → trap
block BT                                                     ;; BT = [] -> results of f
  <original operators, minus the final `end`; every `return`, `return_call`,
   `return_call_indirect` and `return_call_ref` is immediately preceded by the refund>
end
global.get G  i32.const cost(f)  i32.add  global.set G        ;; refund
end
```

   `BT` is the empty block type, a single value type, or the type index from step 3. Because the
   wrapper block becomes the target of every branch that left the function's outermost label, all
   exits pass the refund; tail calls refund before transferring, so tail recursion does not grow
   the logical stack. All other bytes (other sections, operator encodings, custom and name
   sections) are copied unchanged.

**Semantics.** `$sh` is per instance, so each `hive.call` callee starts with a full budget. The
active frames' costs therefore always sum to at most 65 536 per instance; the call that would push
the sum above it traps. The node reports this trap as **`stack limit exceeded`** (it is the only
trap after which `__ndsr_stack` is negative), and the injected operators are metered like any
other (§7), so `gas_used` up to the trap is identical everywhere. Example: an empty function that
calls itself without bound costs 4 + 0 + 0 units per frame and is stopped after the same amount of
gas on native x86_64/aarch64 code, `pulley64` and `pulley32` (armv7).

**Engine headroom (not consensus).** Measured frames use at most ~21 bytes per unit (v128
values live across a call, Cranelift and Pulley) and 32–48 bytes for an empty frame. NDSR
requires an engine stack of at least 64 bytes × 65 536 + 512 KiB (4.5 MiB) per instance and
defaults to 8 MiB; the worst fixture in the test suite needs 1.3 MiB. Should an engine stack
ever overflow first anyway, the node treats it as a node error (no receipt is signed) rather than
reporting a trap whose position is not deterministic.

**Version note.** Introduced pre-launch in `hive-wasm-v1` (spec revision 2026-10). It changes
`gas_used` — and hence `receipt_hash` — of every execution: +12 per executed function call (e.g.
the `test-vectors.json → execution` call: 557 → 581, two calls: `__alloc` and `__hive_entry`;
SDK-built modules: +11–14 % for small Rust/AssemblyScript calls, +25–30 % for call-heavy Go and
QuickJS code),
and it replaces the previous "512 KiB wasm stack" limit. Module addresses, the ABI and
artifacts are unchanged. All nodes of a network MUST run the same revision.

## 10. State and atomicity

* State is a per-module key/value map persisted by the node (SQLite).
* Writes are buffered in a journal of call frames. A `hive.call` pushes a frame; callee success
  merges it into the caller's frame, callee failure discards it.
* Only when the **top-level** call succeeds is the journal committed, atomically. Trap,
  abort, out of gas or validation failure → nothing is written and no events are recorded.

## 11. Receipt

`receipt_hash = keccak256(canonical_json({v, module_address, function,
input_hash, output_hash, events_hash, gas_used, success}))` with
`input_hash = keccak256(input bytes)`, `output_hash = keccak256(output bytes)` (empty on failure),
`events_hash = keccak256(canonical_json([{"name","data"}, …]))` in emission order (`[]` on failure).
The `error` string is informational and not hashed.

## 12. Conformance checklist for SDKs

1. Emit `runtime: "hive-wasm-v1"`, a `language`, a `compiler`, **sorted unique** `functions`, no timestamps or extra keys.
2. Compute the address over canonical JSON minus `manifest_address`, using Keccak-256; Python must use
   `ensure_ascii=False` and never fall back to `hashlib.sha3_256`.
3. Export `memory`, `__alloc` returning real linear-memory pointers, and
   `__hive_entry(i32,i32,i32)->i64` returning packed (ptr,len); dispatch by sorted index.
4. Import only §6.4 functions with exact signatures. No WASI.
5. Reproduce every value in `test-vectors.json`.
