package hbc

import (
	"archive/zip"
	"bytes"
	"errors"
	"fmt"
	"io"
	"regexp"
	"sort"
	"strings"
)

// Runtime is the only accepted manifest runtime.
const Runtime = "hive-wasm-v1"

// Limits from HBC_SPEC §2.
const (
	MaxHBCBytes      = 16 << 20
	MaxManifestBytes = 64 << 10
	MaxWasmBytes     = 12 << 20 // raised from 8 MiB
)

// Manifest is the complete v1 manifest schema. No other keys exist.
type Manifest struct {
	Name            string
	Language        string
	Compiler        string
	Runtime         string
	Functions       []string
	Version         string // optional
	Description     string // optional
	ManifestAddress string // optional; never part of the hashed bytes
}

var (
	reLanguage = regexp.MustCompile(`^[a-z0-9_+-]{1,32}$`)
	reFunction = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]{0,63}$`)
	reAddress  = regexp.MustCompile(`^0x[0-9a-f]{64}$`)
)

// Validate checks the manifest against HBC_SPEC §3 (not the address).
func (m *Manifest) Validate() error {
	switch {
	case len(m.Name) < 1 || len(m.Name) > 128:
		return errors.New("manifest: name must be 1..128 bytes")
	case !reLanguage.MatchString(m.Language):
		return fmt.Errorf("manifest: language %q must match [a-z0-9_+-]{1,32}", m.Language)
	case len(m.Compiler) < 1 || len(m.Compiler) > 128:
		return errors.New("manifest: compiler must be 1..128 bytes")
	case m.Runtime != Runtime:
		return fmt.Errorf("manifest: runtime must be %q, got %q", Runtime, m.Runtime)
	case len(m.Functions) < 1 || len(m.Functions) > 256:
		return errors.New("manifest: functions must have 1..256 entries")
	case len(m.Version) > 1024:
		return errors.New("manifest: version longer than 1024 bytes")
	case len(m.Description) > 1024:
		return errors.New("manifest: description longer than 1024 bytes")
	}
	for i, f := range m.Functions {
		if !reFunction.MatchString(f) {
			return fmt.Errorf("manifest: invalid function name %q", f)
		}
		if i > 0 && m.Functions[i-1] >= f {
			return fmt.Errorf("manifest: functions must be sorted ascending and unique (%q before %q)", m.Functions[i-1], f)
		}
	}
	if m.ManifestAddress != "" && !reAddress.MatchString(m.ManifestAddress) {
		return fmt.Errorf("manifest: malformed manifest_address %q", m.ManifestAddress)
	}
	return nil
}

// Object returns the manifest as a JSON object, optionally with manifest_address.
func (m *Manifest) Object(withAddress bool) map[string]any {
	o := map[string]any{
		"name":      m.Name,
		"language":  m.Language,
		"compiler":  m.Compiler,
		"runtime":   m.Runtime,
		"functions": append([]string(nil), m.Functions...),
	}
	if m.Version != "" {
		o["version"] = m.Version
	}
	if m.Description != "" {
		o["description"] = m.Description
	}
	if withAddress && m.ManifestAddress != "" {
		o["manifest_address"] = m.ManifestAddress
	}
	return o
}

// CanonicalBytes is canonical_json(manifest − manifest_address): the bytes that
// are hashed into the address.
func (m *Manifest) CanonicalBytes() ([]byte, error) {
	return CanonicalJSON(m.Object(false))
}

// FuncID returns the NDSR function id of name (its index in the sorted list), or -1.
func (m *Manifest) FuncID(name string) int {
	i := sort.SearchStrings(m.Functions, name)
	if i < len(m.Functions) && m.Functions[i] == name {
		return i
	}
	return -1
}

// Address computes keccak256(canonical_json(manifest − manifest_address) ‖ wasm).
func Address(m *Manifest, wasm []byte) (string, error) {
	c, err := m.CanonicalBytes()
	if err != nil {
		return "", err
	}
	return Keccak256Hex(c, wasm), nil
}

// NormalizeAddress lower-cases an address and strips a `hive:` prefix.
func NormalizeAddress(a string) (string, error) {
	a = strings.ToLower(strings.TrimPrefix(strings.TrimSpace(a), "hive:"))
	if !reAddress.MatchString(a) {
		return "", fmt.Errorf("malformed module address %q", a)
	}
	return a, nil
}

// ParseManifest parses manifest.json, rejecting unknown or duplicate keys,
// floats and wrong types.
func ParseManifest(text []byte) (*Manifest, error) {
	if len(text) > MaxManifestBytes {
		return nil, errors.New("manifest.json exceeds 64 KiB")
	}
	v, err := decodeStrict(text)
	if err != nil {
		return nil, fmt.Errorf("manifest.json: %w", err)
	}
	obj, ok := v.(map[string]any)
	if !ok {
		return nil, errors.New("manifest.json must be a JSON object")
	}
	m := &Manifest{}
	str := func(k string, dst *string, required bool) error {
		raw, ok := obj[k]
		if !ok {
			if required {
				return fmt.Errorf("manifest: missing %q", k)
			}
			return nil
		}
		s, ok := raw.(string)
		if !ok {
			return fmt.Errorf("manifest: %q must be a string", k)
		}
		*dst = s
		return nil
	}
	for k := range obj {
		switch k {
		case "name", "language", "compiler", "runtime", "functions", "version", "description", "manifest_address":
		default:
			return nil, fmt.Errorf("manifest: unknown key %q (not allowed by hive-wasm-v1)", k)
		}
	}
	for _, f := range []struct {
		k   string
		dst *string
		req bool
	}{
		{"name", &m.Name, true}, {"language", &m.Language, true}, {"compiler", &m.Compiler, true},
		{"runtime", &m.Runtime, true}, {"version", &m.Version, false}, {"description", &m.Description, false},
		{"manifest_address", &m.ManifestAddress, false},
	} {
		if err := str(f.k, f.dst, f.req); err != nil {
			return nil, err
		}
	}
	fns, ok := obj["functions"].([]any)
	if !ok {
		return nil, errors.New("manifest: \"functions\" must be an array of strings")
	}
	for _, f := range fns {
		s, ok := f.(string)
		if !ok {
			return nil, errors.New("manifest: \"functions\" must be an array of strings")
		}
		m.Functions = append(m.Functions, s)
	}
	if err := m.Validate(); err != nil {
		return nil, err
	}
	return m, nil
}

// Artifact is a loaded, verified .hbc.
type Artifact struct {
	Manifest *Manifest
	Wasm     []byte
	Address  string
	// ManifestJSON is the manifest.json entry as stored.
	ManifestJSON []byte
}

// Build validates m against wasm, computes the address and returns the
// artifact (with Manifest.ManifestAddress set). m.Functions must already be
// sorted and unique — Build does not sort.
func Build(m Manifest, wasm []byte) (*Artifact, error) {
	if len(wasm) > MaxWasmBytes {
		return nil, fmt.Errorf("module.wasm is %d bytes (limit %d)", len(wasm), MaxWasmBytes)
	}
	m.ManifestAddress = ""
	if err := m.Validate(); err != nil {
		return nil, err
	}
	addr, err := Address(&m, wasm)
	if err != nil {
		return nil, err
	}
	m.ManifestAddress = addr
	mj, err := CanonicalJSON(m.Object(true))
	if err != nil {
		return nil, err
	}
	return &Artifact{Manifest: &m, Wasm: wasm, Address: addr, ManifestJSON: mj}, nil
}

// Bytes returns the deterministic .hbc archive: entries manifest.json then
// module.wasm, stored (no compression), zero timestamps.
func (a *Artifact) Bytes() ([]byte, error) {
	var buf bytes.Buffer
	w := zip.NewWriter(&buf)
	for _, e := range []struct {
		name string
		data []byte
	}{{"manifest.json", a.ManifestJSON}, {"module.wasm", a.Wasm}} {
		f, err := w.CreateHeader(&zip.FileHeader{Name: e.name, Method: zip.Store})
		if err != nil {
			return nil, err
		}
		if _, err := f.Write(e.data); err != nil {
			return nil, err
		}
	}
	if err := w.Close(); err != nil {
		return nil, err
	}
	if buf.Len() > MaxHBCBytes {
		return nil, fmt.Errorf(".hbc is %d bytes (limit %d)", buf.Len(), MaxHBCBytes)
	}
	return buf.Bytes(), nil
}

// Load parses and verifies a .hbc: exactly the two entries, size limits, the
// manifest schema, and (when present) manifest_address equal to the computed
// address.
func Load(data []byte) (*Artifact, error) {
	if len(data) > MaxHBCBytes {
		return nil, errors.New(".hbc exceeds 16 MiB")
	}
	r, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return nil, fmt.Errorf("not a .hbc (zip) file: %w", err)
	}
	var mj, wasm []byte
	seen := map[string]bool{}
	for _, f := range r.File {
		if seen[f.Name] {
			return nil, fmt.Errorf(".hbc: duplicate entry %q", f.Name)
		}
		seen[f.Name] = true
		var limit int64
		switch f.Name {
		case "manifest.json":
			limit = MaxManifestBytes
		case "module.wasm":
			limit = MaxWasmBytes
		default:
			return nil, fmt.Errorf(".hbc: unexpected entry %q (only manifest.json and module.wasm are allowed)", f.Name)
		}
		rc, err := f.Open()
		if err != nil {
			return nil, err
		}
		b, err := io.ReadAll(io.LimitReader(rc, limit+1))
		rc.Close()
		if err != nil {
			return nil, err
		}
		if int64(len(b)) > limit {
			return nil, fmt.Errorf(".hbc: %s exceeds %d bytes", f.Name, limit)
		}
		if f.Name == "manifest.json" {
			mj = b
		} else {
			wasm = b
		}
	}
	if mj == nil || wasm == nil {
		return nil, errors.New(".hbc must contain manifest.json and module.wasm")
	}
	m, err := ParseManifest(mj)
	if err != nil {
		return nil, err
	}
	addr, err := Address(m, wasm)
	if err != nil {
		return nil, err
	}
	if m.ManifestAddress != "" && m.ManifestAddress != addr {
		return nil, fmt.Errorf("manifest_address %s does not match computed address %s", m.ManifestAddress, addr)
	}
	return &Artifact{Manifest: m, Wasm: wasm, Address: addr, ManifestJSON: mj}, nil
}
