package main

import (
	"errors"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"

	"github.com/Necter-Network/hivekit/hivekit-go/hbc"
)

type multiFlag []string

func (m *multiFlag) String() string     { return fmt.Sprint(*m) }
func (m *multiFlag) Set(s string) error { *m = append(*m, s); return nil }

// findNDSR locates the reference runtime: -ndsr, $NDSR_BIN, `ndsr` on PATH,
// or tools/ndsr in the working directory or any parent (the necter checkout).
func findNDSR(flagPath string) string {
	for _, c := range []string{flagPath, os.Getenv("NDSR_BIN")} {
		if c != "" {
			return c
		}
	}
	if p, err := exec.LookPath("ndsr"); err == nil {
		return p
	}
	starts := []string{}
	if wd, err := os.Getwd(); err == nil {
		starts = append(starts, wd)
	}
	if exe, err := os.Executable(); err == nil {
		starts = append(starts, filepath.Dir(exe))
	}
	for _, dir := range starts {
		for {
			p := filepath.Join(dir, "tools", "ndsr")
			if fi, err := os.Stat(p); err == nil && !fi.IsDir() && fi.Mode()&0o111 != 0 {
				return p
			}
			parent := filepath.Dir(dir)
			if parent == dir {
				break
			}
			dir = parent
		}
	}
	return ""
}

func cmdRun(args []string) (int, error) {
	fs := flag.NewFlagSet("run", flag.ContinueOnError)
	inputFile := fs.String("input-file", "", "read input from file")
	gas := fs.Uint64("gas", 1_000_000, "gas limit")
	dataDir := fs.String("data-dir", "", "persist state here")
	ndsrFlag := fs.String("ndsr", "", "ndsr binary")
	local := fs.Bool("local", false, "use the built-in development runner")
	var modules multiFlag
	fs.Var(&modules, "module", "additional .hbc callable via hive.call (repeatable)")
	if err := parseInterspersed(fs, args); err != nil {
		return 0, err
	}
	if fs.NArg() < 2 || fs.NArg() > 3 {
		return 0, errors.New("usage: hivec run [flags] <file.hbc> <function> [input]")
	}
	hbcPath, fn := fs.Arg(0), fs.Arg(1)
	var input []byte
	switch {
	case *inputFile != "" && fs.NArg() == 3:
		return 0, errors.New("give either an input argument or -input-file, not both")
	case *inputFile != "":
		b, err := os.ReadFile(*inputFile)
		if err != nil {
			return 0, err
		}
		input = b
	case fs.NArg() == 3:
		input = []byte(fs.Arg(2))
	}

	load := func(p string) (*hbc.Artifact, []byte, error) {
		b, err := os.ReadFile(p)
		if err != nil {
			return nil, nil, err
		}
		a, err := hbc.Load(b)
		if err != nil {
			return nil, nil, fmt.Errorf("%s: %w", p, err)
		}
		return a, b, nil
	}
	art, _, err := load(hbcPath)
	if err != nil {
		return 0, err
	}
	var extra []*hbc.Artifact
	var extraBytes [][]byte
	for _, m := range modules {
		a, b, err := load(m)
		if err != nil {
			return 0, err
		}
		extra = append(extra, a)
		extraBytes = append(extraBytes, b)
	}

	ndsr := ""
	if !*local {
		ndsr = findNDSR(*ndsrFlag)
	}
	if ndsr == "" {
		if !*local {
			fmt.Fprintln(os.Stderr, "hivec: ndsr not found; using the built-in development runner (no gas, no receipt)")
		}
		return runLocal(art, fn, input, extra, *dataDir)
	}

	// ndsr resolves hive.call targets from <data-dir>/modules/<address>.hbc.
	dir := *dataDir
	if dir == "" && len(extra) > 0 {
		tmp, err := os.MkdirTemp("", "hivec-run-*")
		if err != nil {
			return 0, err
		}
		defer os.RemoveAll(tmp)
		dir = tmp
	}
	if len(extra) > 0 {
		mdir := filepath.Join(dir, "modules")
		if err := os.MkdirAll(mdir, 0o755); err != nil {
			return 0, err
		}
		for i, a := range extra {
			if err := os.WriteFile(filepath.Join(mdir, a.Address+".hbc"), extraBytes[i], 0o644); err != nil {
				return 0, err
			}
		}
	}
	nargs := []string{"run", hbcPath, fn, "--gas", strconv.FormatUint(*gas, 10)}
	if *inputFile != "" {
		nargs = append(nargs, "--input-file", *inputFile)
	} else {
		nargs = append(nargs, "--input="+string(input))
	}
	if dir != "" {
		nargs = append(nargs, "--data-dir", dir)
	}
	cmd := exec.Command(ndsr, nargs...)
	cmd.Stdout, cmd.Stderr = os.Stdout, os.Stderr
	if err := cmd.Run(); err != nil {
		var ee *exec.ExitError
		if errors.As(err, &ee) {
			return ee.ExitCode(), nil
		}
		return 0, err
	}
	return 0, nil
}
