// Package hivekit is the Go guest SDK for Necter modules (NDSR `hive-wasm-v1`).
//
// A module registers its functions with Define, DefineJSON or DefineRaw from an
// init function (or a package-level var initializer). The package exports the
// hive-wasm-v1 guest ABI (`memory`, `__alloc`, `__hive_entry`) and binds the host
// imports (storage, events, cross-module calls, hashing, logging, abort).
//
// Build with TinyGo for the bare `wasm-unknown` target (no WASI), then package
// with hivec:
//
//	hivec build ./examples/math_module
//
// or by hand:
//
//	tinygo build -target=wasm-unknown -no-debug -o module.wasm ./mymodule
//	hivec package module.wasm -name mymodule
//
// Function ids are indexes into the sorted function list, exactly as in the
// manifest; dispatch never depends on registration order.
//
// Example:
//
//	package main
//
//	import hivekit "github.com/Necter-Network/hivekit/hivekit-go"
//
//	func init() {
//		hivekit.Define("addNumbers", func(in map[string]any) map[string]any {
//			a, _ := in["a"].(float64)
//			b, _ := in["b"].(float64)
//			return map[string]any{"total": a + b}
//		})
//	}
//
//	func main() {}
package hivekit

import (
	"encoding/json"
	"errors"
	"fmt"
	"sort"
)

// Version is the SDK version, recorded in the manifest `compiler` field by hivec.
const Version = "1.0.0"

// Handler is a JSON-object function: the input is the parsed JSON object (an
// empty input is passed as an empty map) and the returned map is serialized as
// the output. Numbers arrive as float64.
type Handler func(input map[string]any) map[string]any

// RawHandler receives the raw input bytes and returns the raw output bytes,
// which must be valid UTF-8. A non-nil error fails the call with its message.
type RawHandler func(input []byte) ([]byte, error)

type entry struct {
	name string
	fn   RawHandler
}

var (
	registry = map[string]RawHandler{}
	sorted   []entry // rebuilt lazily; nil when stale
)

func nameOK(s string) bool {
	if len(s) == 0 || len(s) > 64 {
		return false
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case c == '_' || (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z'):
		case c >= '0' && c <= '9' && i > 0:
		default:
			return false
		}
	}
	return true
}

// DefineRaw registers a function that works on raw bytes. Registering the same
// name twice, or an invalid name (`[A-Za-z_][A-Za-z0-9_]{0,63}`), panics.
func DefineRaw(name string, fn RawHandler) {
	if !nameOK(name) {
		panic("hivekit: invalid function name " + name)
	}
	if _, dup := registry[name]; dup {
		panic("hivekit: function registered twice: " + name)
	}
	registry[name] = fn
	sorted = nil
}

// Define registers a JSON-object function (see Handler).
func Define(name string, fn Handler) {
	DefineRaw(name, func(input []byte) ([]byte, error) {
		in := map[string]any{}
		if len(input) > 0 {
			if err := json.Unmarshal(input, &in); err != nil {
				return nil, fmt.Errorf("invalid JSON input: %v", err)
			}
		}
		out := fn(in)
		if out == nil {
			out = map[string]any{}
		}
		return marshal(out)
	})
}

// DefineJSON registers a typed function. The input is decoded into In (an empty
// input decodes as the zero value); the result is encoded as JSON. A non-nil
// error fails the call.
func DefineJSON[In any, Out any](name string, fn func(In) (Out, error)) {
	DefineRaw(name, func(input []byte) ([]byte, error) {
		var in In
		if len(input) > 0 {
			if err := json.Unmarshal(input, &in); err != nil {
				return nil, fmt.Errorf("invalid JSON input: %v", err)
			}
		}
		out, err := fn(in)
		if err != nil {
			return nil, err
		}
		return marshal(out)
	})
}

func marshal(v any) ([]byte, error) {
	b, err := json.Marshal(v)
	if err != nil {
		return nil, fmt.Errorf("cannot encode output: %v", err)
	}
	return b, nil
}

func table() []entry {
	if sorted == nil {
		sorted = make([]entry, 0, len(registry))
		for n, fn := range registry {
			sorted = append(sorted, entry{n, fn})
		}
		sort.Slice(sorted, func(i, j int) bool { return sorted[i].name < sorted[j].name })
	}
	return sorted
}

// Functions returns the registered function names sorted ascending by byte
// value. A function's id (as used by NDSR) is its index in this list.
func Functions() []string {
	t := table()
	out := make([]string, len(t))
	for i, e := range t {
		out[i] = e.name
	}
	return out
}

// FuncID returns the NDSR function id for name, or -1.
func FuncID(name string) int {
	for i, e := range table() {
		if e.name == name {
			return i
		}
	}
	return -1
}

// ErrFunctionNotFound is returned when a function id or name is not registered.
var ErrFunctionNotFound = errors.New("function not found")

// dispatch runs function id funcID; it is what __hive_entry calls.
func dispatch(funcID int, input []byte) ([]byte, error) {
	t := table()
	if funcID < 0 || funcID >= len(t) {
		return nil, ErrFunctionNotFound
	}
	return t[funcID].fn(input)
}

// functionsJSON is the sorted function list as a JSON array; it is exported to
// tooling through `__hive_functions` so hivec can build the manifest from the
// module itself rather than from source text.
func functionsJSON() []byte {
	b, _ := json.Marshal(Functions())
	return b
}
