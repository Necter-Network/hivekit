package hbc

import (
	"errors"
	"fmt"
	"strings"
)

// Wasm value types used by the ABI.
const (
	I32 byte = 0x7f
	I64 byte = 0x7e
	F32 byte = 0x7d
	F64 byte = 0x7c
)

// FuncType is a wasm function signature.
type FuncType struct {
	Params, Results []byte
}

func (t FuncType) String() string {
	name := func(v []byte) string {
		s := make([]string, len(v))
		for i, b := range v {
			switch b {
			case I32:
				s[i] = "i32"
			case I64:
				s[i] = "i64"
			case F32:
				s[i] = "f32"
			case F64:
				s[i] = "f64"
			default:
				s[i] = fmt.Sprintf("0x%02x", b)
			}
		}
		return strings.Join(s, ",")
	}
	return "(" + name(t.Params) + ")->(" + name(t.Results) + ")"
}

func (t FuncType) equal(p, r []byte) bool {
	return string(t.Params) == string(p) && string(t.Results) == string(r)
}

// Import is one module import. Kind: 0 func, 1 table, 2 memory, 3 global, 4 tag.
type Import struct {
	Module, Name string
	Kind         byte
	Type         FuncType // functions only
}

// Export is one module export. Kind as for Import.
type Export struct {
	Name  string
	Kind  byte
	Type  FuncType // functions only
	Index uint32
}

// WasmInterface is the import/export surface of a module.
type WasmInterface struct {
	Imports  []Import
	Exports  []Export
	HasStart bool
}

type reader struct {
	b []byte
	i int
}

var errTrunc = errors.New("wasm: truncated module")

func (r *reader) byte() (byte, error) {
	if r.i >= len(r.b) {
		return 0, errTrunc
	}
	c := r.b[r.i]
	r.i++
	return c, nil
}

func (r *reader) u32() (uint32, error) {
	var res uint64
	for shift := 0; shift < 35; shift += 7 {
		c, err := r.byte()
		if err != nil {
			return 0, err
		}
		res |= uint64(c&0x7f) << shift
		if c < 0x80 {
			return uint32(res), nil
		}
	}
	return 0, errors.New("wasm: bad LEB128")
}

func (r *reader) bytes(n uint32) ([]byte, error) {
	if uint64(r.i)+uint64(n) > uint64(len(r.b)) {
		return nil, errTrunc
	}
	s := r.b[r.i : r.i+int(n)]
	r.i += int(n)
	return s, nil
}

func (r *reader) name() (string, error) {
	n, err := r.u32()
	if err != nil {
		return "", err
	}
	b, err := r.bytes(n)
	return string(b), err
}

func (r *reader) limits() error {
	flag, err := r.byte()
	if err != nil {
		return err
	}
	if _, err := r.u32(); err != nil {
		return err
	}
	if flag&1 != 0 {
		if _, err := r.u32(); err != nil {
			return err
		}
	}
	return nil
}

// ParseInterface reads the type, import, function, export and start sections.
func ParseInterface(wasm []byte) (*WasmInterface, error) {
	if len(wasm) < 8 || string(wasm[:4]) != "\x00asm" {
		return nil, errors.New("wasm: bad magic")
	}
	r := &reader{b: wasm, i: 8}
	var types []FuncType
	var funcs []uint32 // type index per function (imports first)
	wi := &WasmInterface{}
	for r.i < len(r.b) {
		id, err := r.byte()
		if err != nil {
			return nil, err
		}
		size, err := r.u32()
		if err != nil {
			return nil, err
		}
		body, err := r.bytes(size)
		if err != nil {
			return nil, err
		}
		s := &reader{b: body}
		switch id {
		case 1: // type
			n, err := s.u32()
			if err != nil {
				return nil, err
			}
			for k := uint32(0); k < n; k++ {
				form, err := s.byte()
				if err != nil {
					return nil, err
				}
				if form != 0x60 {
					return nil, fmt.Errorf("wasm: unsupported type form 0x%02x", form)
				}
				var ft FuncType
				for _, dst := range []*[]byte{&ft.Params, &ft.Results} {
					c, err := s.u32()
					if err != nil {
						return nil, err
					}
					v, err := s.bytes(c)
					if err != nil {
						return nil, err
					}
					*dst = append([]byte{}, v...)
				}
				types = append(types, ft)
			}
		case 2: // import
			n, err := s.u32()
			if err != nil {
				return nil, err
			}
			for k := uint32(0); k < n; k++ {
				var im Import
				if im.Module, err = s.name(); err != nil {
					return nil, err
				}
				if im.Name, err = s.name(); err != nil {
					return nil, err
				}
				if im.Kind, err = s.byte(); err != nil {
					return nil, err
				}
				switch im.Kind {
				case 0:
					ti, err := s.u32()
					if err != nil {
						return nil, err
					}
					if int(ti) >= len(types) {
						return nil, errors.New("wasm: bad type index")
					}
					im.Type = types[ti]
					funcs = append(funcs, ti)
				case 1:
					if _, err := s.byte(); err != nil {
						return nil, err
					}
					if err := s.limits(); err != nil {
						return nil, err
					}
				case 2:
					if err := s.limits(); err != nil {
						return nil, err
					}
				case 3:
					if _, err := s.bytes(2); err != nil {
						return nil, err
					}
				default:
					if _, err := s.bytes(2); err != nil { // tag: attribute + type index (small)
						return nil, err
					}
				}
				wi.Imports = append(wi.Imports, im)
			}
		case 3: // function
			n, err := s.u32()
			if err != nil {
				return nil, err
			}
			for k := uint32(0); k < n; k++ {
				ti, err := s.u32()
				if err != nil {
					return nil, err
				}
				funcs = append(funcs, ti)
			}
		case 7: // export
			n, err := s.u32()
			if err != nil {
				return nil, err
			}
			for k := uint32(0); k < n; k++ {
				var ex Export
				if ex.Name, err = s.name(); err != nil {
					return nil, err
				}
				if ex.Kind, err = s.byte(); err != nil {
					return nil, err
				}
				if ex.Index, err = s.u32(); err != nil {
					return nil, err
				}
				if ex.Kind == 0 {
					if int(ex.Index) >= len(funcs) || int(funcs[ex.Index]) >= len(types) {
						return nil, errors.New("wasm: bad function index in export")
					}
					ex.Type = types[funcs[ex.Index]]
				}
				wi.Exports = append(wi.Exports, ex)
			}
		case 8:
			wi.HasStart = true
		}
	}
	return wi, nil
}

