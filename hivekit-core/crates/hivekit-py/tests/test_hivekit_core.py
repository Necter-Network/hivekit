"""Smoke test of the `hivekit_core` extension against NDSR's known-answer vectors.

    maturin develop -m hivekit-core/crates/hivekit-py/Cargo.toml
    python hivekit-core/crates/hivekit-py/tests/test_hivekit_core.py
"""

import base64
import json
import pathlib

import hivekit_core as hk

VECTORS = json.loads(
    (pathlib.Path(__file__).resolve().parents[2] / "hivekit-core/tests/data/test-vectors.json").read_text()
)


def test_vectors():
    ma = VECTORS["manifest_address"]
    wasm = bytes.fromhex(ma["wasm_hex"])
    m = ma["manifest"]
    r = hk.package_wasm(
        wasm,
        name=m["name"],
        language=m["language"],
        functions=m["functions"],
        compiler=m["compiler"],
        version=m["version"],
    )
    assert r.manifest_address == ma["manifest_address"]
    assert r.hbc_bytes == base64.b64decode(ma["hbc_base64"])
    assert r.functions == ["echo"]
    assert hk.compute_address(ma["canonical_manifest"], wasm) == ma["manifest_address"]

    info = hk.inspect(r.hbc_bytes)
    assert info["manifest_address"] == ma["manifest_address"]
    assert info["executable"] is True
    assert info["imports"] == ["hive.emit"]
    assert info["manifest"]["runtime"] == hk.RUNTIME_ID == "hive-wasm-v1"

    for c in VECTORS["canonical_json"]:
        assert hk.canonical_json(json.dumps(c["input"])) == c["canonical"]
    for c in VECTORS["keccak256"]:
        assert hk.keccak256_hex(c["input_utf8"].encode()) == c["hash"]


def test_errors():
    for bad in [b"nope", b""]:
        try:
            hk.inspect(bad)
        except ValueError:
            pass
        else:
            raise AssertionError("inspect accepted garbage")
    try:
        hk.compile("#[hive_export] fn f() {}", b"", "m")
    except ValueError as e:
        assert "module.wasm" in str(e)
    else:
        raise AssertionError("compile packaged source text")
    try:
        hk.compute_address('{"name":"n","created_at":"x"}', b"\0asm\x01\0\0\0")
    except ValueError as e:
        assert "created_at" in str(e)
    else:
        raise AssertionError("unknown manifest key accepted")


def test_detect_and_normalize():
    src = '#[hive_export]\nfn add_numbers(i: Value) -> Value { i }\n#[hive_export("greet")]\nfn g(i: Value) -> Value { i }'
    assert hk.detect_functions(src, "rust") == ["addNumbers", "greet"]
    assert hk.normalize_address("hive:0X" + "AB" * 32) == "0x" + "ab" * 32
    assert hk.normalize_address("0x12") is None


if __name__ == "__main__":
    for name, fn in list(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print("ok", name)
