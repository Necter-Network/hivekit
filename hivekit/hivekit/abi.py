"""Static check of a module against the hive-wasm-v1 guest ABI (HBC_SPEC §6.1, §6.4).
Mirror of hivekit-js/src/abi.ts; the node performs the authoritative validation."""

from __future__ import annotations

from typing import Dict, List, Tuple

I32, I64 = 0x7F, 0x7E
Sig = Tuple[Tuple[int, ...], Tuple[int, ...]]

ALLOWED_IMPORTS: Dict[str, Sig] = {
    "hive.call": ((I32,) * 6, (I64,)),
    "hive.emit": ((I32,) * 4, ()),
    "hive.abort": ((I32,) * 2, ()),
    "storage.get": ((I32,) * 2, (I64,)),
    "storage.set": ((I32,) * 4, ()),
    "storage.del": ((I32,) * 2, ()),
    "console.log": ((I32,) * 2, ()),
    "crypto.hash": ((I32,) * 2, (I64,)),
    "env.abort": ((I32,) * 4, ()),
}
REQUIRED_EXPORTS: Dict[str, Sig] = {
    "__alloc": ((I32,), (I32,)),
    "__hive_entry": ((I32,) * 3, (I64,)),
}


class AbiError(ValueError):
    pass


def validate_abi(wasm: bytes) -> Dict[str, List[str]]:
    """Raise AbiError if ``wasm`` violates the hive-wasm-v1 ABI."""
    if wasm[:4] != b"\0asm":
        raise AbiError("not a wasm module")
    p = 8

    def u32() -> int:
        nonlocal p
        r = s = 0
        while True:
            b = wasm[p]
            p += 1
            r |= (b & 0x7F) << s
            if not b & 0x80:
                return r
            s += 7

    def name() -> str:
        nonlocal p
        n = u32()
        v = wasm[p : p + n].decode("utf-8")
        p += n
        return v

    types: List[Sig] = []
    func_types: List[int] = []
    imports: List[str] = []
    exports: Dict[str, Tuple[int, int]] = {}
    has_memory = False
    while p < len(wasm):
        sid = wasm[p]
        p += 1
        size = u32()
        end = p + size
        if sid == 1:
            for _ in range(u32()):
                if wasm[p] != 0x60:
                    raise AbiError("unsupported type form")
                p += 1
                n = u32()
                params = tuple(wasm[p : p + n])
                p += n
                n = u32()
                results = tuple(wasm[p : p + n])
                p += n
                types.append((params, results))
        elif sid == 2:
            for _ in range(u32()):
                key = f"{name()}.{name()}"
                kind = wasm[p]
                p += 1
                if kind != 0:
                    raise AbiError(f"import {key}: only function imports are allowed")
                t = u32()
                if key not in ALLOWED_IMPORTS:
                    hint = " (WASI is not available on hive-wasm-v1)" if key.startswith("wasi") else ""
                    raise AbiError(f"import {key} is not part of hive-wasm-v1{hint}")
                if types[t] != ALLOWED_IMPORTS[key]:
                    raise AbiError(f"import {key} has the wrong signature")
                imports.append(key)
                func_types.append(t)
        elif sid == 3:
            for _ in range(u32()):
                func_types.append(u32())
        elif sid == 5:
            if u32() != 1:
                raise AbiError("module must define exactly one memory")
            if wasm[p] & ~1:
                raise AbiError("memory must be a non-shared 32-bit memory")
            has_memory = True
        elif sid == 7:
            for _ in range(u32()):
                nm = name()
                kind = wasm[p]
                p += 1
                exports[nm] = (kind, u32())
        p = end
    if exports.get("memory", (None,))[0] != 2 or not has_memory:
        raise AbiError('module must export its memory as "memory"')
    for nm, want in REQUIRED_EXPORTS.items():
        e = exports.get(nm)
        if e is None or e[0] != 0:
            raise AbiError(f"module must export function {nm}")
        if types[func_types[e[1]]] != want:
            raise AbiError(f"export {nm} has the wrong signature")
    return {"imports": imports, "exports": list(exports)}
