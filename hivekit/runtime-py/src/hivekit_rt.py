# HiveKit Python runtime module (hive-wasm-v1).
#
# Installed as `hivekit` inside the embedded interpreter, so module code can
# `from hivekit import hive` exactly as with the host SDK. Host functions come
# from the native `_hive` module. Semantics match hivekit/module.py:
#   input  : the call input parsed as JSON; the raw string if it is not JSON; {} if empty
#   output : str returned as-is, None as "", anything else as compact JSON
#   style  : first parameter named `ctx` -> fn(ctx); one parameter -> fn(input);
#            two or more -> fn(input, ctx)
import _hive
import json

__all__ = ["hive", "HiveModule", "HiveAbort", "HiveCallError", "NRC1Config", "ConsensusContext", "ConsensusResult"]

_REASONS = {-1: "module not found", -2: "function not found", -3: "callee failed",
            -5: "call depth exceeded", -6: "malformed address"}


class HiveAbort(Exception):
    """Raised by hive.fail(); the runtime fails the call with exactly this message."""


class HiveCallError(Exception):
    def __init__(self, code, address, function):
        self.code = code
        Exception.__init__(self, "hive.call(%s, %s) failed: %s (code %d)" % (
            address, function, _REASONS.get(code, "error"), code))


def _text(v):
    if isinstance(v, str):
        return v
    if v is None:
        return ""
    return json.dumps(v, separators=(",", ":"), ensure_ascii=False)


def _decode(s):
    if s == "":
        return {}
    try:
        return json.loads(s)
    except Exception:
        return s


class _Storage:
    """Raw string key/value state of this module."""

    def get(self, key, default=None):
        v = _hive.storage_get(str(key))
        return default if v is None else v

    def set(self, key, value):
        _hive.storage_set(str(key), value if isinstance(value, str) else _text(value))

    def delete(self, key):
        _hive.storage_del(str(key))
        return True

    def keys(self, prefix=""):
        raise NotImplementedError("listing keys is not available on hive-wasm-v1 (no key enumeration in the host ABI)")

    def all(self, prefix=""):
        raise NotImplementedError("listing keys is not available on hive-wasm-v1 (no key enumeration in the host ABI)")


class _Db(_Storage):
    """JSON-valued view of the module state (same API as the host SDK's HiveDB)."""

    def get(self, key, default=None):
        v = _hive.storage_get(str(key))
        return default if v is None else json.loads(v)

    def set(self, key, value):
        _hive.storage_set(str(key), json.dumps(value, separators=(",", ":"), ensure_ascii=False))


class _Logger:
    def __init__(self, name):
        self._name = name

    def _emit(self, level, message):
        _hive.log("[%s] [%s] %s" % (self._name, level, message))

    def info(self, message):
        self._emit("INFO", message)

    def warn(self, message):
        self._emit("WARN", message)

    def error(self, message):
        self._emit("ERROR", message)

    def debug(self, message):
        self._emit("DEBUG", message)


class _Files:
    def __getattr__(self, name):
        raise NotImplementedError("hive.files is not available inside a module; upload files through the CCS gateway")


class NodeInfo:
    id = "ndsr"
    location = ""
    version = "hive-wasm-v1"


class RequestInfo:
    ip = ""
    headers = {}
    timestamp = 0


class HiveContext:
    def __init__(self, module, input):
        self.input = input
        self.user = input.get("__user", "") if isinstance(input, dict) else ""
        self.env = dict(module._config.get("environment", {}) or {})
        self.node = NodeInfo()
        self.request = RequestInfo()
        self.log = _Logger(module._config.get("name", "hivekit"))
        self.db = module.db
        self.storage = module.storage
        self.files = module.files


class NRC1Config:
    def __init__(self, reward_token, reward_chain, reward_per_unit, token_standard="ERC-20",
                 units_per_execution=1, min_units=0, max_units=0, display_name="",
                 description="", icon_url=""):
        self.reward_token = reward_token
        self.reward_chain = reward_chain
        self.reward_per_unit = reward_per_unit
        self.token_standard = token_standard
        self.units_per_execution = units_per_execution
        self.min_units = min_units
        self.max_units = max_units
        self.display_name = display_name
        self.description = description
        self.icon_url = icon_url


class ConsensusContext:
    def __init__(self, input, round=0, participants=None):
        self.input = input
        self.round = round
        self.participants = participants or []


class ConsensusResult:
    def __init__(self, output, compute_units=None):
        self.output = output
        self.compute_units = compute_units


def _style(fn):
    code = getattr(fn, "__code__", None)
    if code is None:
        return "input"
    params = code.co_varnames[:code.co_argcount]
    if not params:
        return "none"
    if params[0] == "ctx":
        return "ctx"
    if len(params) >= 2:
        return "both"
    return "input"


class HiveModule:
    def __init__(self):
        self._registry = {}
        self._config = {}
        self.storage = _Storage()
        self.db = _Db()
        self.files = _Files()

    def define(self, name, fn=None):
        if not isinstance(name, str):
            raise TypeError("hive.define: name must be a string literal")

        def register(f):
            if name in self._registry:
                raise ValueError("hive.define: function %s is defined more than once" % name)
            self._registry[name] = f
            return f

        if fn is not None:
            return register(fn)
        return register

    def consensus(self, config):
        def register(fn):
            def run(input, ctx):
                data = input if isinstance(input, dict) else {}
                c = ConsensusContext(data, int(data.get("__round", 0)), data.get("__participants", []))
                result = fn(c)
                units = result.compute_units
                if units is None:
                    units = config.units_per_execution
                if config.min_units > 0 and units < config.min_units:
                    units = config.min_units
                if config.max_units > 0 and units > config.max_units:
                    units = config.max_units
                out = dict(result.output)
                out["__compute_units"] = units
                return out
            self._registry["__consensus"] = run
            return fn
        return register

    def config(self, configuration=None):
        self._config = dict(configuration or {})
        return self

    def schedule(self, *args, **kwargs):
        return self

    def emit(self, name, data=None):
        _hive.emit(str(name), json.dumps(data, separators=(",", ":"), ensure_ascii=False))

    def call_raw(self, address, function, input=""):
        r = _hive.call(str(address), str(function), _text(input))
        if isinstance(r, int):
            raise HiveCallError(r, address, function)
        return r

    def call(self, address, function, input=""):
        return _decode(self.call_raw(address, function, input))

    def try_call(self, address, function, input=""):
        r = _hive.call(str(address), str(function), _text(input))
        return None if isinstance(r, int) else _decode(r)

    def hash(self, data):
        return _hive.hash(data if isinstance(data, str) else _text(data))

    def log(self, *args):
        _hive.log(" ".join(a if isinstance(a, str) else _text(a) for a in args))

    def fail(self, message):
        # Unwind to the entry point before aborting so the node records the
        # message (deep wasm backtraces can push it out of the error string).
        raise HiveAbort(str(message))

    def list_functions(self):
        return sorted(self._registry)

    def _dispatch(self, name, raw):
        fn = self._registry.get(name)
        if fn is None:
            raise HiveAbort("function %s is in the manifest but was not registered with hive.define" % name)
        data = _decode(raw)
        style = _style(fn)
        if style == "ctx":
            result = fn(HiveContext(self, data))
        elif style == "both":
            result = fn(data, HiveContext(self, data))
        elif style == "none":
            result = fn()
        else:
            result = fn(data)
        return _text(result)


hive = HiveModule()
