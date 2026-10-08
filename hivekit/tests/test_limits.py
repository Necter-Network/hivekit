"""Size limits from HBC_SPEC §2: module.wasm <= 12 MiB, .hbc <= 16 MiB."""
import pytest

from hivekit.canonical import canonical_bytes
from hivekit.hbc import MAX_HBC_BYTES, MAX_WASM_BYTES, ManifestError, _zip_stored, build_manifest, package_hbc, read_hbc


def pad_to(wasm: bytes, total: int) -> bytes:
    """``wasm`` plus one custom section so the result is exactly ``total`` bytes."""
    size = total - len(wasm) - 5  # section id + 4-byte LEB size
    leb = bytes([(size & 0x7F) | 0x80, ((size >> 7) & 0x7F) | 0x80, ((size >> 14) & 0x7F) | 0x80, size >> 21])
    out = wasm + b"\x00" + leb + b"\x03pad"
    return out + bytes(total - len(out))


@pytest.fixture
def base(vectors):
    return bytes.fromhex(vectors["manifest_address"]["wasm_hex"])


MANIFEST = build_manifest(name="big", language="wat", compiler="limits-test/1", functions=["echo"])


def test_limits_match_spec():
    assert MAX_WASM_BYTES == 12 * 1024 * 1024
    assert MAX_HBC_BYTES == 16 * 1024 * 1024


def test_module_wasm_at_limit_accepted(base):
    wasm = pad_to(base, MAX_WASM_BYTES)
    p = package_hbc(MANIFEST, wasm)
    assert len(p.hbc) <= MAX_HBC_BYTES
    _, got, addr = read_hbc(p.hbc)
    assert addr == p.manifest_address
    assert got == wasm


def test_module_wasm_over_limit_rejected(base):
    wasm = pad_to(base, MAX_WASM_BYTES + 1)
    with pytest.raises(ManifestError):
        package_hbc(MANIFEST, wasm)
    hbc = _zip_stored([("manifest.json", canonical_bytes(MANIFEST)), ("module.wasm", wasm)])
    with pytest.raises(ManifestError, match="module.wasm exceeds"):
        read_hbc(hbc)
