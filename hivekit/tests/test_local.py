"""In-process SDK behaviour (ported from the original test_sdk.py script)."""
import base64

import pytest

from hivekit import (
    ConsensusContext,
    ConsensusResult,
    FileData,
    FileRef,
    HiveAbort,
    HiveCallError,
    HiveDB,
    HiveFiles,
    HiveModule,
    NRC1Config,
    hive,
)
from hivekit.compiler import (
    CompileError,
    collect_hive_functions,
    collect_schedules,
    extract_hive_config,
    extract_nrc1_config,
    is_consensus_module,
)
from hivekit.hbc import build_manifest


def test_imports():
    from hivekit import ConsensusContext, HiveContext, HiveLogger, NodeInfo, RequestInfo  # noqa: F401

    assert isinstance(hive, HiveModule)


# ── handler styles ────────────────────────────────────────────────────────────


@pytest.fixture
def m():
    return HiveModule()


def test_input_style(m):
    @m.define("addNumbers")
    def add_numbers(input):
        return {"total": input["a"] + input["b"]}

    @m.define("greet")
    def greet(input):
        return {"message": f"Hello, {input.get('name', 'stranger')}!"}

    assert m.invoke_local("addNumbers", {"a": 10, "b": 32})["total"] == 42
    assert m.invoke_local("greet", {"name": "Alice"})["message"] == "Hello, Alice!"
    assert "stranger" in m.invoke_local("greet", {})["message"]


def test_ctx_style_and_db(m):
    @m.define("getProfile")
    def get_profile(ctx):
        ctx.db.set("last_call", ctx.input.get("userId"))
        ctx.log.info("getProfile called")
        return {"userId": ctx.input.get("userId"), "node": ctx.node.id}

    r = m.invoke_local("getProfile", {"userId": "alice"})
    assert r == {"userId": "alice", "node": "local-dev-node"}
    assert m.db.get("last_call") == "alice"


def test_both_style_env(m):
    m.config({"name": "EchoApp", "environment": {"NETWORK": "testnet"}})

    @m.define("echo")
    def echo(input, ctx):
        ctx.log.info("echo called")
        return {"echo": input.get("msg"), "env": ctx.env}

    assert m.invoke_local("echo", {"msg": "hello"}) == {"echo": "hello", "env": {"NETWORK": "testnet"}}


def test_no_arg_and_string_io(m):
    @m.define("ping")
    def ping():
        return "pong"

    @m.define("raw")
    def raw(input):
        return f"got {input}"

    assert m.invoke_local("ping") == "pong"
    assert m.invoke_local("raw", "not json") == "got not json"


def test_duplicate_define_rejected(m):
    m.define("a")(lambda input: 1)
    with pytest.raises(ValueError, match="more than once"):
        m.define("a")(lambda input: 2)


def test_exceptions_propagate(m):
    @m.define("boom")
    def boom(input):
        m.fail("nope")

    with pytest.raises(HiveAbort, match="nope"):
        m.invoke_local("boom", {})


def test_emit_hash_and_storage(m):
    @m.define("save")
    def save(input):
        m.storage.set("raw", "text")
        m.emit("saved", {"k": input["k"]})
        return {"hash": m.hash("abc")}

    r = m.invoke_local("save", {"k": 1})
    assert r["hash"] == "0x4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"
    assert m.events == [{"name": "saved", "data": {"k": 1}}]
    assert m.storage.get("raw") == "text"


def test_call_needs_host(m):
    with pytest.raises(NotImplementedError, match="needs a node"):
        m.call("0x" + "1" * 64, "f", {})
    m.host_call = lambda a, f, i: -2
    with pytest.raises(HiveCallError, match="function not found"):
        m.call("0x" + "1" * 64, "f", {})
    assert m.try_call("0x" + "1" * 64, "f") is None
    m.host_call = lambda a, f, i: '{"ok":true}'
    assert m.call("0x" + "1" * 64, "f", {"x": 1}) == {"ok": True}


# ── hive.db / hive.files (local dev) ──────────────────────────────────────────


def test_db():
    db = HiveDB()
    db.set("user:alice", {"score": 42})
    assert db.get("user:alice") == {"score": 42}
    db.set("user:bob", {"score": 10})
    keys = db.keys("user:")
    assert "user:alice" in keys and "user:bob" in keys
    assert db.delete("user:bob") is True
    assert db.get("user:bob") is None
    assert "user:alice" in db.all("user:")


def test_files():
    files = HiveFiles()
    raw = b"Hello, HiveKit files!"
    b64 = base64.b64encode(raw).decode()
    ref = files.upload(b64, name="hello.txt", type="text/plain", metadata={"author": "test"})
    assert isinstance(ref, FileRef)
    assert ref.url.startswith("hivekit://files/")
    assert ref.name == "hello.txt" and ref.size == len(raw)
    fd = files.get(ref.file_id)
    assert isinstance(fd, FileData)
    assert fd.data == b64
    assert fd.data_url.startswith("data:text/plain;base64,")
    assert base64.b64decode(fd.data) == raw
    ref2 = files.upload(b"raw bytes", name="raw.bin", type="application/octet-stream")
    assert HiveFiles.decode(files.get(ref2.file_id).data) == b"raw bytes"
    assert len(files.list()) == 2
    assert files.delete(ref.file_id) is True
    assert files.get(ref.file_id) is None
    enc = HiveFiles.encode(b"hello")
    assert enc == base64.b64encode(b"hello").decode()
    assert HiveFiles.decode(enc) == b"hello"


