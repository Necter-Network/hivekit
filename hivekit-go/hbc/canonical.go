// Package hbc builds, reads and verifies NDSR `.hbc` artifacts (HBC_SPEC v1):
// the manifest schema, canonical JSON, the Keccak-256 content address and the
// two-entry ZIP container. It is plain Go (no TinyGo, no cgo).
package hbc

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"sort"
	"strings"
	"unicode/utf8"

	"golang.org/x/crypto/sha3"
)

// MaxSafeInt is the largest integer magnitude canonical JSON accepts (2^53−1).
const MaxSafeInt = 1<<53 - 1

// Keccak256 returns the original (Ethereum) Keccak-256 digest — not FIPS SHA3-256.
func Keccak256(parts ...[]byte) []byte {
	h := sha3.NewLegacyKeccak256()
	for _, p := range parts {
		h.Write(p)
	}
	return h.Sum(nil)
}

// Keccak256Hex returns "0x" + lowercase hex of Keccak256.
func Keccak256Hex(parts ...[]byte) string {
	return "0x" + hex.EncodeToString(Keccak256(parts...))
}

// CanonicalJSON encodes v as canonical JSON (docs/HBC_SPEC.md §4): object keys sorted by
// code point (byte order of UTF-8), no whitespace, non-ASCII emitted as raw
// UTF-8 (only `"`, `\` and control characters are escaped, exactly like
// Python's json.dumps(ensure_ascii=False) and serde_json), integers only with
// |n| ≤ 2^53−1.
//
// Note that encoding/json is not usable here even with SetEscapeHTML(false):
// it still escapes U+2028/U+2029 and would serialize floats.
//
// v may be any value produced by encoding/json decoding (map[string]any,
// []any, string, bool, nil, float64 holding an integer, json.Number), plus
// Go integer types, []string and map[string]string. Other values are first
// round-tripped through encoding/json (decoded with UseNumber).
func CanonicalJSON(v any) ([]byte, error) {
	var buf bytes.Buffer
	if err := writeCanonical(&buf, v); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

// CanonicalizeJSON parses JSON text (rejecting duplicate keys) and returns its
// canonical form.
func CanonicalizeJSON(text []byte) ([]byte, error) {
	v, err := decodeStrict(text)
	if err != nil {
		return nil, err
	}
	return CanonicalJSON(v)
}

func writeCanonical(buf *bytes.Buffer, v any) error {
	switch t := v.(type) {
	case nil:
		buf.WriteString("null")
	case bool:
		if t {
			buf.WriteString("true")
		} else {
			buf.WriteString("false")
		}
	case string:
		return writeString(buf, t)
	case json.Number:
		return writeNumber(buf, string(t))
	case float64:
		if t != math.Trunc(t) || math.IsInf(t, 0) || math.IsNaN(t) {
			return fmt.Errorf("canonical JSON: non-integer number %v", t)
		}
		if math.Abs(t) > MaxSafeInt {
			return fmt.Errorf("canonical JSON: integer %v out of range", t)
		}
		fmt.Fprintf(buf, "%d", int64(t))
	case float32:
		return writeCanonical(buf, float64(t))
	case int:
		return writeInt(buf, int64(t))
	case int8:
		return writeInt(buf, int64(t))
	case int16:
		return writeInt(buf, int64(t))
	case int32:
		return writeInt(buf, int64(t))
	case int64:
		return writeInt(buf, t)
	case uint:
		return writeUint(buf, uint64(t))
	case uint8:
		return writeInt(buf, int64(t))
	case uint16:
		return writeInt(buf, int64(t))
	case uint32:
		return writeInt(buf, int64(t))
	case uint64:
		return writeUint(buf, t)
	case []any:
		buf.WriteByte('[')
		for i, e := range t {
			if i > 0 {
				buf.WriteByte(',')
			}
			if err := writeCanonical(buf, e); err != nil {
				return err
			}
		}
		buf.WriteByte(']')
	case []string:
		buf.WriteByte('[')
		for i, e := range t {
			if i > 0 {
				buf.WriteByte(',')
			}
			if err := writeString(buf, e); err != nil {
				return err
			}
		}
		buf.WriteByte(']')
	case map[string]any:
		keys := make([]string, 0, len(t))
		for k := range t {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		buf.WriteByte('{')
		for i, k := range keys {
			if i > 0 {
				buf.WriteByte(',')
			}
			if err := writeString(buf, k); err != nil {
				return err
			}
			buf.WriteByte(':')
			if err := writeCanonical(buf, t[k]); err != nil {
				return err
			}
		}
		buf.WriteByte('}')
	case map[string]string:
		m := make(map[string]any, len(t))
		for k, s := range t {
			m[k] = s
		}
		return writeCanonical(buf, m)
	default:
		raw, err := json.Marshal(v)
		if err != nil {
			return fmt.Errorf("canonical JSON: %w", err)
		}
		g, err := decodeStrict(raw)
		if err != nil {
			return err
		}
		return writeCanonical(buf, g)
	}
	return nil
}

func writeInt(buf *bytes.Buffer, n int64) error {
	if n > MaxSafeInt || n < -MaxSafeInt {
		return fmt.Errorf("canonical JSON: integer %d out of range", n)
	}
	fmt.Fprintf(buf, "%d", n)
	return nil
}

func writeUint(buf *bytes.Buffer, n uint64) error {
	if n > MaxSafeInt {
		return fmt.Errorf("canonical JSON: integer %d out of range", n)
	}
	fmt.Fprintf(buf, "%d", n)
	return nil
}

func writeNumber(buf *bytes.Buffer, s string) error {
	if strings.ContainsAny(s, ".eE") {
		return fmt.Errorf("canonical JSON: non-integer number %s", s)
	}
	var n int64
	if _, err := fmt.Sscan(s, &n); err != nil {
		return fmt.Errorf("canonical JSON: integer %s out of range", s)
	}
	return writeInt(buf, n)
}

const hexDigits = "0123456789abcdef"

func writeString(buf *bytes.Buffer, s string) error {
	if !utf8.ValidString(s) {
		return fmt.Errorf("canonical JSON: string is not valid UTF-8")
	}
	buf.WriteByte('"')
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch c {
		case '"':
			buf.WriteString(`\"`)
		case '\\':
			buf.WriteString(`\\`)
		case '\n':
			buf.WriteString(`\n`)
		case '\r':
			buf.WriteString(`\r`)
		case '\t':
			buf.WriteString(`\t`)
		case '\b':
			buf.WriteString(`\b`)
		case '\f':
			buf.WriteString(`\f`)
		default:
			if c < 0x20 {
				buf.WriteString(`\u00`)
				buf.WriteByte(hexDigits[c>>4])
				buf.WriteByte(hexDigits[c&0xf])
			} else {
				buf.WriteByte(c)
			}
		}
	}
	buf.WriteByte('"')
	return nil
}

// decodeStrict parses one JSON value with UseNumber, rejecting duplicate object
// keys and trailing data.
func decodeStrict(text []byte) (any, error) {
	dec := json.NewDecoder(bytes.NewReader(text))
	dec.UseNumber()
	v, err := decodeValue(dec)
	if err != nil {
		return nil, err
	}
	if rest := bytes.TrimSpace(text[dec.InputOffset():]); len(rest) > 0 {
		return nil, fmt.Errorf("JSON: trailing data")
	}
	return v, nil
}

func decodeValue(dec *json.Decoder) (any, error) {
	tok, err := dec.Token()
	if err != nil {
		return nil, fmt.Errorf("JSON: %w", err)
	}
	switch t := tok.(type) {
	case json.Delim:
		switch t {
		case '{':
			m := map[string]any{}
			for dec.More() {
				kt, err := dec.Token()
				if err != nil {
					return nil, fmt.Errorf("JSON: %w", err)
				}
				k := kt.(string)
				if _, dup := m[k]; dup {
					return nil, fmt.Errorf("JSON: duplicate key %q", k)
				}
				v, err := decodeValue(dec)
				if err != nil {
					return nil, err
				}
				m[k] = v
			}
			if _, err := dec.Token(); err != nil {
				return nil, fmt.Errorf("JSON: %w", err)
			}
			return m, nil
		case '[':
			a := []any{}
			for dec.More() {
				v, err := decodeValue(dec)
				if err != nil {
					return nil, err
				}
				a = append(a, v)
			}
			if _, err := dec.Token(); err != nil {
				return nil, fmt.Errorf("JSON: %w", err)
			}
			return a, nil
		}
		return nil, fmt.Errorf("JSON: unexpected %v", t)
	default:
		return t, nil
	}
}
