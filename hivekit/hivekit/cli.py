"""
hivec — HiveKit Python compiler CLI

  hivec build <file.py> [-o dir]             compile to .hbc
  hivec inspect <file.hbc>                   validate and show the manifest
  hivec run <file.py|.hbc> <fn> [input]      execute (with ndsr when available)
  hivec functions <file.py>                  list exported functions
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import click

from .abi import validate_abi
from .compiler import CompileError, collect_hive_functions, compile_file
from .hbc import read_hbc
from .runner import build_and_run, find_ndsr, run_local


def _fail(e: Exception) -> None:
    click.echo(f"error: {e}", err=True)
    sys.exit(1)


@click.group()
@click.version_option(package_name="hivekit")
def main() -> None:
    """HiveKit compiler and developer tools for hive-wasm-v1 (NDSR)."""


@main.command()
@click.argument("source", type=click.Path(exists=True, dir_okay=False))
@click.option("--out", "-o", default="dist", show_default=True, help="Output directory")
@click.option("--name", "-n", default=None, help="Module name (default: file stem)")
@click.option("--module-version", default=None, help="Manifest version")
@click.option("--description", default=None, help="Manifest description")
def build(source, out, name, module_version, description):
    """Compile a Python module to a .hbc artifact."""
    try:
        r = compile_file(source, output_dir=out, module_name=name, version=module_version, description=description)
    except (CompileError, ValueError, FileNotFoundError) as e:
        _fail(e)
    click.echo(json.dumps({
        "hbc": r.hbc_path,
        "manifest_address": r.manifest_address,
        "functions": r.functions,
        "wasm_bytes": len(r.wasm_bytes),
        "compiler": r.manifest["compiler"],
    }, indent=2))


@main.command()
@click.argument("hbc_file", type=click.Path(exists=True, dir_okay=False))
def inspect(hbc_file):
    """Validate a .hbc (with `ndsr inspect` when available) and print its manifest."""
    ndsr = find_ndsr()
    if ndsr:
        sys.exit(subprocess.run([ndsr, "inspect", hbc_file]).returncode)
    try:
        manifest, wasm, addr = read_hbc(Path(hbc_file).read_bytes())
    except Exception as e:  # noqa: BLE001
        _fail(e)
    abi_error = None
    try:
        validate_abi(wasm)
    except ValueError as e:
        abi_error = str(e)
    click.echo(json.dumps({
        "manifest_address": addr,
        "manifest": manifest,
        "functions": [{"func_id": i, "name": n} for i, n in enumerate(manifest["functions"])],
        "wasm_bytes": len(wasm),
        "abi_valid": abi_error is None,
        "abi_error": abi_error,
    }, indent=2, ensure_ascii=False))


@main.command()
@click.argument("source", type=click.Path(exists=True, dir_okay=False))
@click.argument("function_name")
@click.argument("input_text", required=False, default="")
@click.option("--gas", default=1_000_000_000, show_default=True, help="Gas limit (ndsr)")
@click.option("--data-dir", default=None, help="Persist state between runs (ndsr)")
@click.option("--local", is_flag=True, help="Run in this Python process instead of ndsr")
def run(source, function_name, input_text, gas, data_dir, local):
    """Execute one function.

    Builds the module and runs it with the ndsr binary ($NDSR_BIN, PATH, or a
    tools/ndsr in a parent directory). Without ndsr, or with --local, the
    source is imported in this process with an in-memory store.

        hivec run counter.py increment '{"by": 2}'
    """
    ndsr = None if local else find_ndsr()
    if ndsr is None:
        if source.endswith(".hbc"):
            _fail(RuntimeError("running a .hbc needs the ndsr binary (set NDSR_BIN)"))
        try:
            result = run_local(source, function_name, input_text)
        except Exception as e:  # noqa: BLE001
            _fail(e)
        click.echo(json.dumps({"runner": "local", "success": True, "output": result}, indent=2, ensure_ascii=False))
        return
    try:
        report = build_and_run(source, function_name, input_text, gas=gas, data_dir=data_dir, ndsr=ndsr)
    except Exception as e:  # noqa: BLE001
        _fail(e)
    click.echo(json.dumps(dict(report, runner="ndsr"), indent=2, ensure_ascii=False))
    if not report.get("success"):
        sys.exit(2)


@main.command()
@click.argument("source", type=click.Path(exists=True, dir_okay=False))
def functions(source):
    """List exported functions (sorted; func_id = index)."""
    try:
        names = sorted(collect_hive_functions(Path(source).read_text(encoding="utf-8")))
    except CompileError as e:
        _fail(e)
    for i, n in enumerate(names):
        click.echo(f"{i}\t{n}")


if __name__ == "__main__":
    main()
