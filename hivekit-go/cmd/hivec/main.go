// hivec builds, inspects and runs HiveKit Go modules for NDSR (hive-wasm-v1).
//
//	hivec build [flags] <package dir | file.go>    tinygo build -target=wasm-unknown + package
//	hivec package [flags] <module.wasm>             package a pre-built wasm
//	hivec inspect <file.hbc>                        verify and print manifest + ABI
//	hivec functions <file.hbc | module.wasm | dir>  list functions with their ids
//	hivec run [flags] <file.hbc> <fn> [input]       execute (via tools/ndsr when found)
package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	hivekit "github.com/Necter-Network/hivekit/hivekit-go"
	"github.com/Necter-Network/hivekit/hivekit-go/hbc"
)

func main() {
	if len(os.Args) < 2 {
		usage()
		os.Exit(2)
	}
	var err error
	args := os.Args[2:]
	switch os.Args[1] {
	case "build":
		err = cmdBuild(args)
	case "package":
		err = cmdPackage(args)
	case "inspect":
		err = cmdInspect(args)
	case "functions":
		err = cmdFunctions(args)
	case "run":
		var code int
		code, err = cmdRun(args)
		if err == nil && code != 0 {
			os.Exit(code)
		}
	case "version", "--version", "-V":
		fmt.Println(compilerName(""))
	case "help", "--help", "-h":
		usage()
	default:
		fmt.Fprintf(os.Stderr, "hivec: unknown command %q\n\n", os.Args[1])
		usage()
		os.Exit(2)
	}
	if err != nil {
		fmt.Fprintf(os.Stderr, "hivec: %v\n", err)
		os.Exit(1)
	}
}

func usage() {
	fmt.Fprint(os.Stderr, `hivec — HiveKit Go compiler for NDSR (hive-wasm-v1)

Usage:
  hivec build [flags] <package dir | file.go>
      -o DIR            output directory (default "dist")
      -name NAME        module name (default: directory or file name)
      -version V        manifest version (optional)
      -description D    manifest description (optional)
      -tinygo PATH      tinygo binary (default: $TINYGO or "tinygo" on PATH)
      -gc GC            tinygo garbage collector (default: target default, "leaking")
      -keep-wasm        also write NAME.wasm next to the .hbc
  hivec package [flags] <module.wasm>
      -o, -name, -version, -description as above;
      -functions a,b    function list (default: read from the module's __hive_functions export)
      -language L       manifest language (default "go")
  hivec inspect <file.hbc>
  hivec functions <file.hbc | module.wasm | package dir>
  hivec run [flags] <file.hbc> <function> [input]
      -input-file F     read input from a file
      -gas N            gas limit (default 1000000)
      -data-dir DIR     persist state between runs (ndsr --data-dir)
      -module F.hbc     make another module callable via hive.call (repeatable)
      -ndsr PATH        ndsr binary (default: $NDSR_BIN, ndsr on PATH, or tools/ndsr found upward)
      -local            use the built-in development runner instead of ndsr
`)
}

// ── build / package ──────────────────────────────────────────────────────────

type manifestFlags struct {
	out, name, version, description string
}

func (m *manifestFlags) register(fs *flag.FlagSet) {
	fs.StringVar(&m.out, "o", "dist", "output directory")
	fs.StringVar(&m.name, "name", "", "module name")
	fs.StringVar(&m.version, "version", "", "manifest version")
	fs.StringVar(&m.description, "description", "", "manifest description")
}

func cmdBuild(args []string) error {
	fs := flag.NewFlagSet("build", flag.ContinueOnError)
	var mf manifestFlags
	mf.register(fs)
	tinygo := fs.String("tinygo", "", "tinygo binary")
	gc := fs.String("gc", "", "tinygo garbage collector")
	keep := fs.Bool("keep-wasm", false, "also write the .wasm")
	if err := parseInterspersed(fs, args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("usage: hivec build [flags] <package dir | file.go>")
	}
	src := fs.Arg(0)
	if mf.name == "" {
		mf.name = defaultName(src)
	}
	tg, err := findTinyGo(*tinygo)
	if err != nil {
		return err
	}
	tmp, err := os.MkdirTemp("", "hivec-*")
	if err != nil {
		return err
	}
	defer os.RemoveAll(tmp)
	wasmPath := filepath.Join(tmp, "module.wasm")
	tgVersion, err := tinygoBuild(tg, src, wasmPath, *gc)
	if err != nil {
		return err
	}
	wasm, err := os.ReadFile(wasmPath)
	if err != nil {
		return err
	}
	if _, err := packageWasm(wasm, mf, "go", compilerName(tgVersion), nil); err != nil {
		return err
	}
	if *keep {
		return os.WriteFile(filepath.Join(mf.out, mf.name+".wasm"), wasm, 0o644)
	}
	return nil
}

func cmdPackage(args []string) error {
	fs := flag.NewFlagSet("package", flag.ContinueOnError)
	var mf manifestFlags
	mf.register(fs)
	functions := fs.String("functions", "", "comma-separated function list")
	language := fs.String("language", "go", "manifest language")
	compiler := fs.String("compiler", compilerName(""), "manifest compiler")
	if err := parseInterspersed(fs, args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("usage: hivec package [flags] <module.wasm>")
	}
	wasm, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	if mf.name == "" {
		mf.name = defaultName(fs.Arg(0))
	}
	var fns []string
	if *functions != "" {
		for _, f := range strings.Split(*functions, ",") {
			if f = strings.TrimSpace(f); f != "" {
				fns = append(fns, f)
			}
		}
	}
	_, err = packageWasm(wasm, mf, *language, *compiler, fns)
	return err
}

