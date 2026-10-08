//go:build tinygo.wasm

package hivekit

import "unsafe"

// Host imports, exact signatures from HBC_SPEC §6.4. All pointers/lengths are
// i32 (unsigned on the host side); byte-string results are packed i64.

//go:wasmimport hive call
func impCall(addrPtr, addrLen, fnPtr, fnLen, inPtr, inLen uint32) int64

//go:wasmimport hive emit
func impEmit(namePtr, nameLen, dataPtr, dataLen uint32)

//go:wasmimport hive abort
func impAbort(msgPtr, msgLen uint32)

//go:wasmimport storage get
func impStorageGet(keyPtr, keyLen uint32) int64

//go:wasmimport storage set
func impStorageSet(keyPtr, keyLen, valPtr, valLen uint32)

//go:wasmimport storage del
func impStorageDel(keyPtr, keyLen uint32)

//go:wasmimport console log
func impLog(ptr, n uint32)

//go:wasmimport crypto hash
func impHash(ptr, n uint32) int64

func ptrLen(b []byte) (uint32, uint32) {
	if len(b) == 0 {
		return 0, 0
	}
	return uint32(uintptr(unsafe.Pointer(&b[0]))), uint32(len(b))
}

func result(v int64) []byte {
	if v <= 0 {
		return nil
	}
	return take(unpack(v))
}

func hostStorageGet(key []byte) []byte {
	kp, kl := ptrLen(key)
	return result(impStorageGet(kp, kl))
}

func hostStorageSet(key, val []byte) {
	kp, kl := ptrLen(key)
	vp, vl := ptrLen(val)
	impStorageSet(kp, kl, vp, vl)
}

func hostStorageDel(key []byte) {
	kp, kl := ptrLen(key)
	impStorageDel(kp, kl)
}

func hostEmit(name, data []byte) {
	np, nl := ptrLen(name)
	dp, dl := ptrLen(data)
	impEmit(np, nl, dp, dl)
}

func hostCall(addr, fn, in []byte) ([]byte, int64) {
	ap, al := ptrLen(addr)
	fp, fl := ptrLen(fn)
	ip, il := ptrLen(in)
	v := impCall(ap, al, fp, fl, ip, il)
	if v < 0 {
		return nil, v
	}
	return result(v), 0
}

func hostHash(b []byte) string {
	p, n := ptrLen(b)
	return string(result(impHash(p, n)))
}

func hostLog(b []byte) {
	p, n := ptrLen(b)
	impLog(p, n)
}

func hostAbort(msg string) {
	b := []byte(msg)
	if len(b) > 1024 {
		b = b[:1024]
	}
	p, n := ptrLen(b)
	impAbort(p, n)
}