// HostImports lists every import hive-wasm-v1 allows, with its exact signature
// (HBC_SPEC §6.4).
var HostImports = map[string]FuncType{
	"hive.call":   {[]byte{I32, I32, I32, I32, I32, I32}, []byte{I64}},
	"hive.emit":   {[]byte{I32, I32, I32, I32}, nil},
	"hive.abort":  {[]byte{I32, I32}, nil},
	"storage.get": {[]byte{I32, I32}, []byte{I64}},
	"storage.set": {[]byte{I32, I32, I32, I32}, nil},
	"storage.del": {[]byte{I32, I32}, nil},
	"console.log": {[]byte{I32, I32}, nil},
	"crypto.hash": {[]byte{I32, I32}, []byte{I64}},
	"env.abort":   {[]byte{I32, I32, I32, I32}, nil},
}

// CheckABI verifies a module against the hive-wasm-v1 guest ABI: only allowed
// host imports with exact signatures (no WASI, no imported memory/table/global),
// and the exports memory, __alloc(i32)->i32, __hive_entry(i32,i32,i32)->i64.
func CheckABI(wasm []byte) error {
	wi, err := ParseInterface(wasm)
	if err != nil {
		return err
	}
	for _, im := range wi.Imports {
		key := im.Module + "." + im.Name
		if im.Kind != 0 {
			return fmt.Errorf("import %s: only function imports are allowed", key)
		}
		want, ok := HostImports[key]
		if !ok {
			if strings.HasPrefix(im.Module, "wasi") {
				return fmt.Errorf("import %s: WASI is not supported (build with -target=wasm-unknown)", key)
			}
			return fmt.Errorf("import %s is not part of the hive-wasm-v1 host interface", key)
		}
		if !im.Type.equal(want.Params, want.Results) {
			return fmt.Errorf("import %s has signature %s, want %s", key, im.Type, want)
		}
	}
	var mem, alloc, entry bool
	for _, ex := range wi.Exports {
		switch ex.Name {
		case "memory":
			mem = ex.Kind == 2
		case "__alloc":
			if ex.Kind != 0 || !ex.Type.equal([]byte{I32}, []byte{I32}) {
				return fmt.Errorf("__alloc must be (i32)->(i32), got %s", ex.Type)
			}
			alloc = true
		case "__hive_entry":
			if ex.Kind != 0 || !ex.Type.equal([]byte{I32, I32, I32}, []byte{I64}) {
				return fmt.Errorf("__hive_entry must be (i32,i32,i32)->(i64), got %s", ex.Type)
			}
			entry = true
		case "__hive_entry_str", "__str_len":
			return fmt.Errorf("legacy export %s: the __hive_entry_str ABI is not supported", ex.Name)
		}
	}
	switch {
	case !mem:
		return errors.New("module must export its memory as \"memory\"")
	case !alloc:
		return errors.New("module must export __alloc")
	case !entry:
		return errors.New("module must export __hive_entry")
	}
	return nil
}
