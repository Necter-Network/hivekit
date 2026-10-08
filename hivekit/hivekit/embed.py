"""Embed a script into a prebuilt HiveKit interpreter runtime without recompiling it.

Mirror of hivekit-js/src/embed.ts (both produce identical bytes):
the script blob is appended as an active data segment at the end of the
module's initial memory (page aligned), the memory minimum is raised to
cover it, the runtime's ScriptRef (16-byte magic + u32 ptr + u32 len) is
patched in place, and the DataCount section is bumped if present. The
runtime's allocator only grows memory with ``memory.grow``, so these pages
are never handed out.
"""

from __future__ import annotations

import struct
from typing import List, Tuple

SCRIPT_MAGIC = b"HIVEKIT-SCRIPT-1"
PAGE = 65536


def _uleb(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def _sleb(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if (n == 0 and not b & 0x40) or (n == -1 and b & 0x40):
            out.append(b)
            return bytes(out)
        out.append(b | 0x80)


def _read_uleb(b: bytes, p: int) -> Tuple[int, int]:
    result = shift = 0
    while True:
        x = b[p]
        p += 1
        result |= (x & 0x7F) << shift
        if not x & 0x80:
            return result, p
        shift += 7


def _read_sleb32(b: bytes, p: int) -> Tuple[int, int]:
    result = shift = 0
    while True:
        x = b[p]
        p += 1
        result |= (x & 0x7F) << shift
        shift += 7
        if not x & 0x80:
            break
    if shift < 32 and x & 0x40:
        result |= -1 << shift
    return result & 0xFFFFFFFF, p


def script_blob(functions: List[str], source: str) -> bytes:
    """Sorted function names joined by ``\\n``, a NUL, then the UTF-8 source."""
    return "\n".join(functions).encode("utf-8") + b"\0" + source.encode("utf-8")


def embed_script(runtime: bytes, blob: bytes) -> bytes:
    """Return a copy of ``runtime`` with ``blob`` embedded."""
    if runtime[:4] != b"\0asm":
        raise ValueError("runtime is not a wasm module")
    sections: List[List] = []
    p = 8
    while p < len(runtime):
        sid = runtime[p]
        size, p = _read_uleb(runtime, p + 1)
        sections.append([sid, bytearray(runtime[p : p + size])])
        p += size

    mem = next((s for s in sections if s[0] == 5), None)
    data = next((s for s in sections if s[0] == 11), None)
    if mem is None or data is None:
        raise ValueError("runtime has no memory or data section")

    mb = bytes(mem[1])
    count_mem, q = _read_uleb(mb, 0)
    if count_mem != 1:
        raise ValueError("runtime must define exactly one memory")
    flags = mb[q]
    q += 1
    if flags not in (0, 1):
        raise ValueError("runtime memory must be a plain 32-bit memory")
    min_pages, q = _read_uleb(mb, q)
    max_pages = _read_uleb(mb, q)[0] if flags == 1 else None

    db = bytes(data[1])
    count, q = _read_uleb(db, 0)
    count_len = q
    data_end = 0
    patch_at = -1
    for _ in range(count):
        kind, q = _read_uleb(db, q)
        offset = -1
        if kind in (0, 2):
            if kind == 2:
                _, q = _read_uleb(db, q)
            if db[q] != 0x41:
                raise ValueError("runtime data segment offset must be i32.const")
            offset, q = _read_sleb32(db, q + 1)
            if db[q] != 0x0B:
                raise ValueError("runtime data segment offset must be a constant expression")
            q += 1
        elif kind != 1:
            raise ValueError(f"unsupported data segment kind {kind}")
        length, q = _read_uleb(db, q)
        start = q
        q += length
        if offset >= 0:
            data_end = max(data_end, offset + length)
            seg = db[start : start + length]
            at = seg.find(SCRIPT_MAGIC)
            if at >= 0:
                if patch_at >= 0 or seg.find(SCRIPT_MAGIC, at + 1) >= 0:
                    raise ValueError("runtime contains ScriptRef more than once")
                patch_at = start + at + len(SCRIPT_MAGIC)
    if patch_at < 0:
        raise ValueError("runtime has no ScriptRef (not a HiveKit interpreter runtime)")
    if db[patch_at : patch_at + 8] != b"\xff" * 8:
        raise ValueError("runtime already has a script embedded")

    place = max(min_pages * PAGE, -(-data_end // PAGE) * PAGE)
    new_min = place // PAGE + -(-max(len(blob), 1) // PAGE)
    if max_pages is not None and new_min > max_pages:
        raise ValueError("script does not fit in the runtime memory limit")
    if new_min > 1024:
        raise ValueError("script too large: initial memory would exceed 64 MiB")

    body = bytearray(_uleb(count + 1) + db[count_len:])
    shift = len(_uleb(count + 1)) - count_len
    struct.pack_into("<II", body, patch_at + shift, place, len(blob))
    signed_place = place - (1 << 32) if place >= 1 << 31 else place
    body += b"\x00\x41" + _sleb(signed_place) + b"\x0b" + _uleb(len(blob)) + blob
    data[1] = body

    mem[1] = bytearray(bytes([1, flags]) + _uleb(new_min) + (_uleb(max_pages) if max_pages is not None else b""))
    for s in sections:
        if s[0] == 12:
            s[1] = bytearray(_uleb(count + 1))

    out = bytearray(runtime[:8])
    for sid, b in sections:
        out += bytes([sid]) + _uleb(len(b)) + bytes(b)
    return bytes(out)
