package main

import (
	"errors"

	hivekit "github.com/Necter-Network/hivekit/hivekit-go"
)

type pair struct {
	A int64 `json:"a"`
	B int64 `json:"b"`
}

func init() {
	// Typed: JSON in, JSON out; a returned error fails the call.
	hivekit.DefineJSON("addNumbers", func(p pair) (map[string]int64, error) {
		return map[string]int64{"total": p.A + p.B}, nil
	})

	// Persistent state + an event.
	hivekit.DefineJSON("increment", func(in struct {
		By int64 `json:"by"`
	}) (map[string]int64, error) {
		if in.By == 0 {
			in.By = 1
		}
		if in.By < 0 {
			return nil, errors.New("by must be positive")
		}
		var n int64
		if _, err := hivekit.StorageGetJSON("n", &n); err != nil {
			return nil, err
		}
		n += in.By
		if err := hivekit.StorageSetJSON("n", n); err != nil {
			return nil, err
		}
		if err := hivekit.Emit("incremented", map[string]int64{"value": n}); err != nil {
			return nil, err
		}
		return map[string]int64{"value": n}, nil
	})
}

func main() {} // required by Go, never called by NDSR
