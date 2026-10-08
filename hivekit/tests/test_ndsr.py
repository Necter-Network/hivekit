"""End-to-end: modules built by the Python SDK pass `ndsr inspect` and run under `ndsr run`."""
import json

import pytest

from hivekit.abi import validate_abi
from hivekit.canonical import keccak256
from hivekit.compiler import compile_file, compile_source, load_runtime
from hivekit.embed import SCRIPT_MAGIC, embed_script, script_blob
from hivekit.hbc import MAX_WASM_BYTES, read_hbc

from conftest import EXAMPLES, install, ndsr_inspect, ndsr_run, needs_ndsr


@pytest.fixture(scope="module")
def counter(tmp_path_factory):
    out = tmp_path_factory.mktemp("build")
    return compile_file(str(EXAMPLES / "counter.py"), output_dir=str(out))


def test_runtime_is_import_clean_and_fits():
    rt = load_runtime()
    report = validate_abi(rt)
    assert not any(i.startswith("wasi") for i in report["imports"])
    # Leave room for embedded source (up to 1 MiB) under the module.wasm limit.
    assert len(rt) + 1024 * 1024 < MAX_WASM_BYTES


def test_runtime_is_pre_initialized():
    # The build-time snapshot removes the pre-initialization hook from the exports.
    exports = validate_abi(load_runtime())["exports"]
    assert "__hive_preinit" not in exports
    assert {"memory", "__alloc", "__hive_entry"} <= set(exports)


def test_embedding_is_deterministic():
    rt = load_runtime()
    blob = script_blob(["a"], "from hivekit import hive\n")
    a, b = embed_script(rt, blob), embed_script(rt, blob)
    assert a == b
    at = a.find(SCRIPT_MAGIC)
    ptr, length = int.from_bytes(a[at + 16:at + 20], "little"), int.from_bytes(a[at + 20:at + 24], "little")
    assert length == len(blob) and ptr % 65536 == 0
    assert a.endswith(blob)
    with pytest.raises(ValueError, match="already"):
        embed_script(a, blob)


def test_artifact_is_conformant(counter):
    manifest, wasm, addr = read_hbc(open(counter.hbc_path, "rb").read())
    assert addr == counter.manifest_address
    assert manifest == {
        "compiler": "hivekit-py/1.0.0+rustpython@0.6.0",
        "functions": ["get", "increment", "note", "relay"],
        "language": "python",
        "manifest_address": addr,
        "name": "counter",
        "runtime": "hive-wasm-v1",
    }
    validate_abi(wasm)
    again = compile_file(str(EXAMPLES / "counter.py"), output_dir=str(EXAMPLES.parent / ".pytest_cache" / "again"))
    assert again.hbc_bytes == counter.hbc_bytes, "builds must be reproducible"


@needs_ndsr
def test_ndsr_inspect(counter):
    r = ndsr_inspect(counter.hbc_path)
    assert r["abi_valid"] is True and r["abi_error"] is None
    assert r["manifest_address"] == counter.manifest_address
    assert [f["name"] for f in r["functions"]] == ["get", "increment", "note", "relay"]


@needs_ndsr
def test_storage_events_and_revert(counter, tmp_path):
    a = ndsr_run(counter.hbc_path, "increment", '{"by":2}', tmp_path)
    assert a["success"], a["error"]
    assert json.loads(a["output"]) == {"count": 2}
    assert a["events"] == [{"name": "incremented", "data": {"by": 2, "count": 2}}]
    b = ndsr_run(counter.hbc_path, "increment", "", tmp_path)
    assert json.loads(b["output"]) == {"count": 3}
    # Interpreter start-up is in the build-time snapshot: ~5M gas, not ~170M.
    assert 0 < a["gas_used"] < 10_000_000
    bad = ndsr_run(counter.hbc_path, "increment", '{"by":-1}', tmp_path)
    assert bad["success"] is False
    assert "ValueError: increment must be a positive integer" in bad["error"]
    assert bad["events"] == []
    assert json.loads(ndsr_run(counter.hbc_path, "get", "", tmp_path)["output"]) == {"count": 3}


