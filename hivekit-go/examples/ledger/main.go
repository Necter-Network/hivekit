// ledger: storage + events + hive.call + crypto.hash.
//
// `record` calls math_module.multiply in another module through hive.call,
// adds the product to a running total kept in storage, and emits an event.
//
//	hivec build ./examples/math_module
//	hivec build ./examples/ledger
//	hivec run -data-dir ./state -module dist/math_module.hbc dist/ledger.hbc record \
//	    '{"math":"<math_module manifest_address>","a":6,"b":7}'
//	hivec run -data-dir ./state dist/ledger.hbc total
package main

import (
	"errors"
	"math"

	hivekit "github.com/Necter-Network/hivekit/hivekit-go"
)

type recordIn struct {
	Math string `json:"math"` // address of a deployed math_module
	A    int64  `json:"a"`
	B    int64  `json:"b"`
}

type totals struct {
	Total int64 `json:"total"`
	Count int64 `json:"count"`
}

type recordOut struct {
	Product int64  `json:"product"`
	Total   int64  `json:"total"`
	Count   int64  `json:"count"`
	Receipt string `json:"receipt"`
}

type probeIn struct {
	Address string `json:"address"`
	Fn      string `json:"fn"`
}

type probeOut struct {
	OK     bool   `json:"ok"`
	Code   int64  `json:"code"`
	Output string `json:"output,omitempty"`
}

const stateKey = "totals"

func init() {
	hivekit.DefineJSON("record", func(in recordIn) (recordOut, error) {
		var res struct {
			Result float64 `json:"result"`
		}
		if err := hivekit.CallJSON(in.Math, "multiply", map[string]int64{"a": in.A, "b": in.B}, &res); err != nil {
			return recordOut{}, err
		}
		if res.Result != math.Trunc(res.Result) || math.Abs(res.Result) > 1<<53-1 {
			return recordOut{}, errors.New("product is not a safe integer")
		}
		product := int64(res.Result)

		var t totals
		if _, err := hivekit.StorageGetJSON(stateKey, &t); err != nil {
			return recordOut{}, err
		}
		t.Total += product
		t.Count++
		if err := hivekit.StorageSetJSON(stateKey, t); err != nil {
			return recordOut{}, err
		}
		if err := hivekit.Emit("ledger.recorded", map[string]int64{"product": product, "total": t.Total, "count": t.Count}); err != nil {
			return recordOut{}, err
		}
		hivekit.Logf("recorded %d (total %d)", product, t.Total)
		receipt := hivekit.Hash([]byte(hivekit.StorageGetString(stateKey)))
		return recordOut{Product: product, Total: t.Total, Count: t.Count, Receipt: receipt}, nil
	})

	hivekit.DefineJSON("total", func(struct{}) (totals, error) {
		var t totals
		_, err := hivekit.StorageGetJSON(stateKey, &t)
		return t, err
	})

	// probe makes a raw hive.call and reports the host result code instead of
	// failing, to exercise the error paths (-1 not found, -2 no function,
	// -3 callee failed, -6 malformed address).
	hivekit.DefineJSON("probe", func(in probeIn) (probeOut, error) {
		out, err := hivekit.Call(in.Address, in.Fn, []byte(`{"a":1,"b":0}`))
		var ce *hivekit.CallError
		if errors.As(err, &ce) {
			return probeOut{Code: ce.Code}, nil
		}
		if err != nil {
			return probeOut{}, err
		}
		return probeOut{OK: true, Output: string(out)}, nil
	})

	// fail writes state and emits an event, then aborts: nothing must persist.
	hivekit.DefineJSON("fail", func(struct{}) (totals, error) {
		hivekit.StorageSetJSON(stateKey, totals{Total: -1, Count: -1})
		hivekit.Emit("ledger.never", map[string]bool{"x": true})
		hivekit.Abort("deliberate failure")
		return totals{}, nil
	})
}

func main() {}
