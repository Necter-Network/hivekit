package main

// End-to-end acceptance: build the examples with TinyGo, package them with
// hivec, and execute them under the reference runtime (tools/ndsr): inspect,
// outputs, storage persistence across runs, events, hive.call (including its
// error codes) and failure atomicity.
//
// Requires TinyGo ($TINYGO or tinygo on PATH) and ndsr ($NDSR_BIN, ndsr on
// PATH, or tools/ndsr in a parent directory). Each test is skipped, with the
// reason, only when one of those binaries is absent.

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/Necter-Network/hivekit/hivekit-go/hbc"
)

type built struct {
	path string
	art  *hbc.Artifact
	wasm []byte
}

var (
	buildOnce sync.Once
	builds    map[string]*built
	buildErr  error
	buildDir  string
)

func tools(t *testing.T) (tinygo, ndsr string) {
	t.Helper()
	tg, err := findTinyGo("")
	if err != nil {
		t.Skip("skipping: " + err.Error())
	}
	if _, err := exec.LookPath(tg); err != nil {
		t.Skipf("skipping: tinygo %q not runnable", tg)
	}
	nd := findNDSR("")
	if nd == "" {
		t.Skip("skipping: ndsr binary not found (set NDSR_BIN or keep tools/ndsr in a parent directory)")
	}
	return tg, nd
}

func TestMain(m *testing.M) {
	summaryOut = io.Discard
	code := m.Run()
	if buildDir != "" {
		os.RemoveAll(buildDir)
	}
	os.Exit(code)
}

// buildExamples compiles every example once per test binary.
func buildExamples(t *testing.T) map[string]*built {
	tg, _ := tools(t)
	buildOnce.Do(func() {
		buildDir, buildErr = os.MkdirTemp("", "hivec-it-*")
		if buildErr != nil {
			return
		}
		builds = map[string]*built{}
		for _, name := range []string{"math_module", "price_oracle", "ledger"} {
			b, err := buildExample(tg, name, buildDir)
			if err != nil {
				buildErr = fmt.Errorf("%s: %w", name, err)
				return
			}
			builds[name] = b
		}
	})
	if buildErr != nil {
		t.Fatal(buildErr)
	}
	return builds
}

func buildExample(tg, name, dir string) (*built, error) {
	wasmPath := filepath.Join(dir, name+".wasm")
	v, err := tinygoBuild(tg, filepath.Join("..", "..", "examples", name), wasmPath, "")
	if err != nil {
		return nil, err
	}
	wasm, err := os.ReadFile(wasmPath)
	if err != nil {
		return nil, err
	}
	if _, err := packageWasm(wasm, manifestFlags{out: dir, name: name}, "go", compilerName(v), nil); err != nil {
		return nil, err
	}
	p := filepath.Join(dir, name+".hbc")
	data, err := os.ReadFile(p)
	if err != nil {
		return nil, err
	}
	art, err := hbc.Load(data)
	if err != nil {
		return nil, err
	}
	return &built{path: p, art: art, wasm: wasm}, nil
}

type runResult struct {
	Success bool   `json:"success"`
	Output  string `json:"output"`
	GasUsed int64  `json:"gas_used"`
	Error   string `json:"error"`
	Receipt struct {
		Receipt struct {
			ModuleAddress string `json:"module_address"`
			Function      string `json:"function"`
			Events        []struct {
				Name string          `json:"name"`
				Data json.RawMessage `json:"data"`
			} `json:"events"`
		} `json:"receipt"`
	} `json:"receipt"`
}

