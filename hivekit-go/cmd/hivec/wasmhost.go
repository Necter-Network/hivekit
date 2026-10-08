package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"unicode/utf8"

	"github.com/Necter-Network/hivekit/hivekit-go/hbc"
	"github.com/tetratelabs/wazero"
	"github.com/tetratelabs/wazero/api"
)

// This file is the built-in development runner (wazero). It implements the
// whole hive-wasm-v1 host interface but is NOT a consensus implementation: no
// gas metering, no receipts, no signatures. `hivec run` uses tools/ndsr when it
// can find it; this runner is the fallback (or forced with -local), and it is
// also how hivec reads a module's function list (`__hive_functions`).

type event struct {
	Name string          `json:"name"`
	Data json.RawMessage `json:"data"`
}

type localHost struct {
	modules map[string]*hbc.Artifact // by address
	state   map[string]map[string][]byte
	events  []event
	logs    []string
}

type frame struct {
	addr  string
	depth int
	mod   api.Module
}

type abortErr struct{ msg string }

func (e *abortErr) Error() string { return e.msg }

const maxDepth = 8

// moduleFunctions instantiates the module with inert host imports and reads
// the JSON function list from its `__hive_functions` export.
func moduleFunctions(wasm []byte) ([]string, error) {
	ctx := context.Background()
	rt := wazero.NewRuntime(ctx)
	defer rt.Close(ctx)
	h := &localHost{state: map[string]map[string][]byte{}}
	var cur *frame
	if err := h.instantiateHost(ctx, rt, func() *frame { return cur }); err != nil {
		return nil, err
	}
	mod, err := rt.InstantiateWithConfig(ctx, wasm, wazero.NewModuleConfig().WithStartFunctions().WithName(""))
	if err != nil {
		return nil, err
	}
	cur = &frame{addr: "introspect", mod: mod}
	fn := mod.ExportedFunction("__hive_functions")
	if fn == nil {
		return nil, errors.New("module has no __hive_functions export (not built with hivekit-go)")
	}
	res, err := fn.Call(ctx)
	if err != nil {
		return nil, fmt.Errorf("__hive_functions: %w", err)
	}
	out, err := readPacked(mod, int64(res[0]))
	if err != nil {
		return nil, err
	}
	var fns []string
	if err := json.Unmarshal(out, &fns); err != nil {
		return nil, fmt.Errorf("__hive_functions returned %q: %w", out, err)
	}
	if len(fns) == 0 {
		return nil, errors.New("the module registers no functions (call hivekit.Define from init)")
	}
	return fns, nil
}

func readPacked(mod api.Module, v int64) ([]byte, error) {
	if v < 0 {
		return nil, fmt.Errorf("guest returned error code %d", v)
	}
	p, n := uint32(uint64(v)>>32), uint32(uint64(v))
	if n == 0 {
		return []byte{}, nil
	}
	b, ok := mod.Memory().Read(p, n)
	if !ok {
		return nil, fmt.Errorf("output range %d+%d outside linear memory", p, n)
	}
	return append([]byte(nil), b...), nil
}

func writeOut(ctx context.Context, mod api.Module, b []byte) int64 {
	if len(b) == 0 {
		return 0
	}
	res, err := mod.ExportedFunction("__alloc").Call(ctx, uint64(len(b)))
	if err != nil {
		panic(err)
	}
	p := uint32(res[0])
	if !mod.Memory().Write(p, b) {
		panic(errors.New("__alloc returned a pointer outside linear memory"))
	}
	return int64(uint64(p)<<32 | uint64(len(b)))
}

func mustRead(mod api.Module, p, n uint32) []byte {
	if n == 0 {
		return nil
	}
	b, ok := mod.Memory().Read(p, n)
	if !ok {
		panic(fmt.Errorf("guest range %d+%d outside linear memory", p, n))
	}
	return append([]byte(nil), b...)
}

