package hbc

import (
	"archive/zip"
	"bytes"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// vectors loads docs/test-vectors.json from the repository root when it
// can be found above this directory, else the vendored copy in testdata.
func vectors(t *testing.T) map[string]json.RawMessage {
	t.Helper()
	path := filepath.Join("testdata", "test-vectors.json")
	if dir, err := filepath.Abs("."); err == nil {
		for {
			p := filepath.Join(dir, "docs", "test-vectors.json")
			if _, err := os.Stat(p); err == nil {
				path = p
				break
			}
			parent := filepath.Dir(dir)
			if parent == dir {
				break
			}
			dir = parent
		}
	}
	t.Logf("test vectors: %s", path)
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var v map[string]json.RawMessage
	if err := json.Unmarshal(b, &v); err != nil {
		t.Fatal(err)
	}
	return v
}

func TestVectorCanonicalJSON(t *testing.T) {
	var cases []struct {
		Canonical string          `json:"canonical"`
		Input     json.RawMessage `json:"input"`
	}
	if err := json.Unmarshal(vectors(t)["canonical_json"], &cases); err != nil {
		t.Fatal(err)
	}
	if len(cases) == 0 {
		t.Fatal("no canonical_json vectors")
	}
	for _, c := range cases {
		got, err := CanonicalizeJSON(c.Input)
		if err != nil {
			t.Fatal(err)
		}
		if string(got) != c.Canonical {
			t.Errorf("canonical_json:\n got  %s\n want %s", got, c.Canonical)
		}
	}
}

func TestVectorKeccak(t *testing.T) {
	var cases []struct {
		Hash  string `json:"hash"`
		Input string `json:"input_utf8"`
	}
	if err := json.Unmarshal(vectors(t)["keccak256"], &cases); err != nil {
		t.Fatal(err)
	}
	for _, c := range cases {
		if got := Keccak256Hex([]byte(c.Input)); got != c.Hash {
			t.Errorf("keccak256(%q) = %s, want %s", c.Input, got, c.Hash)
		}
	}
}

type addrVector struct {
	CanonicalManifest string          `json:"canonical_manifest"`
	HBCBase64         string          `json:"hbc_base64"`
	Manifest          json.RawMessage `json:"manifest"`
	ManifestAddress   string          `json:"manifest_address"`
	WasmHex           string          `json:"wasm_hex"`
}

func addrVec(t *testing.T) addrVector {
	var v addrVector
	if err := json.Unmarshal(vectors(t)["manifest_address"], &v); err != nil {
		t.Fatal(err)
	}
	return v
}

func TestVectorManifestAddress(t *testing.T) {
	v := addrVec(t)
	wasm, err := hex.DecodeString(v.WasmHex)
	if err != nil {
		t.Fatal(err)
	}
	m, err := ParseManifest(v.Manifest)
	if err != nil {
		t.Fatal(err)
	}
	c, err := m.CanonicalBytes()
	if err != nil {
		t.Fatal(err)
	}
	if string(c) != v.CanonicalManifest {
		t.Errorf("canonical manifest:\n got  %s\n want %s", c, v.CanonicalManifest)
	}
	addr, err := Address(m, wasm)
	if err != nil {
		t.Fatal(err)
	}
	if addr != v.ManifestAddress {
		t.Errorf("address = %s, want %s", addr, v.ManifestAddress)
	}

	// Building the same manifest through Build gives the same address and a
	// .hbc that loads back to it.
	art, err := Build(*m, wasm)
	if err != nil {
		t.Fatal(err)
	}
	if art.Address != v.ManifestAddress {
		t.Errorf("Build address = %s", art.Address)
	}
	data, err := art.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	back, err := Load(data)
	if err != nil {
		t.Fatal(err)
	}
	if back.Address != v.ManifestAddress || back.Manifest.ManifestAddress != v.ManifestAddress {
		t.Errorf("round trip address = %s / %s", back.Address, back.Manifest.ManifestAddress)
	}
}

func TestVectorLoadReferenceHBC(t *testing.T) {
	v := addrVec(t)
	data, err := base64.StdEncoding.DecodeString(v.HBCBase64)
	if err != nil {
		t.Fatal(err)
	}
	art, err := Load(data)
	if err != nil {
		t.Fatal(err)
	}
	if art.Address != v.ManifestAddress {
		t.Errorf("address = %s, want %s", art.Address, v.ManifestAddress)
	}
	if err := CheckABI(art.Wasm); err != nil {
		t.Errorf("reference module fails ABI check: %v", err)
	}
	if art.Manifest.FuncID("echo") != 0 || art.Manifest.FuncID("nope") != -1 {
		t.Error("FuncID")
	}
	// Tampering with the module must be detected.
	badWasm := append(append([]byte{}, art.Wasm...), 0)
	tampered, _ := (&Artifact{Manifest: art.Manifest, Wasm: badWasm, ManifestJSON: art.ManifestJSON}).Bytes()
	if _, err := Load(tampered); err == nil || !strings.Contains(err.Error(), "does not match") {
		t.Errorf("tampered .hbc accepted: %v", err)
	}
}

func TestCanonicalJSONRules(t *testing.T) {
	ok := map[string]string{
		`{"b":1,"a":{"d":[],"c":null}}`: `{"a":{"c":null,"d":[]},"b":1}`,
		`"\u2028\u2029<>&é✓"`:           "\"\u2028\u2029<>&é✓\"",
		`"\u0000\u001f\u007f\b\f"`:      `"\u0000\u001f` + "\x7f" + `\b\f"`,
		`{"é":1,"z":2,"a":3}`:           `{"a":3,"z":2,"é":1}`,
		`-9007199254740991`:             `-9007199254740991`,
		` [ 1 , true ] `:                `[1,true]`,
	}
	for in, want := range ok {
		got, err := CanonicalizeJSON([]byte(in))
		if err != nil {
			t.Errorf("%s: %v", in, err)
			continue
		}
		if string(got) != want {
			t.Errorf("%s:\n got  %s\n want %s", in, got, want)
		}
	}
	for _, in := range []string{`1.5`, `1e3`, `9007199254740992`, `{"a":1,"a":2}`, `[1] 2`, `"a"x`, `{"a":1}}`} {
		if got, err := CanonicalizeJSON([]byte(in)); err == nil {
			t.Errorf("%s accepted as %s", in, got)
		}
	}
	if _, err := CanonicalJSON(map[string]any{"x": 0.5}); err == nil {
		t.Error("float accepted")
	}
	if b, err := CanonicalJSON(map[string]any{"n": int64(7), "s": []string{"b", "a"}}); err != nil || string(b) != `{"n":7,"s":["b","a"]}` {
		t.Errorf("got %s %v", b, err)
	}
}

func TestManifestRules(t *testing.T) {
	base := `"compiler":"c","language":"go","name":"n","runtime":"hive-wasm-v1"`
	good := []string{
		`{` + base + `,"functions":["a","b"]}`,
		`{` + base + `,"functions":["B","a","b_1"],"version":"1","description":"d"}`,
	}
	for _, g := range good {
		if _, err := ParseManifest([]byte(g)); err != nil {
			t.Errorf("%s: %v", g, err)
		}
	}
	bad := []string{
		`{` + base + `,"functions":["b","a"]}`,                       // unsorted
		`{` + base + `,"functions":["a","a"]}`,                       // duplicate
		`{` + base + `,"functions":[]}`,                              // empty
		`{` + base + `,"functions":["1a"]}`,                          // bad name
		`{` + base + `,"functions":["a"],"created_at":"x"}`,          // unknown key
		`{` + base + `,"functions":["a"],"consensus":true}`,          // rejected v0 key
		`{` + base + `,"functions":["a"],"nrc1":{}}`,                 // rejected v0 key
		`{` + base + `,"functions":["a"],"manifest_address":"0xAB"}`, // malformed
		`{"compiler":"c","language":"go","name":"n","runtime":"wasm32-wasi","functions":["a"]}`,
		`{"compiler":"c","language":"Go","name":"n","runtime":"hive-wasm-v1","functions":["a"]}`,
		`{"compiler":"c","language":"go","runtime":"hive-wasm-v1","functions":["a"]}`,
		`{` + base + `,"functions":["a"],"name":"dup"}`,
		`{` + base + `,"functions":["a"],"version":1}`,
	}
	for _, b := range bad {
		if _, err := ParseManifest([]byte(b)); err == nil {
			t.Errorf("accepted %s", b)
		}
	}
	if _, err := Build(Manifest{Name: "n", Language: "go", Compiler: "c", Runtime: Runtime, Functions: []string{"b", "a"}}, []byte("\x00asm\x01\x00\x00\x00")); err == nil {
		t.Error("Build accepted unsorted functions")
	}
}

func TestDeterministicArchive(t *testing.T) {
	m := Manifest{Name: "n", Language: "go", Compiler: "c", Runtime: Runtime, Functions: []string{"a"}}
	wasm := []byte("\x00asm\x01\x00\x00\x00")
	a1, _ := Build(m, wasm)
	a2, _ := Build(m, wasm)
	b1, _ := a1.Bytes()
	b2, _ := a2.Bytes()
	if !bytes.Equal(b1, b2) {
		t.Error("archive bytes differ between identical builds")
	}
	if !bytes.Equal(a1.ManifestJSON, []byte(`{"compiler":"c","functions":["a"],"language":"go","manifest_address":"`+a1.Address+`","name":"n","runtime":"hive-wasm-v1"}`)) {
		t.Errorf("manifest.json = %s", a1.ManifestJSON)
	}
}

func TestLoadRejectsExtraEntries(t *testing.T) {
	v := addrVec(t)
	data, _ := base64.StdEncoding.DecodeString(v.HBCBase64)
	art, err := Load(data)
	if err != nil {
		t.Fatal(err)
	}
	// Re-pack with an extra source entry, as the old SDK did with module.go.
	var buf bytes.Buffer
	w := newZip(&buf)
	w.add("manifest.json", art.ManifestJSON)
	w.add("module.wasm", art.Wasm)
	w.add("module.go", []byte("package main"))
	w.close()
	if _, err := Load(buf.Bytes()); err == nil {
		t.Error("extra entry accepted")
	}
	buf.Reset()
	w = newZip(&buf)
	w.add("manifest.json", art.ManifestJSON)
	w.add("module.go", []byte("package main"))
	w.close()
	if _, err := Load(buf.Bytes()); err == nil {
		t.Error("source-only artifact accepted")
	}
}

// wasmModule assembles a tiny module with the given imports and exports
// (functions only, each with an empty body) for ABI-check tests.
func wasmModule(imports [][3]any, exports [][2]any, memory bool) []byte {
	var types []FuncType
	typeIdx := func(ft FuncType) byte {
		for i, t := range types {
			if t.String() == ft.String() {
				return byte(i)
			}
		}
		types = append(types, ft)
		return byte(len(types) - 1)
	}
	var imp, fn, exp, code []byte
	str := func(s string) []byte { return append([]byte{byte(len(s))}, s...) }
	for _, im := range imports {
		imp = append(imp, str(im[0].(string))...)
		imp = append(imp, str(im[1].(string))...)
		imp = append(imp, 0, typeIdx(im[2].(FuncType)))
	}
	for i, ex := range exports {
		fn = append(fn, typeIdx(ex[1].(FuncType)))
		exp = append(exp, str(ex[0].(string))...)
		exp = append(exp, 0, byte(len(imports)+i))
		ft := ex[1].(FuncType)
		body := []byte{0} // no locals
		for _, r := range ft.Results {
			switch r {
			case I32:
				body = append(body, 0x41, 0)
			case I64:
				body = append(body, 0x42, 0)
			}
		}
		body = append(body, 0x0b)
		code = append(code, byte(len(body)))
		code = append(code, body...)
	}
	if memory {
		exp = append(exp, str("memory")...)
		exp = append(exp, 2, 0)
	}
	var ty []byte
	for _, t := range types {
		ty = append(ty, 0x60, byte(len(t.Params)))
		ty = append(ty, t.Params...)
		ty = append(ty, byte(len(t.Results)))
		ty = append(ty, t.Results...)
	}
	sec := func(id byte, n int, body []byte) []byte {
		b := append([]byte{byte(n)}, body...)
		return append([]byte{id, byte(len(b))}, b...)
	}
	nExp := len(exports)
	if memory {
		nExp++
	}
	out := []byte("\x00asm\x01\x00\x00\x00")
	out = append(out, sec(1, len(types), ty)...)
	if len(imports) > 0 {
		out = append(out, sec(2, len(imports), imp)...)
	}
	out = append(out, sec(3, len(exports), fn)...)
	if memory {
		out = append(out, sec(5, 1, []byte{0, 1})...)
	}
	out = append(out, sec(7, nExp, exp)...)
	out = append(out, sec(10, len(exports), code)...)
	return out
}

func TestCheckABI(t *testing.T) {
	alloc := [2]any{"__alloc", FuncType{[]byte{I32}, []byte{I32}}}
	entry := [2]any{"__hive_entry", FuncType{[]byte{I32, I32, I32}, []byte{I64}}}
	get := [3]any{"storage", "get", HostImports["storage.get"]}

	if err := CheckABI(wasmModule([][3]any{get}, [][2]any{alloc, entry}, true)); err != nil {
		t.Errorf("valid module rejected: %v", err)
	}
	cases := map[string][]byte{
		"wasi":          wasmModule([][3]any{{"wasi_snapshot_preview1", "fd_write", FuncType{[]byte{I32, I32, I32, I32}, []byte{I32}}}}, [][2]any{alloc, entry}, true),
		"unknown":       wasmModule([][3]any{{"env", "__hive_storage_get", FuncType{[]byte{I32, I32}, []byte{I32}}}}, [][2]any{alloc, entry}, true),
		"bad signature": wasmModule([][3]any{{"storage", "get", FuncType{[]byte{I32, I32}, []byte{I32}}}}, [][2]any{alloc, entry}, true),
		"no memory":     wasmModule(nil, [][2]any{alloc, entry}, false),
		"no alloc":      wasmModule(nil, [][2]any{entry}, true),
		"legacy entry":  wasmModule(nil, [][2]any{alloc, {"__hive_entry_str", FuncType{[]byte{I32, I32, I32}, []byte{I32}}}}, true),
		"entry i32":     wasmModule(nil, [][2]any{alloc, {"__hive_entry", FuncType{[]byte{I32, I32, I32}, []byte{I32}}}}, true),
	}
	for name, w := range cases {
		if err := CheckABI(w); err == nil {
			t.Errorf("%s: accepted", name)
		} else {
			t.Logf("%s: %v", name, err)
		}
	}
}

type zipW struct{ w *zip.Writer }

func newZip(buf *bytes.Buffer) *zipW { return &zipW{zip.NewWriter(buf)} }

func (z *zipW) add(name string, data []byte) {
	f, _ := z.w.Create(name)
	f.Write(data)
}

func (z *zipW) close() { z.w.Close() }