func ndsrRun(t *testing.T, args ...string) runResult {
	t.Helper()
	_, nd := tools(t)
	cmd := exec.Command(nd, append([]string{"run"}, args...)...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	err := cmd.Run()
	var r runResult
	if jerr := json.Unmarshal(stdout.Bytes(), &r); jerr != nil {
		t.Fatalf("ndsr run %v: %v\nstdout: %s\nstderr: %s", args, err, stdout.String(), stderr.String())
	}
	if r.Success != (err == nil) {
		t.Fatalf("ndsr run %v: success=%v but exit err=%v", args, r.Success, err)
	}
	return r
}

func canon(t *testing.T, raw json.RawMessage) string {
	t.Helper()
	c, err := hbc.CanonicalizeJSON(raw)
	if err != nil {
		t.Fatal(err)
	}
	return string(c)
}

func mustSucceed(t *testing.T, r runResult, wantOutput string) {
	t.Helper()
	if !r.Success {
		t.Fatalf("call failed: %s", r.Error)
	}
	if wantOutput != "" && r.Output != wantOutput {
		t.Fatalf("output = %s, want %s", r.Output, wantOutput)
	}
}

func TestNDSRInspectAndImports(t *testing.T) {
	bs := buildExamples(t)
	_, nd := tools(t)
	for name, b := range bs {
		// Only spec host imports, no WASI.
		wi, err := hbc.ParseInterface(b.wasm)
		if err != nil {
			t.Fatal(err)
		}
		for _, im := range wi.Imports {
			key := im.Module + "." + im.Name
			if _, ok := hbc.HostImports[key]; !ok || strings.HasPrefix(im.Module, "wasi") {
				t.Errorf("%s imports %s", name, key)
			}
		}
		if wi.HasStart {
			t.Logf("%s has a start section", name)
		}
		out, err := exec.Command(nd, "inspect", b.path).CombinedOutput()
		if err != nil {
			t.Fatalf("ndsr inspect %s: %v\n%s", name, err, out)
		}
		var ins struct {
			ABIValid        bool   `json:"abi_valid"`
			ABIError        any    `json:"abi_error"`
			ManifestAddress string `json:"manifest_address"`
			Functions       []struct {
				FuncID int    `json:"func_id"`
				Name   string `json:"name"`
			} `json:"functions"`
		}
		if err := json.Unmarshal(out, &ins); err != nil {
			t.Fatalf("%s: %v\n%s", name, err, out)
		}
		if !ins.ABIValid {
			t.Errorf("%s: ndsr says ABI invalid: %v", name, ins.ABIError)
		}
		if ins.ManifestAddress != b.art.Address {
			t.Errorf("%s: ndsr address %s, hivec address %s", name, ins.ManifestAddress, b.art.Address)
		}
		for i, f := range ins.Functions {
			if f.FuncID != i || f.Name != b.art.Manifest.Functions[i] {
				t.Errorf("%s: ndsr function %d = %+v, manifest has %s", name, i, f, b.art.Manifest.Functions[i])
			}
		}
	}
}

func TestNDSRMathModule(t *testing.T) {
	m := buildExamples(t)["math_module"]
	want := []string{"addNumbers", "divide", "greetUser", "multiply"}
	if strings.Join(m.art.Manifest.Functions, ",") != strings.Join(want, ",") {
		t.Fatalf("functions = %v", m.art.Manifest.Functions)
	}
	// multiply is id 3 in the sorted list; the old SDK dispatched by
	// registration order and ran greetUser here.
	mustSucceed(t, ndsrRun(t, m.path, "multiply", "--input", `{"a":7,"b":6}`), `{"result":42}`)
	mustSucceed(t, ndsrRun(t, m.path, "addNumbers", "--input", `{"a":10,"b":32}`), `{"total":42}`)
	mustSucceed(t, ndsrRun(t, m.path, "greetUser", "--input", `{"name":"Ada"}`), `{"message":"Hello, Ada!"}`)
	mustSucceed(t, ndsrRun(t, m.path, "greetUser"), `{"message":"Hello, stranger!"}`) // empty input
	mustSucceed(t, ndsrRun(t, m.path, "divide", "--input", `{"a":9,"b":2}`), `{"result":4.5}`)

	r := ndsrRun(t, m.path, "divide", "--input", `{"a":1,"b":0}`)
	if r.Success || !strings.Contains(r.Error, "guest abort: division by zero") {
		t.Errorf("divide by zero: success=%v error=%q", r.Success, r.Error)
	}
	r = ndsrRun(t, m.path, "multiply", "--input", `not json`)
	if r.Success || !strings.Contains(r.Error, "invalid JSON input") {
		t.Errorf("bad input: success=%v error=%q", r.Success, r.Error)
	}
	r = ndsrRun(t, m.path, "multiply", "--input", `{"a":1,"b":1}`, "--gas", "1000")
	if r.Success || r.GasUsed != 1000 {
		t.Errorf("out of gas: success=%v gas_used=%d error=%q", r.Success, r.GasUsed, r.Error)
	}
}

func TestNDSRPriceOracleStoragePersists(t *testing.T) {
	p := buildExamples(t)["price_oracle"]
	dir := t.TempDir()
	submit := func(src string, price int) runResult {
		return ndsrRun(t, p.path, "submit", "--data-dir", dir, "--input",
			fmt.Sprintf(`{"pair":"ETH/USD","source":%q,"price":%d}`, src, price))
	}
	r := submit("a", 320012)
	mustSucceed(t, r, `{"pair":"ETH/USD","median":320012,"sources":1}`)
	evs := r.Receipt.Receipt.Events
	if len(evs) != 1 || evs[0].Name != "price.submitted" || canon(t, evs[0].Data) != `{"pair":"ETH/USD","price":320012,"source":"a"}` {
		t.Errorf("events = %+v", evs)
	}
	mustSucceed(t, submit("b", 320100), `{"pair":"ETH/USD","median":320056,"sources":2}`)
	mustSucceed(t, submit("c", 319900), `{"pair":"ETH/USD","median":320012,"sources":3}`)
	mustSucceed(t, ndsrRun(t, p.path, "price", "--data-dir", dir, "--input", `{"pair":"ETH/USD"}`),
		`{"pair":"ETH/USD","median":320012,"sources":3}`)

	if r := submit("d", -5); r.Success || !strings.Contains(r.Error, "positive") {
		t.Errorf("negative price: %+v", r)
	}
	r = ndsrRun(t, p.path, "price", "--data-dir", dir, "--input", `{"pair":"BTC/USD"}`)
	if r.Success || !strings.Contains(r.Error, "no prices for BTC/USD") {
		t.Errorf("unknown pair: %+v", r)
	}
	// A fresh data dir has no state.
	r = ndsrRun(t, p.path, "price", "--data-dir", t.TempDir(), "--input", `{"pair":"ETH/USD"}`)
	if r.Success {
		t.Errorf("state leaked into a fresh data dir: %s", r.Output)
	}
	mustSucceed(t, ndsrRun(t, p.path, "reset", "--data-dir", dir, "--input", `{"pair":"ETH/USD"}`), `{"ok":true}`)
	if r := ndsrRun(t, p.path, "price", "--data-dir", dir, "--input", `{"pair":"ETH/USD"}`); r.Success {
		t.Errorf("reset did not delete: %s", r.Output)
	}
}

func TestNDSRLedgerHiveCall(t *testing.T) {
	bs := buildExamples(t)
	math, ledger := bs["math_module"], bs["ledger"]
	dir := t.TempDir()
	// ndsr resolves hive.call targets from <data-dir>/modules/<address>.hbc.
	mdir := filepath.Join(dir, "modules")
	if err := os.MkdirAll(mdir, 0o755); err != nil {
		t.Fatal(err)
	}
	data, _ := os.ReadFile(math.path)
	if err := os.WriteFile(filepath.Join(mdir, math.art.Address+".hbc"), data, 0o644); err != nil {
		t.Fatal(err)
	}
	run := func(fn, input string) runResult {
		return ndsrRun(t, ledger.path, fn, "--data-dir", dir, "--input", input)
	}
	record := func(a, b int) runResult {
		return run("record", fmt.Sprintf(`{"math":%q,"a":%d,"b":%d}`, math.art.Address, a, b))
	}

	r := record(6, 7)
	mustSucceed(t, r, "")
	var out struct {
		Product, Total, Count int64
		Receipt               string
	}
	if err := json.Unmarshal([]byte(r.Output), &out); err != nil {
		t.Fatal(err)
	}
	if out.Product != 42 || out.Total != 42 || out.Count != 1 {
		t.Errorf("record = %s", r.Output)
	}
	// crypto.hash returns keccak256 of the stored totals.
	if want := hbc.Keccak256Hex([]byte(`{"total":42,"count":1}`)); out.Receipt != want {
		t.Errorf("hash = %s, want %s", out.Receipt, want)
	}
	if evs := r.Receipt.Receipt.Events; len(evs) != 1 || evs[0].Name != "ledger.recorded" ||
		canon(t, evs[0].Data) != `{"count":1,"product":42,"total":42}` {
		t.Errorf("events = %+v", evs)
	}
	mustSucceed(t, record(2, 5), "")
	mustSucceed(t, run("total", ""), `{"total":52,"count":2}`)

	// A failing call writes and emits, then aborts: nothing persists.
	if r := run("fail", ""); r.Success || !strings.Contains(r.Error, "deliberate failure") || len(r.Receipt.Receipt.Events) != 0 {
		t.Errorf("fail: %+v", r)
	}
	mustSucceed(t, run("total", ""), `{"total":52,"count":2}`)

	// hive.call error codes.
	probe := func(addr, fn, want string) {
		t.Helper()
		mustSucceed(t, run("probe", fmt.Sprintf(`{"address":%q,"fn":%q}`, addr, fn)), want)
	}
	probe(math.art.Address, "divide", `{"ok":false,"code":-3}`)
	probe(math.art.Address, "nope", `{"ok":false,"code":-2}`)
	probe("0x"+strings.Repeat("1", 64), "x", `{"ok":false,"code":-1}`)
	probe("not-an-address", "x", `{"ok":false,"code":-6}`)
	probe(math.art.Address, "greetUser", `{"ok":true,"code":0,"output":"{\"message\":\"Hello, stranger!\"}"}`)

	// record against a missing module fails the whole call.
	r = run("record", fmt.Sprintf(`{"math":"0x%s","a":1,"b":1}`, strings.Repeat("2", 64)))
	if r.Success || !strings.Contains(r.Error, "module not found") {
		t.Errorf("missing callee: %+v", r)
	}
}

// The hivec binary delegates `run` to ndsr (staging -module callees), and its
// built-in runner agrees with ndsr on outputs.
func TestHivecRunDelegatesAndLocalAgrees(t *testing.T) {
	bs := buildExamples(t)
	_, nd := tools(t)
	bin := filepath.Join(t.TempDir(), "hivec")
	if out, err := exec.Command("go", "build", "-o", bin, ".").CombinedOutput(); err != nil {
		t.Fatalf("go build hivec: %v\n%s", err, out)
	}
	math, ledger := bs["math_module"], bs["ledger"]
	input := fmt.Sprintf(`{"math":%q,"a":3,"b":4}`, math.art.Address)
	hivec := func(args ...string) (map[string]any, int) {
		cmd := exec.Command(bin, args...)
		cmd.Env = append(os.Environ(), "NDSR_BIN="+nd)
		var stdout bytes.Buffer
		cmd.Stdout = &stdout
		err := cmd.Run()
		code := 0
		if ee, ok := err.(*exec.ExitError); ok {
			code = ee.ExitCode()
		} else if err != nil {
			t.Fatal(err)
		}
		var v map[string]any
		if err := json.Unmarshal(stdout.Bytes(), &v); err != nil {
			t.Fatalf("hivec %v: %v\n%s", args, err, stdout.String())
		}
		return v, code
	}
	viaNDSR, code := hivec("run", "-module", math.path, ledger.path, "record", input)
	if code != 0 || viaNDSR["success"] != true || viaNDSR["receipt"] == nil {
		t.Fatalf("hivec run via ndsr: code %d, %v", code, viaNDSR)
	}
	local, code := hivec("run", "-local", "-module", math.path, ledger.path, "record", input)
	if code != 0 || local["success"] != true {
		t.Fatalf("hivec run -local: code %d, %v", code, local)
	}
	if viaNDSR["output"] != local["output"] {
		t.Errorf("outputs differ: ndsr %v, local %v", viaNDSR["output"], local["output"])
	}
	if _, code := hivec("run", ledger.path, "fail"); code != 2 {
		t.Errorf("failed call exit code = %d, want 2", code)
	}
}

// Building the same source twice gives the same address.
func TestReproducibleBuild(t *testing.T) {
	first := buildExamples(t)["math_module"]
	tg, _ := tools(t)
	again, err := buildExample(tg, "math_module", t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if again.art.Address != first.art.Address {
		t.Errorf("address changed between builds: %s vs %s", first.art.Address, again.art.Address)
	}
}
