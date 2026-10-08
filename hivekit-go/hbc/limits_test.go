package hbc

import (
	"bytes"
	"encoding/hex"
	"strings"
	"testing"
)

// padTo appends one custom section so the module is exactly total bytes.
func padTo(wasm []byte, total int) []byte {
	size := uint32(total - len(wasm) - 5) // section id + 4-byte LEB size
	out := append([]byte{}, wasm...)
	out = append(out, 0,
		byte(size&0x7f)|0x80, byte((size>>7)&0x7f)|0x80, byte((size>>14)&0x7f)|0x80, byte(size>>21),
		3, 'p', 'a', 'd')
	return append(out, make([]byte, total-len(out))...)
}

func TestSizeLimits(t *testing.T) {
	if MaxWasmBytes != 12<<20 || MaxHBCBytes != 16<<20 {
		t.Fatalf("limits %d/%d do not match HBC_SPEC §2", MaxWasmBytes, MaxHBCBytes)
	}
	base, err := hex.DecodeString(addrVec(t).WasmHex)
	if err != nil {
		t.Fatal(err)
	}
	m := Manifest{Name: "big", Language: "wat", Compiler: "limits-test/1", Runtime: Runtime, Functions: []string{"echo"}}

	// Exactly at the module.wasm limit: builds, packs under the .hbc limit, loads.
	at := padTo(base, MaxWasmBytes)
	art, err := Build(m, at)
	if err != nil {
		t.Fatal(err)
	}
	data, err := art.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	got, err := Load(data)
	if err != nil {
		t.Fatal(err)
	}
	if got.Address != art.Address || len(got.Wasm) != MaxWasmBytes {
		t.Fatalf("round trip mismatch")
	}

	// One byte over: Build refuses, and a hand-made archive is rejected on Load.
	over := padTo(base, MaxWasmBytes+1)
	if _, err := Build(m, over); err == nil {
		t.Fatal("Build accepted an oversized module.wasm")
	}
	var buf bytes.Buffer
	w := newZip(&buf)
	w.add("manifest.json", art.ManifestJSON)
	w.add("module.wasm", over)
	w.close()
	if _, err := Load(buf.Bytes()); err == nil || !strings.Contains(err.Error(), "module.wasm exceeds") {
		t.Fatalf("oversized module.wasm: %v", err)
	}
}
