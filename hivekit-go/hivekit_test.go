package hivekit

import (
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"
)

func resetRegistry() {
	registry = map[string]RawHandler{}
	sorted = nil
	ResetMock()
}

func registerMath() {
	Define("addNumbers", func(in map[string]any) map[string]any {
		a, _ := in["a"].(float64)
		b, _ := in["b"].(float64)
		return map[string]any{"total": a + b}
	})
	DefineJSON("multiply", func(p struct{ A, B float64 }) (map[string]float64, error) {
		return map[string]float64{"result": p.A * p.B}, nil
	})
	Define("greetUser", func(in map[string]any) map[string]any {
		return map[string]any{"message": "hi"}
	})
}

// Function ids are indexes into the sorted list, never registration order:
// the old SDK ran greetUser when multiply was called.
func TestDispatchUsesSortedIndex(t *testing.T) {
	resetRegistry()
	registerMath()
	want := []string{"addNumbers", "greetUser", "multiply"}
	if got := Functions(); !reflect.DeepEqual(got, want) {
		t.Fatalf("Functions() = %v, want %v", got, want)
	}
	for i, name := range want {
		if FuncID(name) != i {
			t.Errorf("FuncID(%s) = %d, want %d", name, FuncID(name), i)
		}
	}
	out, err := dispatch(2, []byte(`{"A":6,"B":7}`))
	if err != nil || string(out) != `{"result":42}` {
		t.Fatalf("dispatch(2) = %s, %v", out, err)
	}
	out, _ = dispatch(1, nil)
	if string(out) != `{"message":"hi"}` {
		t.Fatalf("dispatch(1) = %s", out)
	}
	if _, err := dispatch(3, nil); !errors.Is(err, ErrFunctionNotFound) {
		t.Errorf("dispatch(3) err = %v", err)
	}
	if got := string(functionsJSON()); got != `["addNumbers","greetUser","multiply"]` {
		t.Errorf("functionsJSON = %s", got)
	}
}

func TestDefineValidation(t *testing.T) {
	resetRegistry()
	mustPanic := func(name string, f func()) {
		t.Helper()
		defer func() {
			if recover() == nil {
				t.Errorf("%s: no panic", name)
			}
		}()
		f()
	}
	noop := func([]byte) ([]byte, error) { return nil, nil }
	DefineRaw("ok_1", noop)
	mustPanic("duplicate", func() { DefineRaw("ok_1", noop) })
	mustPanic("digit first", func() { DefineRaw("1x", noop) })
	mustPanic("dash", func() { DefineRaw("a-b", noop) })
	mustPanic("empty", func() { DefineRaw("", noop) })
	mustPanic("too long", func() { DefineRaw(strings.Repeat("a", 65), noop) })
	DefineRaw(strings.Repeat("a", 64), noop)
}

func TestHandlers(t *testing.T) {
	resetRegistry()
	registerMath()
	DefineJSON("fails", func(struct{}) (int, error) { return 0, errors.New("nope") })
	DefineRaw("raw", func(in []byte) ([]byte, error) { return append([]byte("got:"), in...), nil })

	cases := []struct{ fn, in, out, err string }{
		{"addNumbers", `{"a":10,"b":32}`, `{"total":42}`, ""},
		{"addNumbers", ``, `{"total":0}`, ""},
		{"addNumbers", `nope`, ``, "invalid JSON input"},
		{"multiply", `{"A":1.5,"B":2}`, `{"result":3}`, ""},
		{"fails", `{}`, ``, "nope"},
		{"raw", `xyz`, `got:xyz`, ""},
		{"missing", `{}`, ``, "function not found"},
	}
	for _, c := range cases {
		out, err := InvokeLocalJSON(c.fn, c.in)
		if c.err != "" {
			if err == nil || !strings.Contains(err.Error(), c.err) {
				t.Errorf("%s(%s): err = %v, want %q", c.fn, c.in, err, c.err)
			}
			continue
		}
		if err != nil || out != c.out {
			t.Errorf("%s(%s) = %s, %v; want %s", c.fn, c.in, out, err, c.out)
		}
	}
}

