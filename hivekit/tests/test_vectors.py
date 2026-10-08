"""Known-answer vectors from docs/test-vectors.json (copied into fixtures/)."""
import base64
import hashlib

import pytest

from hivekit.canonical import canonical_json, keccak256
from hivekit.hbc import ManifestError, build_manifest, manifest_address, package_hbc, read_hbc, validate_manifest


def test_canonical_json(vectors):
    for v in vectors["canonical_json"]:
        assert canonical_json(v["input"]) == v["canonical"]


def test_keccak256_not_sha3(vectors):
    for v in vectors["keccak256"]:
        assert keccak256(v["input_utf8"]) == v["hash"]
    assert keccak256("") != "0x" + hashlib.sha3_256(b"").hexdigest()


def test_manifest_address(vectors):
    v = vectors["manifest_address"]
    assert canonical_json(v["manifest"]) == v["canonical_manifest"]
    wasm = bytes.fromhex(v["wasm_hex"])
    assert manifest_address(v["manifest"], wasm) == v["manifest_address"]
    assert manifest_address(dict(v["manifest"], manifest_address="0xdead"), wasm) == v["manifest_address"]


def test_read_vector_hbc(vectors):
    v = vectors["manifest_address"]
    manifest, wasm, addr = read_hbc(base64.b64decode(v["hbc_base64"]))
    assert addr == v["manifest_address"]
    assert wasm.hex() == v["wasm_hex"]
    assert manifest["manifest_address"] == addr


def test_packaging_reproduces_vector(vectors):
    v = vectors["manifest_address"]
    m = v["manifest"]
    built = build_manifest(m["name"], m["functions"], language=m["language"], compiler=m["compiler"], version=m["version"])
    p = package_hbc(built, bytes.fromhex(v["wasm_hex"]))
    assert p.manifest_address == v["manifest_address"]
    assert read_hbc(p.hbc)[2] == v["manifest_address"]


def test_events_and_receipt_vectors(vectors):
    r = vectors["execution_receipt"]
    assert canonical_json(r["events"]) == r["events_canonical_json"]
    rec = r["signed_envelope"]["receipt"]
    consensus = {k: rec[k] for k in ("v", "module_address", "function", "input_hash", "output_hash", "events_hash", "gas_used", "success")}
    assert canonical_json(consensus) == r["consensus_canonical_json"]
    assert keccak256(r["consensus_canonical_json"]) == r["receipt_hash"]


def test_canonical_json_rules():
    assert canonical_json({"s": "héllo ✓"}) == '{"s":"héllo ✓"}'  # ensure_ascii=False
    assert canonical_json({"b": {"z": 1, "a": [{"d": 1, "c": 2}]}, "a": 0}) == '{"a":0,"b":{"a":[{"c":2,"d":1}],"z":1}}'
    with pytest.raises(ValueError, match="float"):
        canonical_json({"x": 1.5})
    with pytest.raises(ValueError, match="range"):
        canonical_json([2**53])


BASE = {"name": "m", "language": "python", "compiler": "x/1", "runtime": "hive-wasm-v1", "functions": ["a", "b"]}


@pytest.mark.parametrize("key", ["created_at", "nrc1", "consensus", "wasm_ready", "schedules", "config"])
def test_manifest_rejects_extra_keys(key):
    with pytest.raises(ManifestError, match="not allowed"):
        validate_manifest(dict(BASE, **{key: 1}))


def test_manifest_rejects_unsorted_and_bad_runtime():
    with pytest.raises(ManifestError, match="sorted"):
        validate_manifest(dict(BASE, functions=["b", "a"]))
    with pytest.raises(ManifestError, match="runtime"):
        validate_manifest(dict(BASE, runtime="wasm32-wasi"))
    assert build_manifest("m", ["b", "B", "_a", "a"], compiler="c")["functions"] == ["B", "_a", "a", "b"]