func compilerName(tinygoVersion string) string {
	s := "hivec-go/" + hivekit.Version
	if tinygoVersion != "" {
		s += " tinygo/" + tinygoVersion
	}
	return s
}

func defaultName(src string) string {
	abs, err := filepath.Abs(src)
	if err != nil {
		abs = src
	}
	base := filepath.Base(abs)
	if ext := filepath.Ext(base); ext == ".go" || ext == ".wasm" {
		if ext == ".go" && (base == "main.go") {
			return filepath.Base(filepath.Dir(abs))
		}
		return strings.TrimSuffix(base, ext)
	}
	return base
}

// packageWasm validates the module ABI, determines the function list (from
// the module itself unless given), writes NAME.hbc and prints a summary.
func packageWasm(wasm []byte, mf manifestFlags, language, compiler string, fns []string) (*hbc.Artifact, error) {
	if err := hbc.CheckABI(wasm); err != nil {
		return nil, fmt.Errorf("module is not hive-wasm-v1: %w", err)
	}
	embedded, embErr := moduleFunctions(wasm)
	switch {
	case fns == nil && embErr != nil:
		return nil, fmt.Errorf("cannot read the function list from the module (%v); pass -functions", embErr)
	case fns == nil:
		fns = embedded
	default:
		sortUnique(&fns)
		if embErr == nil && strings.Join(fns, ",") != strings.Join(embedded, ",") {
			return nil, fmt.Errorf("-functions %v does not match the module's registered functions %v", fns, embedded)
		}
	}
	art, err := hbc.Build(hbc.Manifest{
		Name: mf.name, Language: language, Compiler: compiler, Runtime: hbc.Runtime,
		Functions: fns, Version: mf.version, Description: mf.description,
	}, wasm)
	if err != nil {
		return nil, err
	}
	data, err := art.Bytes()
	if err != nil {
		return nil, err
	}
	if err := os.MkdirAll(mf.out, 0o755); err != nil {
		return nil, err
	}
	out := filepath.Join(mf.out, mf.name+".hbc")
	if err := os.WriteFile(out, data, 0o644); err != nil {
		return nil, err
	}
	b, _ := json.MarshalIndent(map[string]any{
		"hbc":              out,
		"manifest_address": art.Address,
		"functions":        art.Manifest.Functions,
		"wasm_bytes":       len(wasm),
		"compiler":         art.Manifest.Compiler,
	}, "", "  ")
	fmt.Fprintln(summaryOut, string(b))
	return art, nil
}

func sortUnique(s *[]string) {
	m := map[string]bool{}
	var out []string
	for _, x := range *s {
		if !m[x] {
			m[x] = true
			out = append(out, x)
		}
	}
	sortStrings(out)
	*s = out
}

// ── inspect / functions ──────────────────────────────────────────────────────

func cmdInspect(args []string) error {
	if len(args) != 1 {
		return errors.New("usage: hivec inspect <file.hbc>")
	}
	data, err := os.ReadFile(args[0])
	if err != nil {
		return err
	}
	art, err := hbc.Load(data)
	if err != nil {
		return err
	}
	abiErr := hbc.CheckABI(art.Wasm)
	wi, _ := hbc.ParseInterface(art.Wasm)
	var imports []string
	if wi != nil {
		for _, im := range wi.Imports {
			imports = append(imports, im.Module+"."+im.Name)
		}
	}
	res := map[string]any{
		"manifest":         art.Manifest.Object(true),
		"manifest_address": art.Address,
		"wasm_bytes":       len(art.Wasm),
		"imports":          imports,
		"abi_ok":           abiErr == nil,
	}
	if abiErr != nil {
		res["abi_error"] = abiErr.Error()
	}
	printJSON(res)
	if abiErr != nil {
		return abiErr
	}
	return nil
}

func cmdFunctions(args []string) error {
	if len(args) != 1 {
		return errors.New("usage: hivec functions <file.hbc | module.wasm | package dir | file.go>")
	}
	src := args[0]
	if fi, err := os.Stat(src); err == nil && (fi.IsDir() || strings.HasSuffix(src, ".go")) {
		tg, err := findTinyGo("")
		if err != nil {
			return err
		}
		tmp, err := os.MkdirTemp("", "hivec-*")
		if err != nil {
			return err
		}
		defer os.RemoveAll(tmp)
		src = filepath.Join(tmp, "module.wasm")
		if _, err := tinygoBuild(tg, args[0], src, ""); err != nil {
			return err
		}
	}
	data, err := os.ReadFile(src)
	if err != nil {
		return err
	}
	var fns []string
	if strings.HasSuffix(src, ".hbc") {
		art, err := hbc.Load(data)
		if err != nil {
			return err
		}
		fns = art.Manifest.Functions
	} else if fns, err = moduleFunctions(data); err != nil {
		return err
	}
	for i, f := range fns {
		fmt.Printf("%d\t%s\n", i, f)
	}
	return nil
}

// ── helpers ──────────────────────────────────────────────────────────────────

// summaryOut receives build summaries (tests silence it).
var summaryOut io.Writer = os.Stdout

func printJSON(v any) {
	b, _ := json.MarshalIndent(v, "", "  ")
	fmt.Println(string(b))
}

func sortStrings(s []string) {
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j] < s[j-1]; j-- {
			s[j], s[j-1] = s[j-1], s[j]
		}
	}
}

// parseInterspersed lets flags follow positional arguments.
func parseInterspersed(fs *flag.FlagSet, args []string) error {
	var pos []string
	for {
		if err := fs.Parse(args); err != nil {
			return err
		}
		if fs.NArg() == 0 {
			break
		}
		pos = append(pos, fs.Arg(0))
		args = fs.Args()[1:]
	}
	return fs.Parse(pos)
}
