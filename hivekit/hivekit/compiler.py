"""
HiveKit Python compiler: a module source file -> spec-conformant ``.hbc``.

The module runs in the HiveKit Python runtime: RustPython compiled to
``wasm32-unknown-unknown`` (``runtime/hivekit-py-runtime.wasm``, built
reproducibly by ``runtime-py/build.sh``). It imports only the hive-wasm-v1
host functions; there is no WASI. The source is embedded into a copy of the
runtime (see :mod:`hivekit.embed`), so building needs no Rust toolchain.

Exported functions are the names passed to ``@hive.define("name")`` /
``hive.define("name", fn)`` (string literals), plus ``__consensus`` for
``@hive.consensus(...)``. They are sorted; ``func_id`` = index.
"""

from __future__ import annotations

import ast
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional

from .abi import validate_abi
from .embed import embed_script, script_blob
from .hbc import MAX_WASM_BYTES, build_manifest, package_hbc, sort_functions
from .version import __version__

RUNTIME_ENGINE = "rustpython@0.6.0"
RUNTIME_WASM = Path(__file__).parent / "runtime" / "hivekit-py-runtime.wasm"
MAX_SOURCE_BYTES = 1024 * 1024


def compiler_id() -> str:
    return f"hivekit-py/{__version__}+{RUNTIME_ENGINE}"


class CompileError(ValueError):
    pass


@dataclass
class CompileResult:
    name: str
    manifest_address: str
    functions: List[str]
    hbc_path: Optional[str]
    manifest: Dict[str, Any]
    hbc_bytes: bytes = b""
    wasm_bytes: bytes = b""
    is_wasm: bool = True
    extra: Dict[str, Any] = field(default_factory=dict)


# ── Function discovery ────────────────────────────────────────────────────────


def _is_hive_attr(node: ast.AST, attr: str) -> bool:
    return isinstance(node, ast.Attribute) and node.attr == attr and isinstance(node.value, ast.Name) and node.value.id == "hive"


def collect_hive_functions(source: str) -> List[str]:
    """Exported function names in order of appearance.

    Raises CompileError for a non-literal name or a syntax error."""
    try:
        tree = ast.parse(source)
    except SyntaxError as e:
        raise CompileError(f"syntax error at line {e.lineno}: {e.msg}") from None
    found = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Call) and _is_hive_attr(node.func, "define"):
            if not node.args or not isinstance(node.args[0], ast.Constant) or not isinstance(node.args[0].value, str):
                raise CompileError(f"line {node.lineno}: hive.define(...) must take a string literal name as its first argument")
            found.append((node.lineno, node.col_offset, node.args[0].value))
        elif isinstance(node, ast.Call) and _is_hive_attr(node.func, "consensus"):
            if not any(n == "__consensus" for _, _, n in found):
                found.append((node.lineno, node.col_offset, "__consensus"))
    return [n for _, _, n in sorted(found)]


def is_consensus_module(source: str) -> bool:
    """True if the source uses ``@hive.consensus(...)``."""
    return bool(re.search(r"@hive\.consensus\s*\(", source))


def collect_schedules(source: str) -> List[dict]:
    """``hive.schedule("name", "cron", fn)`` registrations (informational only:
    schedules are not part of the v1 manifest)."""
    return [
        {"name": m.group(1), "cron": m.group(2)}
        for m in re.finditer(r'hive\.schedule\s*\(\s*["\']([^"\']+)["\']\s*,\s*["\']([^"\']+)["\']', source)
    ]


def extract_hive_config(source: str) -> Optional[dict]:
    """Best-effort static read of ``hive.config({...})`` literals (informational)."""
    match = re.search(r"hive\.config\s*\(\s*\{([^}]+)\}", source, re.DOTALL)
    if not match:
        return None
    body = match.group(1)
    cfg: dict = {}
    for f in ("name", "version", "memory", "storage"):
        m = re.search(rf'["\']?{f}["\']?\s*:\s*["\']([^"\']+)["\']', body)
        if m:
            cfg[f] = m.group(1)
    tm = re.search(r'["\']?tags["\']?\s*:\s*\[([^\]]+)\]', body)
    if tm:
        cfg["tags"] = [t.strip().strip("'\"") for t in tm.group(1).split(",") if t.strip().strip("'\"")]
    return cfg or None


def extract_nrc1_config(source: str) -> Optional[dict]:
    """Best-effort static read of ``NRC1Config(...)`` literals (informational;
    reward configuration is not part of the v1 manifest)."""
    match = re.search(r"NRC1Config\s*\(([^)]+)\)", source, re.DOTALL)
    if not match:
        return None
    body = match.group(1)
    config: dict = {}
    for f in ("reward_token", "reward_chain", "reward_per_unit", "token_standard", "display_name", "description", "icon_url"):
        m = re.search(rf'{f}\s*=\s*["\']([^"\']+)["\']', body)
        if m:
            config[f] = m.group(1)
    for f in ("units_per_execution", "min_units", "max_units"):
        m = re.search(rf"{f}\s*=\s*(\d+)", body)
        if m:
            config[f] = int(m.group(1))
    return config or None


# ── Compilation ───────────────────────────────────────────────────────────────


def load_runtime() -> bytes:
    if not RUNTIME_WASM.exists():
        raise CompileError(f"HiveKit Python runtime not found at {RUNTIME_WASM}; rebuild it with runtime-py/build.sh")
    return RUNTIME_WASM.read_bytes()


def compile_source(
    source: str,
    name: str,
    version: Optional[str] = None,
    description: Optional[str] = None,
    runtime: Optional[bytes] = None,
) -> CompileResult:
    """Compile module source text to ``.hbc`` bytes (nothing is written)."""
    if len(source.encode("utf-8")) > MAX_SOURCE_BYTES:
        raise CompileError("source exceeds 1 MiB")
    names = collect_hive_functions(source)
    if not names:
        raise CompileError('no @hive.define("name") functions found')
    try:
        functions = sort_functions(names)
    except ValueError as e:
        raise CompileError(str(e)) from None
    wasm = embed_script(runtime if runtime is not None else load_runtime(), script_blob(functions, source))
    if len(wasm) > MAX_WASM_BYTES:
        raise CompileError(f"module.wasm would be {len(wasm)} bytes (limit {MAX_WASM_BYTES})")
    validate_abi(wasm)
    manifest = build_manifest(name, functions, language="python", compiler=compiler_id(), version=version, description=description)
    p = package_hbc(manifest, wasm)
    return CompileResult(
        name=name,
        manifest_address=p.manifest_address,
        functions=functions,
        hbc_path=None,
        manifest=p.manifest,
        hbc_bytes=p.hbc,
        wasm_bytes=wasm,
    )


def compile_file(
    source_path: str,
    output_dir: str = "dist",
    module_name: Optional[str] = None,
    version: Optional[str] = None,
    description: Optional[str] = None,
) -> CompileResult:
    """Compile a module file and write ``<output_dir>/<name>.hbc``."""
    path = Path(source_path)
    if not path.exists():
        raise FileNotFoundError(f"Source file not found: {source_path}")
    name = module_name or path.stem
    r = compile_source(path.read_text(encoding="utf-8"), name, version=version, description=description)
    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)
    hbc_path = out / f"{name}.hbc"
    hbc_path.write_bytes(r.hbc_bytes)
    (out / f"{name}.manifest.json").write_text(json.dumps(r.manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    r.hbc_path = str(hbc_path)
    return r