# ── config / schedule / consensus ─────────────────────────────────────────────


def test_config_and_env(m):
    m.config({"name": "PriceOracle", "version": "1.0.0", "tags": ["depin"], "environment": {"FEED_URL": "https://api.example.com"}})
    assert m._config["name"] == "PriceOracle"

    @m.define("getEnv")
    def get_env(ctx):
        return {"feedUrl": ctx.env.get("FEED_URL")}

    assert m.invoke_local("getEnv", {})["feedUrl"] == "https://api.example.com"


def test_schedule(m):
    ran = []
    m.schedule("syncPrices", "*/5 * * * *", lambda: ran.append("sync"))
    m.schedule("cleanup", "0 * * * *", lambda: ran.append("cleanup"))
    assert m.list_schedules() == {"syncPrices": "*/5 * * * *", "cleanup": "0 * * * *"}
    m.invoke_schedule("syncPrices")
    assert ran == ["sync"]


def test_consensus(m):
    @m.consensus(NRC1Config(reward_token="0xOracleToken", reward_chain="base", reward_per_unit="500000000000000000",
                            units_per_execution=10, min_units=2, max_units=20, display_name="Price Oracle"))
    def oracle(ctx: ConsensusContext) -> ConsensusResult:
        return ConsensusResult(output={"price": ctx.input.get("price", 3200), "round": ctx.round}, compute_units=12)

    r = m.invoke_local("__consensus", {"__round": 7, "__participants": ["0xA", "0xB"], "price": 3500})
    assert r == {"price": 3500, "round": 7, "__compute_units": 12}


def test_consensus_clamp(m):
    @m.consensus(NRC1Config(reward_token="0xT", reward_chain="base", reward_per_unit="1", units_per_execution=5, min_units=2, max_units=8))
    def clamp_fn(ctx):
        return ConsensusResult(output={"ok": True}, compute_units=99)

    assert m.invoke_local("__consensus", {})["__compute_units"] == 8


# ── compiler utilities ────────────────────────────────────────────────────────

APP_SOURCE = """
from hivekit import hive, NRC1Config
hive.config({"name": "TestApp", "version": "2.0.0", "tags": ["depin", "ai"],
             "memory": "512mb", "environment": {"API_KEY": "secret"}})

@hive.define("multiply")
def mul(input): return {}

@hive.define("addNumbers")
def add(input): return {}

hive.define("direct", lambda input: 1)

hive.schedule("sync",    "*/5 * * * *", lambda: None)
hive.schedule("cleanup", "0 0 * * *",   lambda: None)
"""

CONSENSUS_SOURCE = """
@hive.consensus(NRC1Config(
    reward_token='0xAbc', reward_chain='base', reward_per_unit='1000',
    units_per_execution=5,
))
def oracle(ctx): ...
"""


def test_collect_functions():
    assert collect_hive_functions(APP_SOURCE) == ["multiply", "addNumbers", "direct"]
    assert collect_hive_functions(CONSENSUS_SOURCE) == ["__consensus"]
    # comments and strings are not code
    assert collect_hive_functions('# hive.define("x")\ns = \'hive.define("y")\'\n') == []


def test_collect_functions_requires_literals():
    with pytest.raises(CompileError, match="string literal"):
        collect_hive_functions("name = 'a'\n@hive.define(name)\ndef f(input): ...\n")
    with pytest.raises(CompileError, match="syntax error at line 1"):
        collect_hive_functions("def broken(:\n")


def test_static_helpers():
    assert collect_schedules(APP_SOURCE)[0] == {"name": "sync", "cron": "*/5 * * * *"}
    cfg = extract_hive_config(APP_SOURCE)
    assert cfg["name"] == "TestApp" and cfg["version"] == "2.0.0" and "depin" in cfg["tags"]
    assert not is_consensus_module(APP_SOURCE) and is_consensus_module(CONSENSUS_SOURCE)
    nrc1 = extract_nrc1_config(CONSENSUS_SOURCE)
    assert nrc1["reward_token"] == "0xAbc" and nrc1["units_per_execution"] == 5


def test_build_manifest_is_spec_conformant():
    m = build_manifest("TestApp", ["multiply", "addNumbers"], version="2.0.0")
    assert m == {
        "compiler": m["compiler"],
        "functions": ["addNumbers", "multiply"],
        "language": "python",
        "name": "TestApp",
        "runtime": "hive-wasm-v1",
        "version": "2.0.0",
    }
    assert m["compiler"].startswith("hivekit-py/")
    for forbidden in ("created_at", "schedules", "config", "nrc1", "consensus", "wasm_ready"):
        assert forbidden not in m