// instantiateHost registers the host modules; cur returns the active frame.
func (h *localHost) instantiateHost(ctx context.Context, rt wazero.Runtime, cur func() *frame) error {
	st := func() map[string][]byte {
		f := cur()
		if h.state[f.addr] == nil {
			h.state[f.addr] = map[string][]byte{}
		}
		return h.state[f.addr]
	}
	_, err := rt.NewHostModuleBuilder("storage").
		NewFunctionBuilder().WithFunc(func(ctx context.Context, m api.Module, kp, kl uint32) int64 {
		return writeOut(ctx, m, st()[string(mustRead(m, kp, kl))])
	}).Export("get").
		NewFunctionBuilder().WithFunc(func(_ context.Context, m api.Module, kp, kl, vp, vl uint32) {
		if kl == 0 {
			panic(errors.New("storage key must not be empty"))
		}
		k, v := string(mustRead(m, kp, kl)), mustRead(m, vp, vl)
		if len(v) == 0 {
			delete(st(), k)
		} else {
			st()[k] = v
		}
	}).Export("set").
		NewFunctionBuilder().WithFunc(func(_ context.Context, m api.Module, kp, kl uint32) {
		delete(st(), string(mustRead(m, kp, kl)))
	}).Export("del").
		Instantiate(ctx)
	if err != nil {
		return err
	}
	_, err = rt.NewHostModuleBuilder("console").
		NewFunctionBuilder().WithFunc(func(_ context.Context, m api.Module, p, n uint32) {
		h.logs = append(h.logs, string(mustRead(m, p, n)))
	}).Export("log").Instantiate(ctx)
	if err != nil {
		return err
	}
	_, err = rt.NewHostModuleBuilder("crypto").
		NewFunctionBuilder().WithFunc(func(ctx context.Context, m api.Module, p, n uint32) int64 {
		return writeOut(ctx, m, []byte(hbc.Keccak256Hex(mustRead(m, p, n))))
	}).Export("hash").Instantiate(ctx)
	if err != nil {
		return err
	}
	_, err = rt.NewHostModuleBuilder("env").
		NewFunctionBuilder().WithFunc(func(_ context.Context, _ api.Module, _, _, line, col uint32) {
		panic(&abortErr{fmt.Sprintf("guest abort (AssemblyScript) at line %d, column %d", line, col)})
	}).Export("abort").Instantiate(ctx)
	if err != nil {
		return err
	}
	_, err = rt.NewHostModuleBuilder("hive").
		NewFunctionBuilder().WithFunc(func(_ context.Context, m api.Module, p, n uint32) {
		msg := ""
		if n <= 1024 {
			msg = strings.ToValidUTF8(string(mustRead(m, p, n)), "�")
		}
		panic(&abortErr{"guest abort: " + msg})
	}).Export("abort").
		NewFunctionBuilder().WithFunc(func(_ context.Context, m api.Module, np, nl, dp, dl uint32) {
		name, data := mustRead(m, np, nl), mustRead(m, dp, dl)
		if !validEventName(name) {
			panic(fmt.Errorf("invalid event name %q", name))
		}
		c, err := hbc.CanonicalizeJSON(data)
		if err != nil {
			panic(fmt.Errorf("event data must be canonical-JSON compatible: %w", err))
		}
		h.events = append(h.events, event{string(name), c})
	}).Export("emit").
		NewFunctionBuilder().WithFunc(func(ctx context.Context, m api.Module, ap, al, fp, fl, ip, il uint32) int64 {
		f := cur()
		addr, fn, in := string(mustRead(m, ap, al)), string(mustRead(m, fp, fl)), mustRead(m, ip, il)
		if f.depth+1 > maxDepth {
			return -5
		}
		if _, err := hbc.NormalizeAddress(addr); err != nil || addr != strings.ToLower(addr) {
			return -6
		}
		art := h.modules[addr]
		if art == nil {
			return -1
		}
		id := art.Manifest.FuncID(fn)
		if id < 0 {
			return -2
		}
		out, err := h.exec(ctx, art, id, in, f.depth+1)
		if err != nil {
			return -3
		}
		return writeOut(ctx, m, out)
	}).Export("call").
		Instantiate(ctx)
	return err
}

