"""
Running modules locally.

``run_with_ndsr`` builds the module and executes it with the NDSR binary (the
ground-truth runtime: real gas, receipts, storage, events, hive.call).
``run_local`` imports the source in this Python process with an in-memory
store — quick feedback for handler logic, without the node's semantics.
"""

from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, Optional

from .module import _registry, _decode, _encode


def find_ndsr(start: Optional[str] = None) -> Optional[str]:
    """``$NDSR_BIN``, ``ndsr`` on PATH, or ``tools/ndsr`` in an ancestor directory."""
    env = os.environ.get("NDSR_BIN")
    if env:
        return env if Path(env).is_file() else None
    found = shutil.which("ndsr")
    if found:
        return found
    for base in (Path(start or os.getcwd()), Path(__file__).resolve().parent):
        for d in [base.resolve(), *base.resolve().parents]:
            p = d / "tools" / "ndsr"
            if p.is_file():
                return str(p)
    return None


def run_with_ndsr(
    ndsr: str,
    hbc_path: str,
    function_name: str,
    input_text: str,
    gas: int = 1_000_000_000,
    data_dir: Optional[str] = None,
) -> Dict[str, Any]:
    """``ndsr run`` and return its JSON report (success, output, gas_used, error, receipt)."""
    cmd = [ndsr, "run", hbc_path, function_name, f"--input={input_text}", "--gas", str(gas)]
    if data_dir:
        cmd += ["--data-dir", data_dir]
    p = subprocess.run(cmd, capture_output=True, text=True)
    try:
        report = json.loads(p.stdout)
    except ValueError:
        raise RuntimeError(f"ndsr run failed (exit {p.returncode}): {p.stderr or p.stdout}") from None
    report["events"] = ((report.get("receipt") or {}).get("receipt") or {}).get("events", [])
    return report


def build_and_run(
    source_path: str,
    function_name: str,
    input_text: str,
    gas: int = 1_000_000_000,
    data_dir: Optional[str] = None,
    ndsr: Optional[str] = None,
) -> Dict[str, Any]:
    """Compile ``source_path`` (or use a ``.hbc``) and run it with NDSR."""
    from .compiler import compile_file

    ndsr = ndsr or find_ndsr()
    if ndsr is None:
        raise FileNotFoundError("ndsr binary not found (set NDSR_BIN or put ndsr on PATH)")
    if source_path.endswith(".hbc"):
        return run_with_ndsr(ndsr, source_path, function_name, input_text, gas, data_dir)
    with tempfile.TemporaryDirectory() as tmp:
        r = compile_file(source_path, output_dir=tmp)
        return run_with_ndsr(ndsr, r.hbc_path, function_name, input_text, gas, data_dir)


def run_local(source_path: str, function_name: str, input_data: Any) -> Any:
    """Import a module source in-process and invoke one function.

    Uses the module-level ``hive`` object the source imports; the registry is
    cleared first so each run is isolated."""
    import hivekit

    _registry.clear()
    hivekit.hive.__init__()
    path = Path(source_path).resolve()
    if not path.exists():
        raise FileNotFoundError(f"Source file not found: {source_path}")
    spec = importlib.util.spec_from_file_location("_hivekit_module", path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["_hivekit_module"] = mod
    spec.loader.exec_module(mod)
    if function_name not in _registry:
        raise KeyError(f"Function '{function_name}' not found. Available functions: {sorted(_registry)}")
    raw = input_data if isinstance(input_data, str) else _encode(input_data)
    return _decode(_registry[function_name](raw))
