package hivekit

import (
	"encoding/json"
	"fmt"
)

// The functions in this file are the ergonomic layer over the hive-wasm-v1 host
// imports (HBC_SPEC §6.4). Under TinyGo (`tinygo.wasm`) they call the real
// imports; in a native build they use the in-memory MockHost so modules can be
// unit-tested with `go test`.

// ── storage ──────────────────────────────────────────────────────────────────

// StorageGet returns the value stored under key in this module's state. The
// boolean is false when the key is absent (an empty value is never stored:
// setting an empty value deletes the key).
func StorageGet(key string) ([]byte, bool) {
	v := hostStorageGet([]byte(key))
	return v, len(v) > 0
}

// StorageGetString is StorageGet returning a string ("" when absent).
func StorageGetString(key string) string {
	v, _ := StorageGet(key)
	return string(v)
}

// StorageSet stores value under key. An empty value deletes the key; an empty
// key traps. Keys are limited to 256 bytes and values to 64 KiB.
func StorageSet(key string, value []byte) {
	hostStorageSet([]byte(key), value)
}

// StorageSetString is StorageSet for string values.
func StorageSetString(key, value string) {
	hostStorageSet([]byte(key), []byte(value))
}

// StorageDel deletes key.
func StorageDel(key string) {
	hostStorageDel([]byte(key))
}

// StorageGetJSON decodes the JSON value stored under key into v. It returns
// false (and leaves v untouched) when the key is absent.
func StorageGetJSON(key string, v any) (bool, error) {
	b, ok := StorageGet(key)
	if !ok {
		return false, nil
	}
	return true, json.Unmarshal(b, v)
}

// StorageSetJSON stores v encoded as JSON under key.
func StorageSetJSON(key string, v any) error {
	b, err := json.Marshal(v)
	if err != nil {
		return err
	}
	StorageSet(key, b)
	return nil
}

// ── events ───────────────────────────────────────────────────────────────────

// Emit appends the event {name, data} to this call's receipt. data is encoded
// with encoding/json. The node rejects (traps on) names outside
// `[A-Za-z0-9_.:-]{1,64}` and data that is not canonical-JSON compatible: no
// non-integral numbers and integers within ±(2^53−1). Use strings for
// fractional or large amounts.
func Emit(name string, data any) error {
	b, err := json.Marshal(data)
	if err != nil {
		return err
	}
	EmitRaw(name, b)
	return nil
}

// EmitRaw appends an event whose data is the given JSON text.
func EmitRaw(name string, dataJSON []byte) {
	hostEmit([]byte(name), dataJSON)
}

// ── cross-module calls ───────────────────────────────────────────────────────

// hive.call result codes (HBC_SPEC §6.5).
const (
	CallModuleNotFound   = -1
	CallFunctionNotFound = -2
	CallFailed           = -3
	CallDepthExceeded    = -5
	CallBadAddress       = -6
)

// CallError is returned by Call when the host reports a negative code.
type CallError struct {
	Code    int64
	Address string
	Func    string
}

func (e *CallError) Error() string {
	var why string
	switch e.Code {
	case CallModuleNotFound:
		why = "module not found"
	case CallFunctionNotFound:
		why = "function not found"
	case CallFailed:
		why = "callee failed"
	case CallDepthExceeded:
		why = "call depth exceeded"
	case CallBadAddress:
		why = "malformed module address"
	default:
		why = "error"
	}
	return fmt.Sprintf("hive.call %s.%s: %s (%d)", e.Address, e.Func, why, e.Code)
}

// Call synchronously runs function fn of the module at address (canonical
// `0x` + 64 lowercase hex) with the given input and returns its output. The
// callee's state writes and events are merged into this call only if it
// succeeds; on failure a *CallError is returned and nothing it did is kept.
func Call(address, fn string, input []byte) ([]byte, error) {
	out, code := hostCall([]byte(address), []byte(fn), input)
	if code < 0 {
		return nil, &CallError{Code: code, Address: address, Func: fn}
	}
	return out, nil
}

// CallJSON encodes in as JSON, calls address.fn, and decodes the output into
// out (out may be nil to discard it).
func CallJSON(address, fn string, in any, out any) error {
	var b []byte
	if in != nil {
		var err error
		if b, err = json.Marshal(in); err != nil {
			return err
		}
	}
	res, err := Call(address, fn, b)
	if err != nil {
		return err
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(res, out)
}

// ── hashing, logging, abort ──────────────────────────────────────────────────

// Hash returns keccak256(data) as "0x" + 64 lowercase hex.
func Hash(data []byte) string { return hostHash(data) }

// Log writes a debug line to the node log (no consensus effect; at most 4 KiB).
func Log(msg string) { hostLog([]byte(msg)) }

// Logf is Log with fmt formatting.
func Logf(format string, args ...any) { Log(fmt.Sprintf(format, args...)) }

// Abort fails the current call with msg. It does not return; nothing the call
// wrote or emitted is kept.
func Abort(msg string) {
	hostAbort(msg)
	panic("unreachable")
}