func validEventName(b []byte) bool {
	if len(b) == 0 || len(b) > 64 {
		return false
	}
	for _, c := range b {
		if !(c == '_' || c == '.' || c == ':' || c == '-' || (c >= '0' && c <= '9') || (c|0x20 >= 'a' && c|0x20 <= 'z')) {
			return false
		}
	}
	return true
}

// exec runs one call in a fresh runtime + instance. State writes and events
// are kept only on success.
func (h *localHost) exec(ctx context.Context, art *hbc.Artifact, funcID int, input []byte, depth int) (out []byte, err error) {
	savedState := map[string]map[string][]byte{}
	for a, kv := range h.state {
		c := map[string][]byte{}
		for k, v := range kv {
			c[k] = v
		}
		savedState[a] = c
	}
	nEvents := len(h.events)
	defer func() {
		if r := recover(); r != nil {
			if e, ok := r.(error); ok {
				err = e
			} else {
				err = fmt.Errorf("%v", r)
			}
		}
		if err != nil {
			h.state = savedState
			h.events = h.events[:nEvents]
		}
	}()

	rt := wazero.NewRuntime(ctx)
	defer rt.Close(ctx)
	if err := hbc.CheckABI(art.Wasm); err != nil {
		return nil, err
	}
	fr := &frame{addr: art.Address, depth: depth}
	if err := h.instantiateHost(ctx, rt, func() *frame { return fr }); err != nil {
		return nil, err
	}
	mod, err := rt.InstantiateWithConfig(ctx, art.Wasm, wazero.NewModuleConfig().WithStartFunctions().WithName(""))
	if err != nil {
		return nil, err
	}
	fr.mod = mod
	var p uint64
	if len(input) > 0 {
		r, err := mod.ExportedFunction("__alloc").Call(ctx, uint64(len(input)))
		if err != nil {
			return nil, err
		}
		p = r[0]
		if !mod.Memory().Write(uint32(p), input) {
			return nil, errors.New("__alloc returned a pointer outside linear memory")
		}
	}
	r, err := mod.ExportedFunction("__hive_entry").Call(ctx, uint64(funcID), p, uint64(len(input)))
	if err != nil {
		var ae *abortErr
		if errors.As(err, &ae) {
			return nil, ae
		}
		return nil, err
	}
	out, err = readPacked(mod, int64(r[0]))
	if err != nil {
		return nil, err
	}
	if !utf8.Valid(out) {
		return nil, errors.New("output is not valid UTF-8")
	}
	return out, nil
}

// runLocal executes art.fn with the development runner. State is loaded from
// and saved to dataDir/hivec-state.json when dataDir is set.
func runLocal(art *hbc.Artifact, fn string, input []byte, extra []*hbc.Artifact, dataDir string) (int, error) {
	h := &localHost{modules: map[string]*hbc.Artifact{art.Address: art}, state: map[string]map[string][]byte{}}
	for _, a := range extra {
		h.modules[a.Address] = a
	}
	statePath := ""
	if dataDir != "" {
		statePath = dataDir + "/hivec-state.json"
		if b, err := os.ReadFile(statePath); err == nil {
			if err := json.Unmarshal(b, &h.state); err != nil {
				return 1, fmt.Errorf("reading %s: %w", statePath, err)
			}
		}
	}
	id := art.Manifest.FuncID(fn)
	res := map[string]any{"runner": "hivec-local (no gas, no receipt)"}
	var code int
	if id < 0 {
		res["success"], res["error"] = false, fmt.Sprintf("function %q not found in manifest", fn)
		code = 2
	} else if out, err := h.exec(context.Background(), art, id, input, 0); err != nil {
		res["success"], res["error"], res["output"] = false, err.Error(), ""
		code = 2
	} else {
		res["success"], res["output"] = true, string(out)
		evs := h.events
		if evs == nil {
			evs = []event{}
		}
		res["events"] = evs
		if statePath != "" {
			if err := os.MkdirAll(dataDir, 0o755); err != nil {
				return 1, err
			}
			b, _ := json.Marshal(h.state)
			if err := os.WriteFile(statePath, b, 0o644); err != nil {
				return 1, err
			}
		}
	}
	if len(h.logs) > 0 {
		res["logs"] = h.logs
	}
	printJSON(res)
	return code, nil
}
