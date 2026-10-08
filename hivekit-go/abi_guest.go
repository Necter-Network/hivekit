//go:build tinygo.wasm

package hivekit

import "unsafe"

// hive-wasm-v1 guest ABI (HBC_SPEC §6). NDSR instantiates a fresh instance per
// call and never calls `_start` or `_initialize`, so the Go runtime (heap and
// package initializers, including every init() that registers functions) is
// brought up lazily by the first export the host calls: always `__alloc` or
// `__hive_entry`.

//go:linkname runtimeInitialize _initialize
func runtimeInitialize()

var initialized bool

func ensureInit() {
	if !initialized {
		initialized = true
		runtimeInitialize()
	}
}

// pins keeps buffers handed to the host reachable until the guest takes them
// back (input bytes, host results), whatever GC the module is built with.
var pins map[uintptr][]byte

// lastOutput keeps the current output buffer alive after __hive_entry returns.
var lastOutput []byte

//export __alloc
func hiveAlloc(n int32) int32 {
	ensureInit()
	if n <= 0 {
		return 0
	}
	buf := make([]byte, n)
	p := uintptr(unsafe.Pointer(&buf[0]))
	if pins == nil {
		pins = map[uintptr][]byte{}
	}
	pins[p] = buf
	return int32(p)
}

// take returns the bytes at (ptr, len) that the host placed in a buffer from
// __alloc, releasing the pin.
func take(ptr, n uint32) []byte {
	if n == 0 {
		return nil
	}
	if buf, ok := pins[uintptr(ptr)]; ok && uint32(len(buf)) >= n {
		delete(pins, uintptr(ptr))
		return buf[:n:n]
	}
	// Not one of ours: copy out of linear memory.
	src := unsafe.Slice((*byte)(unsafe.Pointer(uintptr(ptr))), n)
	out := make([]byte, n)
	copy(out, src)
	return out
}

func unpack(v int64) (uint32, uint32) {
	return uint32(uint64(v) >> 32), uint32(uint64(v))
}

func pack(b []byte) int64 {
	if len(b) == 0 {
		return 0
	}
	return int64(uint64(uintptr(unsafe.Pointer(&b[0])))<<32 | uint64(len(b)))
}

//export __hive_entry
func hiveEntry(funcID int32, ptr int32, n int32) int64 {
	ensureInit()
	input := take(uint32(ptr), uint32(n))
	out, err := dispatch(int(funcID), input)
	if err != nil {
		if err == ErrFunctionNotFound {
			return -2
		}
		Abort(err.Error())
	}
	lastOutput = out
	return pack(out)
}

// __hive_functions returns the sorted function list as a JSON array (packed).
// It is not part of the NDSR ABI (extra exports are ignored); hivec uses it to
// build the manifest from the compiled module itself.
//
//export __hive_functions
func hiveFunctions() int64 {
	ensureInit()
	lastOutput = functionsJSON()
	return pack(lastOutput)
}
