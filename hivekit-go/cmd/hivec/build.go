package main

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

func findTinyGo(flagPath string) (string, error) {
	for _, c := range []string{flagPath, os.Getenv("TINYGO")} {
		if c != "" {
			return c, nil
		}
	}
	p, err := exec.LookPath("tinygo")
	if err != nil {
		return "", errors.New("tinygo not found: install TinyGo (https://tinygo.org) or pass -tinygo / set $TINYGO")
	}
	return p, nil
}

// tinygoBuild compiles src for the bare wasm-unknown target (no WASI imports)
// without debug info (DWARF embeds local paths and would change the address).
// It returns the TinyGo version.
func tinygoBuild(tinygo, src, out, gc string) (string, error) {
	ver, err := exec.Command(tinygo, "version").Output()
	if err != nil {
		return "", fmt.Errorf("running %s version: %w", tinygo, err)
	}
	version := ""
	if f := strings.Fields(string(ver)); len(f) >= 3 {
		version = f[2]
	}
	args := []string{"build", "-target=wasm-unknown", "-no-debug", "-o", out}
	if gc != "" {
		args = append(args, "-gc="+gc)
	}
	args = append(args, src)
	cmd := exec.Command(tinygo, args...)
	var stderr bytes.Buffer
	cmd.Stdout = os.Stderr
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return "", fmt.Errorf("tinygo build failed: %v\n%s", err, stderr.String())
	}
	return version, nil
}
