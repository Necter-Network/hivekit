# hivekit-go

Go SDK for Necter modules. Modules are compiled with TinyGo to a bare WebAssembly
module that implements the NDSR `hive-wasm-v1` guest ABI, then packaged as a `.hbc`
artifact (`manifest.json` + `module.wasm`) whose content address is
`keccak256(canonical_json(manifest − manifest_address) ‖ module.wasm)`.

Normative references: [`docs/HBC_SPEC.md`](../docs/HBC_SPEC.md) (artifact format, ABI, host imports,
limits, canonical JSON, receipts).

| Package          | What it is |
|------------------|------------|
| `hivekit`        | the guest SDK you import in a module |
| `hivekit/hbc`    | manifests, canonical JSON, Keccak-256 address, `.hbc` read/write, ABI checker |
| `cmd/hivec`      | build / package / inspect / functions / run |

## Requirements

* Go ≥ 1.21
* [TinyGo](https://tinygo.org) (tested with 0.42.0). Homebrew has no `tinygo` formula
  in core; use `brew tap tinygo-org/tools && brew install tinygo` or unpack a release
  tarball and point `$TINYGO` at `bin/tinygo`.
* For `hivec run` and the end-to-end tests: the reference runtime `ndsr`
  (`$NDSR_BIN`, `ndsr` on `PATH`, or `tools/ndsr` in any parent directory).

Install `ndsr` and a prebuilt `hivec` (the installer also checks Go and TinyGo):

```sh
curl -fsSL https://necter.network/install.sh | sh -s -- go
```

Or build the CLI yourself:

```sh
go install github.com/Necter-Network/hivekit/hivekit-go/cmd/hivec@v1.0.0
```

Add the SDK to your module:

```sh
go get github.com/Necter-Network/hivekit/hivekit-go@v1.0.0
```

## Writing a module

```go
package main

import (
	"errors"

	hivekit "github.com/Necter-Network/hivekit/hivekit-go"
)

type pair struct {
	A float64 `json:"a"`
	B float64 `json:"b"`
}

func init() {
	// Typed: input decoded from JSON, output encoded as JSON, error fails the call.
	hivekit.DefineJSON("divide", func(p pair) (map[string]float64, error) {
		if p.B == 0 {
			return nil, errors.New("division by zero")
		}
		return map[string]float64{"result": p.A / p.B}, nil
	})

	// Untyped JSON object in, JSON object out.
	hivekit.Define("greet", func(in map[string]any) map[string]any {
		name, _ := in["name"].(string)
		return map[string]any{"message": "Hello, " + name}
	})

	// Raw bytes in, raw UTF-8 bytes out.
	hivekit.DefineRaw("echo", func(in []byte) ([]byte, error) { return in, nil })
}

func main() {} // required by Go, never called by NDSR
```

Register functions from `init()` (or package-level initializers). Names must match
`[A-Za-z_][A-Za-z0-9_]{0,63}`; registering a name twice panics.

### Host API

| Go | Host import | Notes |
|----|-------------|-------|
| `StorageGet(key) ([]byte, bool)`, `StorageGetString`, `StorageGetJSON` | `storage.get` | module-private state; absent → `false` |
| `StorageSet(key, val)`, `StorageSetString`, `StorageSetJSON` | `storage.set` | empty value deletes; key 1–256 B, value ≤ 64 KiB |
| `StorageDel(key)` | `storage.del` | |
| `Emit(name, data)`, `EmitRaw(name, json)` | `hive.emit` | name `[A-Za-z0-9_.:-]{1,64}`; data must be canonical-JSON compatible |
| `Call(addr, fn, input) ([]byte, error)`, `CallJSON(addr, fn, in, &out)` | `hive.call` | negative host codes come back as `*CallError` (`CallModuleNotFound` −1, `CallFunctionNotFound` −2, `CallFailed` −3, `CallDepthExceeded` −5, `CallBadAddress` −6) |
| `Hash(data) string` | `crypto.hash` | Keccak-256 as `0x…` |
| `Log(msg)`, `Logf(...)` | `console.log` | node debug log, no consensus effect |
| `Abort(msg)` | `hive.abort` | fails the call; nothing it wrote or emitted is kept |

State writes and events are committed only if the top-level call succeeds. A failed
`hive.call` callee's writes and events are discarded and the caller sees `CallFailed`.

**No floats in events.** NDSR rejects event data with non-integral numbers or
integers beyond ±(2⁵³−1). Use integer minor units or strings. (Function *outputs* are
just UTF-8 text and may contain any JSON.) `encoding/json` encodes a whole-valued
`float64` such as `42.0` as `42`, which is accepted.

## Build, inspect, run

```sh
hivec build ./examples/math_module                 # → dist/math_module.hbc
hivec inspect dist/math_module.hbc                 # verify address, manifest, imports
hivec functions dist/math_module.hbc               # ids = index in the sorted list
hivec run dist/math_module.hbc multiply '{"a":7,"b":6}'
```

`hivec build` runs `tinygo build -target=wasm-unknown -no-debug`, checks that the
module only imports spec host functions (no WASI) and exports `memory`, `__alloc`,
`__hive_entry`, reads the registered function list from the module itself, and writes
the manifest:

```json
{"compiler":"hivec-go/1.0.0 tinygo/0.42.0","functions":["addNumbers","divide","greetUser","multiply"],
 "language":"go","manifest_address":"0x…","name":"math_module","runtime":"hive-wasm-v1"}
```

Flags: `-o DIR`, `-name`, `-version`, `-description`, `-tinygo PATH`, `-gc`, `-keep-wasm`.
`hivec package module.wasm -name N` packages a wasm you built yourself.

`hivec run` executes through `ndsr run` and prints its signed receipt. Useful flags:

```sh
# state persists between runs
hivec run -data-dir ./state dist/price_oracle.hbc submit '{"pair":"ETH/USD","source":"a","price":320012}'
hivec run -data-dir ./state dist/price_oracle.hbc price  '{"pair":"ETH/USD"}'

# make other modules callable through hive.call (staged in <data-dir>/modules/<address>.hbc)
hivec run -module dist/math_module.hbc dist/ledger.hbc record '{"math":"0x<math address>","a":6,"b":7}'
```

Without `ndsr` (or with `-local`) `hivec run` falls back to a built-in wazero runner
that implements the full host interface but has **no gas metering and produces no
receipt**; use it for quick iteration only.

## Testing modules natively

In a normal `go test` build the host imports are served by an in-memory mock
(`hivekit.Mock`): storage map, recorded events and logs, and an `OnCall` hook for
`hive.call`. `InvokeLocal` runs a function the way NDSR would, rolling back storage and
events when it fails:

```go
func TestSubmit(t *testing.T) {
	hivekit.ResetMock()
	out, err := hivekit.InvokeLocalJSON("submit", `{"pair":"ETH/USD","source":"a","price":5}`)
	// inspect out, err, hivekit.Mock.Storage, hivekit.Mock.Events
}
```

## How the guest ABI is implemented

* `__alloc(len) -> ptr` returns a pointer into linear memory (a Go-allocated buffer kept
  reachable until the guest consumes it).
* `__hive_entry(func_id, ptr, len) -> i64` returns `(ptr << 32) | len`; `func_id` indexes
  the **sorted** function list, the same list as `manifest.functions`. Handler errors
  call `hive.abort` with the message.
* NDSR never calls `_start` or `_initialize`. TinyGo's `wasm-unknown` target is a
  reactor (`_initialize` initializes the heap and runs package initializers), so the
  SDK calls it lazily on the first `__alloc`/`__hive_entry`. This is what makes
  `init()` registration work under NDSR. It costs roughly 35–40k gas per call.
* Typical gas: 70–90k for a small JSON function, ~400k for a call that does a
  `hive.call` plus storage and events; the `ndsr` default limit is 1,000,000.
* Panics (index out of range, nil map write, …) trap the call; recover is not
  available on this target, so return errors instead.
* The module also exports `__hive_functions` (ignored by NDSR) so tooling can read the
  function list from the compiled module.

## The `hbc` package

```go
art, err := hbc.Build(hbc.Manifest{Name: "m", Language: "go", Compiler: "x/1",
	Runtime: hbc.Runtime, Functions: []string{"a", "b"}}, wasm)   // functions must be sorted
data, err := art.Bytes()          // deterministic stored ZIP, fixed entry order, zero timestamps
art, err = hbc.Load(data)         // rejects extra entries, unknown keys, bad address, oversize
err = hbc.CheckABI(wasm)          // imports/exports vs HBC_SPEC §6
b, err := hbc.CanonicalJSON(v)    // HBC_SPEC canonical JSON
```

`CanonicalJSON` does not use `encoding/json` for output: even with
`SetEscapeHTML(false)` the standard encoder escapes U+2028/U+2029 and writes floats, both
of which would change the hash. The tests reproduce the `canonical_json`, `keccak256`
and `manifest_address` vectors from `docs/test-vectors.json`.

## Changes from the pre-v1 SDK

* `__hive_entry_str` / `__str_len` and offset-based `__alloc` are gone; WASI builds are
  rejected by NDSR and by `hivec`.
* `hivekit.Consensus`, `NRC1Config` and the `consensus` / `nrc1` / `created_at` manifest
  keys are removed: v1 manifests reject them and there is no other place to carry them.
* `CompileFile` (which could package Go *source* as `module.go`) is removed; use
  `hivec build`, or `hbc.Build` with a compiled module.
* `InvokeLocal` now takes and returns bytes; `RegisteredFunctions` is `Functions`
  (sorted).

## Tests

```sh
go vet ./... && gofmt -l . && go test ./...
```

`cmd/hivec/integration_test.go` builds the three examples with TinyGo and runs them
under `ndsr` (inspect, outputs, errors, out-of-gas, storage persistence, events,
`hive.call` and its error codes, failure atomicity, reproducible addresses). It skips
only when `tinygo` or `ndsr` cannot be found.

## License

Apache-2.0. See [LICENSE](../LICENSE) and [NOTICE](../NOTICE).