func TestMockHostStorageEventsAbort(t *testing.T) {
	resetRegistry()
	DefineJSON("inc", func(in struct{ By int }) (map[string]int, error) {
		var n int
		if _, err := StorageGetJSON("n", &n); err != nil {
			return nil, err
		}
		n += in.By
		if err := StorageSetJSON("n", n); err != nil {
			return nil, err
		}
		if err := Emit("inc", map[string]int{"n": n}); err != nil {
			return nil, err
		}
		if n > 10 {
			Abort("too big")
		}
		return map[string]int{"n": n}, nil
	})
	DefineRaw("strings", func([]byte) ([]byte, error) {
		StorageSetString("k", "v")
		v := StorageGetString("k")
		StorageSet("k", nil) // empty value deletes
		_, present := StorageGet("k")
		StorageSetString("d", "x")
		StorageDel("d")
		Log("hello")
		return []byte(v + "," + map[bool]string{true: "present", false: "absent"}[present] + "," + StorageGetString("d")), nil
	})

	for i, want := range []string{`{"n":4}`, `{"n":8}`} {
		out, err := InvokeLocalJSON("inc", `{"By":4}`)
		if err != nil || out != want {
			t.Fatalf("call %d: %s %v", i, out, err)
		}
	}
	if _, err := InvokeLocalJSON("inc", `{"By":4}`); err == nil || !strings.Contains(err.Error(), "too big") {
		t.Fatalf("abort: %v", err)
	}
	if got := string(Mock.Storage["n"]); got != "8" {
		t.Errorf("aborted call leaked state: n = %s", got)
	}
	if len(Mock.Events) != 2 || Mock.Events[1].Name != "inc" || string(Mock.Events[1].Data) != `{"n":8}` {
		t.Errorf("events = %+v", Mock.Events)
	}
	out, err := InvokeLocalJSON("strings", "")
	if err != nil || out != "v,absent," {
		t.Errorf("strings = %q %v", out, err)
	}
	if len(Mock.Logs) != 1 || Mock.Logs[0] != "hello" {
		t.Errorf("logs = %v", Mock.Logs)
	}
	if got := Mock.StorageKeys(); !reflect.DeepEqual(got, []string{"n"}) {
		t.Errorf("keys = %v", got)
	}
}

func TestMockCallAndHash(t *testing.T) {
	resetRegistry()
	const callee = "0x1111111111111111111111111111111111111111111111111111111111111111"
	Mock.OnCall = func(addr, fn string, in []byte) ([]byte, int64) {
		if addr != callee {
			return nil, CallModuleNotFound
		}
		if fn != "double" {
			return nil, CallFunctionNotFound
		}
		var v struct{ N int }
		json.Unmarshal(in, &v)
		b, _ := json.Marshal(map[string]int{"n": v.N * 2})
		return b, 0
	}
	var out struct{ N int }
	if err := CallJSON(callee, "double", map[string]int{"n": 21}, &out); err != nil || out.N != 42 {
		t.Fatalf("CallJSON = %+v %v", out, err)
	}
	_, err := Call(callee, "triple", nil)
	var ce *CallError
	if !errors.As(err, &ce) || ce.Code != CallFunctionNotFound {
		t.Errorf("err = %v", err)
	}
	if _, err := Call("0x22", "x", nil); err == nil || !strings.Contains(err.Error(), "module not found") {
		t.Errorf("err = %v", err)
	}
	if h := Hash([]byte("abc")); h != "0x4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45" {
		t.Errorf("Hash = %s", h)
	}
}

func TestEmitValidatesName(t *testing.T) {
	resetRegistry()
	DefineRaw("e", func([]byte) ([]byte, error) {
		EmitRaw("bad name", []byte(`{}`))
		return nil, nil
	})
	if _, err := InvokeLocal("e", nil); err == nil {
		t.Error("invalid event name accepted")
	}
}
