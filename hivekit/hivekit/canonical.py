"""Canonical JSON and Keccak-256 (docs/HBC_SPEC.md §4).

Canonical JSON: keys sorted by code point at every depth, no whitespace,
UTF-8 without ``\\u`` escaping of non-ASCII (``ensure_ascii=False``), integers
only with |n| <= 2**53 - 1. Floats are rejected.

Keccak-256 is the original Keccak (Ethereum), never FIPS ``hashlib.sha3_256``.
"""

from __future__ import annotations

import json
from typing import Any, Union

from Crypto.Hash import keccak as _keccak

MAX_SAFE_INTEGER = 2**53 - 1


def _check(value: Any, path: str) -> None:
    if value is None or isinstance(value, (bool, str)):
        return
    if isinstance(value, int):
        if abs(value) > MAX_SAFE_INTEGER:
            raise ValueError(f"canonical JSON: integer out of range at {path}")
        return
    if isinstance(value, float):
        raise ValueError(f"canonical JSON: float not allowed at {path}")
    if isinstance(value, (list, tuple)):
        for i, v in enumerate(value):
            _check(v, f"{path}[{i}]")
        return
    if isinstance(value, dict):
        for k, v in value.items():
            if not isinstance(k, str):
                raise ValueError(f"canonical JSON: non-string key at {path}")
            _check(v, f"{path}.{k}")
        return
    raise ValueError(f"canonical JSON: unsupported type {type(value).__name__} at {path}")


def canonical_json(value: Any) -> str:
    """Canonical JSON text of ``value``."""
    _check(value, "$")
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def canonical_bytes(value: Any) -> bytes:
    """Canonical JSON as UTF-8 bytes."""
    return canonical_json(value).encode("utf-8")


def keccak256(data: Union[bytes, bytearray, str]) -> str:
    """Keccak-256 of ``data`` (UTF-8 for ``str``) as ``0x`` + 64 lowercase hex."""
    if isinstance(data, str):
        data = data.encode("utf-8")
    h = _keccak.new(digest_bits=256)
    h.update(bytes(data))
    return "0x" + h.hexdigest()
