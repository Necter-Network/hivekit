// math_module: pure functions, no host state.
//
//	hivec build ./examples/math_module
//	hivec run dist/math_module.hbc multiply '{"a":7,"b":6}'
package main

import (
	"errors"

	hivekit "github.com/Necter-Network/hivekit/hivekit-go"
)

type pair struct {
	A float64 `json:"a"`
	B float64 `json:"b"`
}

func init() {
	hivekit.Define("addNumbers", func(in map[string]any) map[string]any {
		a, _ := in["a"].(float64)
		b, _ := in["b"].(float64)
		return map[string]any{"total": a + b}
	})

	hivekit.DefineJSON("multiply", func(p pair) (map[string]float64, error) {
		return map[string]float64{"result": p.A * p.B}, nil
	})

	hivekit.DefineJSON("divide", func(p pair) (map[string]float64, error) {
		if p.B == 0 {
			return nil, errors.New("division by zero")
		}
		return map[string]float64{"result": p.A / p.B}, nil
	})

	hivekit.Define("greetUser", func(in map[string]any) map[string]any {
		name, _ := in["name"].(string)
		if name == "" {
			name = "stranger"
		}
		return map[string]any{"message": "Hello, " + name + "!"}
	})
}

func main() {}
