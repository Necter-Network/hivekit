"""``.hbc`` artifacts (HBC_SPEC.md §2–§5): manifest schema, content address,
deterministic ZIP packaging and reading."""

from __future__ import annotations

import io
import json
import re
import zipfile
from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Tuple

from .canonical import canonical_bytes, keccak256

RUNTIME = "hive-wasm-v1"
MAX_WASM_BYTES = 12 * 1024 * 1024  # HBC_SPEC §2 (raised from 8 MiB)
MAX_MANIFEST_BYTES = 64 * 1024
MAX_HBC_BYTES = 16 * 1024 * 1024

ALLOWED_KEYS = {"name", "language", "compiler", "runtime", "functions", "version", "description", "manifest_address"}
_FN_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]{0,63}$")
_LANG_RE = re.compile(r"^[a-z0-9_+-]{1,32}$")
_ADDR_RE = re.compile(r"^0x[0-9a-f]{64}$")


class ManifestError(ValueError):
    pass


def is_function_name(name: str) -> bool:
    return isinstance(name, str) and bool(_FN_RE.match(name))


def sort_functions(names: List[str]) -> List[str]:
    """Sort by byte value, rejecting duplicates and invalid names."""
    out = sorted(names, key=lambda s: s.encode("utf-8"))
    for a, b in zip(out, out[1:]):
        if a == b:
            raise ManifestError(f"function {a!r} is defined more than once")
    for n in out:
        if not is_function_name(n):
            raise ManifestError(f"invalid function name {n!r} (must match [A-Za-z_][A-Za-z0-9_]{{0,63}})")
    return out


def validate_manifest(m: Dict[str, Any]) -> None:
    """Raise ManifestError if ``m`` violates HBC_SPEC §3."""
    if not isinstance(m, dict):
        raise ManifestError("manifest must be an object")
    for k in m:
        if k not in ALLOWED_KEYS:
            raise ManifestError(f"manifest: key {k!r} is not allowed")

    def text(key: str, lo: int, hi: int, required: bool) -> None:
        if key not in m:
            if required:
                raise ManifestError(f"manifest: {key!r} is required")
            return
        v = m[key]
        if not isinstance(v, str):
            raise ManifestError(f"manifest: {key!r} must be a string")
        n = len(v.encode("utf-8"))
        if not lo <= n <= hi:
            raise ManifestError(f"manifest: {key!r} must be {lo}-{hi} bytes")

    text("name", 1, 128, True)
    text("language", 1, 32, True)
    text("compiler", 1, 128, True)
    text("version", 0, 1024, False)
    text("description", 0, 1024, False)
    if not _LANG_RE.match(m["language"]):
        raise ManifestError("manifest: 'language' must match [a-z0-9_+-]{1,32}")
    if m.get("runtime") != RUNTIME:
        raise ManifestError(f"manifest: 'runtime' must be {RUNTIME!r}")
    fns = m.get("functions")
    if not isinstance(fns, list) or not 1 <= len(fns) <= 256:
        raise ManifestError("manifest: 'functions' must have 1-256 entries")
    for i, f in enumerate(fns):
        if not is_function_name(f):
            raise ManifestError(f"manifest: invalid function name {f!r}")
        if i and fns[i - 1].encode() >= f.encode():
            raise ManifestError("manifest: 'functions' must be sorted ascending and unique")
    if "manifest_address" in m and not isinstance(m["manifest_address"], str):
        raise ManifestError("manifest: 'manifest_address' must be a string")


def build_manifest(
    name: str,
    functions: List[str],
    language: str = "python",
    compiler: Optional[str] = None,
    version: Optional[str] = None,
    description: Optional[str] = None,
) -> Dict[str, Any]:
    """A validated manifest (without ``manifest_address``)."""
    if compiler is None:
        from .compiler import compiler_id

        compiler = compiler_id()
    m: Dict[str, Any] = {
        "name": name,
        "language": language,
        "compiler": compiler,
        "runtime": RUNTIME,
        "functions": sort_functions(list(functions)),
    }
    if version is not None:
        m["version"] = version
    if description is not None:
        m["description"] = description
    validate_manifest(m)
    return m


def manifest_address(manifest: Dict[str, Any], wasm: bytes) -> str:
    """keccak256(canonical_json(manifest - manifest_address) || wasm)."""
    rest = {k: v for k, v in manifest.items() if k != "manifest_address"}
    return keccak256(canonical_bytes(rest) + bytes(wasm))


def normalize_address(address: str) -> str:
    a = address.strip()
    if a.lower().startswith("hive:"):
        a = a[5:]
    a = a.lower()
    if not _ADDR_RE.match(a):
        raise ValueError(f"invalid module address: {address!r}")
    return a


@dataclass
class Packaged:
    hbc: bytes
    manifest: Dict[str, Any]
    manifest_address: str


def _zip_stored(entries: List[Tuple[str, bytes]]) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_STORED) as zf:
        for name, data in entries:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            info.external_attr = 0o100644 << 16
            info.create_system = 3
            zf.writestr(info, data)
    return buf.getvalue()


def package_hbc(manifest: Dict[str, Any], wasm: bytes) -> Packaged:
    """Validate, address and package ``manifest.json`` + ``module.wasm``."""
    if len(wasm) > MAX_WASM_BYTES:
        raise ManifestError(f"module.wasm is {len(wasm)} bytes; the hive-wasm-v1 limit is {MAX_WASM_BYTES}")
    base = {k: v for k, v in manifest.items() if k != "manifest_address"}
    validate_manifest(base)
    addr = manifest_address(base, wasm)
    full = dict(base, manifest_address=addr)
    mbytes = canonical_bytes(full)
    if len(mbytes) > MAX_MANIFEST_BYTES:
        raise ManifestError("manifest.json exceeds 64 KiB")
    return Packaged(_zip_stored([("manifest.json", mbytes), ("module.wasm", bytes(wasm))]), full, addr)


def read_hbc(data: bytes) -> Tuple[Dict[str, Any], bytes, str]:
    """Read a ``.hbc``: enforce container rules, verify the address.
    Returns (manifest, wasm, manifest_address)."""
    if len(data) > MAX_HBC_BYTES:
        raise ManifestError(".hbc exceeds 16 MiB")
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        names = [i.filename for i in zf.infolist()]
        if len(set(names)) != len(names):
            raise ManifestError("duplicate ZIP entries")
        extra = set(names) - {"manifest.json", "module.wasm"}
        if extra:
            raise ManifestError(f"unexpected entries {sorted(extra)} (only manifest.json and module.wasm are allowed)")
        if set(names) != {"manifest.json", "module.wasm"}:
            raise ManifestError(".hbc must contain manifest.json and module.wasm")
        for i in zf.infolist():
            limit = MAX_MANIFEST_BYTES if i.filename == "manifest.json" else MAX_WASM_BYTES
            if i.file_size > limit:
                raise ManifestError(f"{i.filename} exceeds its size limit")
        mbytes = zf.read("manifest.json")
        wasm = zf.read("module.wasm")
    manifest = json.loads(mbytes.decode("utf-8"))
    validate_manifest(manifest)
    addr = manifest_address(manifest, wasm)
    declared = manifest.get("manifest_address")
    if declared is not None and normalize_address(declared) != addr:
        raise ManifestError(f"manifest_address mismatch: declared {declared}, computed {addr}")
    return manifest, wasm, addr
