//go:build !tinygo.wasm

package hivekit

import (
	"encoding/hex"
	"fmt"
	"os"
	"sort"

	"golang.org/x/crypto/sha3"
)

// In a native (non-TinyGo) build the host imports are served by MockHost, an
// in-memory stand-in for NDSR so modules can be unit-tested with `go test`.
// It is not a consensus implementation: there is no gas, and InvokeLocal does
// not roll back state on failure unless Atomic is set.

// Event is one emitted event.
type Event struct {
	Name string
	Data []byte // JSON text
}

// MockCallHandler serves hive.call in native builds. Return a negative code to
// simulate a host error (see the Call* constants).
type MockCallHandler func(address, fn string, input []byte) ([]byte, int64)

// MockHost is the native-build host.
type MockHost struct {
	Storage map[string][]byte
	Events  []Event
	Logs    []string
	// OnCall serves hive.call; nil means every call returns CallModuleNotFound.
	OnCall MockCallHandler
	// Atomic discards storage writes and events of a failed InvokeLocal call.
	Atomic bool
	// LogToStderr echoes Log output to stderr.
	LogToStderr bool
}

// Mock is the host used by native builds. Reset it between tests with ResetMock.
var Mock = newMock()

func newMock() *MockHost {
	return &MockHost{Storage: map[string][]byte{}, Atomic: true}
}

// ResetMock replaces Mock with an empty host and returns it.
func ResetMock() *MockHost {
	Mock = newMock()
	return Mock
}

// StorageKeys returns the stored keys, sorted.
func (m *MockHost) StorageKeys() []string {
	keys := make([]string, 0, len(m.Storage))
	for k := range m.Storage {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

// AbortError is the panic value raised by Abort in native builds; InvokeLocal
// turns it into an error.
type AbortError struct{ Message string }

func (e *AbortError) Error() string { return "guest abort: " + e.Message }

// InvokeLocal runs a registered function by name against Mock, the way NDSR
// would run it: an error or Abort fails the call and (with Mock.Atomic) its
// storage writes and events are discarded.
func InvokeLocal(name string, input []byte) (out []byte, err error) {
	id := FuncID(name)
	if id < 0 {
		return nil, fmt.Errorf("%w: %s (have %v)", ErrFunctionNotFound, name, Functions())
	}
	snapshot := map[string][]byte{}
	for k, v := range Mock.Storage {
		snapshot[k] = v
	}
	nEvents := len(Mock.Events)
	defer func() {
		if r := recover(); r != nil {
			if ae, ok := r.(*AbortError); ok {
				err = ae
			} else {
				err = fmt.Errorf("guest panic: %v", r)
			}
		}
		if err != nil && Mock.Atomic {
			Mock.Storage = snapshot
			Mock.Events = Mock.Events[:nEvents]
		}
	}()
	return dispatch(id, input)
}

// InvokeLocalJSON is InvokeLocal with string input and output.
func InvokeLocalJSON(name, input string) (string, error) {
	out, err := InvokeLocal(name, []byte(input))
	return string(out), err
}

func hostStorageGet(key []byte) []byte {
	mustKey(key)
	v := Mock.Storage[string(key)]
	return append([]byte(nil), v...)
}

func hostStorageSet(key, val []byte) {
	mustKey(key)
	if len(val) == 0 {
		delete(Mock.Storage, string(key))
		return
	}
	Mock.Storage[string(key)] = append([]byte(nil), val...)
}

func hostStorageDel(key []byte) {
	delete(Mock.Storage, string(key))
}

func mustKey(key []byte) {
	if len(key) == 0 || len(key) > 256 {
		panic(&AbortError{Message: "storage key must be 1..256 bytes"})
	}
}

func hostEmit(name, data []byte) {
	if !eventNameOK(name) {
		panic(&AbortError{Message: "invalid event name " + string(name)})
	}
	Mock.Events = append(Mock.Events, Event{Name: string(name), Data: append([]byte(nil), data...)})
}

func eventNameOK(b []byte) bool {
	if len(b) == 0 || len(b) > 64 {
		return false
	}
	for _, c := range b {
		ok := c == '_' || c == '.' || c == ':' || c == '-' ||
			(c >= '0' && c <= '9') || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
		if !ok {
			return false
		}
	}
	return true
}

func hostCall(addr, fn, in []byte) ([]byte, int64) {
	if Mock.OnCall == nil {
		return nil, CallModuleNotFound
	}
	out, code := Mock.OnCall(string(addr), string(fn), in)
	if code < 0 {
		return nil, code
	}
	return out, 0
}

func hostHash(b []byte) string {
	h := sha3.NewLegacyKeccak256()
	h.Write(b)
	return "0x" + hex.EncodeToString(h.Sum(nil))
}

func hostLog(b []byte) {
	Mock.Logs = append(Mock.Logs, string(b))
	if Mock.LogToStderr {
		fmt.Fprintln(os.Stderr, string(b))
	}
}

func hostAbort(msg string) {
	panic(&AbortError{Message: msg})
}