@needs_ndsr
def test_ctx_style_and_hash(counter):
    r = ndsr_run(counter.hbc_path, "note", '{"text":"héllo"}')
    assert r["success"], r["error"]
    assert json.loads(r["output"]) == {"saved": True, "hash": keccak256("héllo")}


@needs_ndsr
def test_hive_call(counter, tmp_path):
    install(tmp_path, counter.manifest_address, counter.hbc_bytes)
    inp = json.dumps({"address": counter.manifest_address, "function": "increment", "input": {"by": 4}})
    r = ndsr_run(counter.hbc_path, "relay", inp, tmp_path)
    assert r["success"], r["error"]
    assert json.loads(r["output"]) == {"relayed": {"count": 4}}
    assert [e["name"] for e in r["events"]] == ["incremented", "relayed"]
    assert json.loads(ndsr_run(counter.hbc_path, "get", "", tmp_path)["output"]) == {"count": 4}
    missing = ndsr_run(counter.hbc_path, "relay", json.dumps({"address": "0x" + "1" * 64, "function": "get"}), tmp_path)
    assert missing["success"] is False
    assert "module not found (code -1)" in missing["error"]


SANDBOX = '''
from hivekit import hive
import json

@hive.define("use_time")
def use_time():
    import time
    return time.time()

@hive.define("use_random")
def use_random():
    import random
    return random.random()

@hive.define("use_os")
def use_os():
    import os
    return os.getcwd()

@hive.define("json_roundtrip")
def json_roundtrip(input):
    return json.dumps({"b": [1, "é", None, True], "a": input}, sort_keys=True)

@hive.define("raw")
def raw(input):
    return "raw:" + input if isinstance(input, str) else "json"

@hive.define("fail")
def fail():
    hive.fail("explicit failure")
'''


@needs_ndsr
def test_determinism_and_sandbox(tmp_path):
    r = compile_source(SANDBOX, "sandbox")
    p = tmp_path / "sandbox.hbc"
    p.write_bytes(r.hbc_bytes)
    for fn, mod in (("use_time", "time"), ("use_random", "random"), ("use_os", "os")):
        out = ndsr_run(p, fn)
        assert out["success"] is False
        assert f"No module named '{mod}'" in out["error"]
    a = ndsr_run(p, "json_roundtrip", '{"x":1}')
    assert a["success"], a["error"]
    assert a["output"] == '{"a": {"x": 1}, "b": [1, "\\u00e9", null, true]}'
    assert ndsr_run(p, "json_roundtrip", '{"x":1}')["gas_used"] == a["gas_used"]
    assert ndsr_run(p, "raw", "not json")["output"] == "raw:not json"
    assert ndsr_run(p, "raw", '{"a":1}')["output"] == "json"
    f = ndsr_run(p, "fail")
    assert f["success"] is False and "explicit failure" in f["error"]


@needs_ndsr
def test_token_example(tmp_path):
    r = compile_file(str(EXAMPLES / "token_module.py"), output_dir=str(tmp_path / "b"))
    run = lambda fn, inp: ndsr_run(r.hbc_path, fn, json.dumps(inp), tmp_path)  # noqa: E731
    assert run("mint", {"to": "alice", "amount": 100})["success"]
    t = run("transfer", {"from": "alice", "to": "bob", "amount": 40})
    assert t["success"], t["error"]
    assert t["events"] == [{"name": "transfer", "data": {"from": "alice", "to": "bob", "amount": 40}}]
    over = run("transfer", {"from": "alice", "to": "bob", "amount": 1000})
    assert over["success"] is False and over["error"].endswith("guest abort: insufficient balance")
    assert json.loads(run("balance", {"account": "bob"})["output"]) == {"account": "bob", "balance": 40}
    assert json.loads(run("balance", {"account": "alice"})["output"]) == {"account": "alice", "balance": 60}
