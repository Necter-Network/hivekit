"""
HiveModule — the core SDK object.

Provides hive.define(), hive.consensus(), hive.schedule(), hive.config(),
hive.db, and hive.files.

Handler styles — all three are supported:

    # 1. ctx-style (spec-aligned, access everything via ctx)
    @hive.define("createUser")
    def create_user(ctx):
        name = ctx.input["name"]
        ctx.db.set(f"user:{name}", {"name": name})
        ctx.log.info(f"created {name}")
        return {"ok": True}

    # 2. input-style (simple, backwards-compatible)
    @hive.define("addNumbers")
    def add_numbers(input):
        return {"total": input["a"] + input["b"]}

    # 3. both (input first, ctx second)
    @hive.define("greet")
    def greet(input, ctx):
        ctx.log.info("greeting")
        return {"message": f"Hello, {input['name']}!"}
"""

import base64
import hashlib
import inspect
import json
import time
import functools
from dataclasses import dataclass, field, asdict
from typing import Callable, Dict, List, Optional, Any, Tuple

# ── Global state (written by the module-level `hive` singleton) ───────────────

_registry: Dict[str, Callable] = {}
_consensus_config: Optional[Dict[str, Any]] = None
_module_config: Dict[str, Any] = {}
_schedules: Dict[str, Dict[str, Any]] = {}


# ── NRC-1 types ───────────────────────────────────────────────────────────────

@dataclass
class NRC1Config:
    """
    NRC-1 configuration — declares this module as a mining application.

    Miners on the Necter network see your app and opt in to run your consensus handler.
    Each successful BFT round credits them ``reward_per_unit * compute_units`` tokens.
    """
    reward_token: str
    reward_chain: str           # "base" | "polygon" | "arbitrum" | "necter"
    reward_per_unit: str        # wei string, e.g. "1000000000000000000" = 1 token
    token_standard: str = "ERC-20"
    units_per_execution: int = 1
    min_units: int = 0
    max_units: int = 0
    display_name: str = ""
    description: str = ""
    icon_url: str = ""

    def to_dict(self) -> Dict[str, Any]:
        return {k: v for k, v in asdict(self).items() if v not in ("", 0)}


@dataclass
class ConsensusContext:
    """Context passed to a consensus handler."""
    input: Dict[str, Any]
    round: int = 0
    participants: List[str] = field(default_factory=list)


@dataclass
class ConsensusResult:
    """Return value from a consensus handler."""
    output: Dict[str, Any]
    compute_units: Optional[int] = None


# ── hive.db ───────────────────────────────────────────────────────────────────

class HiveDB:
    """
    Decentralised key-value store.

    In local dev the store is in-memory (and optionally flushed to disk).
    On NDSR the runtime provides a replicated, content-addressed store.

        ctx.db.set("user:alice", {"score": 42})
        val = ctx.db.get("user:alice")   # {"score": 42}
        ctx.db.delete("user:alice")
        keys = ctx.db.keys("user:")      # all keys with that prefix
    """

    def __init__(self) -> None:
        self._store: Dict[str, Any] = {}

    def set(self, key: str, value: Any) -> None:
        self._store[key] = value

    def get(self, key: str, default: Any = None) -> Any:
        return self._store.get(key, default)

    def delete(self, key: str) -> bool:
        if key in self._store:
            del self._store[key]
            return True
        return False

    def keys(self, prefix: str = "") -> List[str]:
        if prefix:
            return [k for k in self._store if k.startswith(prefix)]
        return list(self._store.keys())

    def all(self, prefix: str = "") -> Dict[str, Any]:
        """Return all key-value pairs, optionally filtered by prefix."""
        if prefix:
            return {k: v for k, v in self._store.items() if k.startswith(prefix)}
        return dict(self._store)

    def clear(self) -> None:
        self._store.clear()


# ── hive.files ────────────────────────────────────────────────────────────────

CHUNK_SIZE = 512 * 1024  # 512 KB per chunk


@dataclass
class FileRef:
    """Metadata returned after uploading a file (no payload)."""
    file_id: str
    url: str            # gateway URL or hivekit://files/{id} in local dev
    name: str
    type: str           # MIME type
    size: int           # total bytes (raw, not base64)
    chunks: int         # number of chunks the file was split into
    metadata: Dict[str, Any] = field(default_factory=dict)


@dataclass
class FileData:
    """Full file record — includes reassembled base64-encoded content."""
    file_id: str
    url: str
    name: str
    type: str
    size: int
    chunks: int
    data: str           # base64-encoded full content (reassembled)
    data_url: str       # data:{type};base64,{data}  — ready for browsers / <img src>
    metadata: Dict[str, Any] = field(default_factory=dict)


class HiveFiles:
    """
    Distributed file storage with transparent chunking.

    Large files are automatically split into 512 KB chunks, each
    content-addressed by its SHA-256 hash.  On download the chunks are
    reassembled transparently.  The caller only ever sees a stable URL
    and base64 data — chunking is invisible.

    Local dev URL:   ``hivekit://files/{file_id}``
    Deployed URL:    ``https://files.necter.network/{file_id}``
                     (set via HiveFiles.set_gateway())

    Usage::

        # Upload (base64 string or raw bytes — both accepted)
        ref = hive.files.upload(
            data=image_bytes,
            name="photo.jpg",
            type="image/jpeg",
            metadata={"uploadedBy": ctx.user},
        )
        print(ref.url)     # https://files.necter.network/abc123...
        print(ref.chunks)  # 3  (transparent to caller)

        # Retrieve — chunks reassembled automatically
        fd = hive.files.get(ref.file_id)
        print(fd.data)      # base64 string
        print(fd.data_url)  # data:image/jpeg;base64,...  (use in <img src>)

        # List / delete
        for f in hive.files.list(): print(f.url)
        hive.files.delete(ref.file_id)
    """

    # Gateway URL — overridden at deploy time by the NDSR runtime.
    # Developers can also call HiveFiles.set_gateway("https://...") manually.
    _gateway: str = "hivekit://files"

    @classmethod
    def set_gateway(cls, url: str) -> None:
        """Point the SDK at a live file gateway (called by the NDSR runtime)."""
        cls._gateway = url.rstrip("/")

    def __init__(self) -> None:
        # chunk_id → raw bytes
        self._chunks: Dict[str, bytes] = {}
        # file_id  → manifest dict
        self._manifests: Dict[str, dict] = {}

    # ── public API ─────────────────────────────────────────────────────────────

    def upload(
        self,
        data,
        name: str = "file",
        type: str = "application/octet-stream",
        metadata: Optional[Dict[str, Any]] = None,
    ) -> FileRef:
        """
        Store a file.  Accepts raw ``bytes`` or a base64 ``str``.

        The file is split into 512 KB chunks, each stored by its SHA-256
        content hash.  A manifest is created that lists all chunks in order.
        The returned ``FileRef.url`` is a stable public URL to the full file.
        """
        raw = _to_bytes(data)
        file_id = hashlib.sha256(raw).hexdigest()

        # Split into chunks and store each one
        chunk_records = []
        for i, offset in enumerate(range(0, max(len(raw), 1), CHUNK_SIZE)):
            chunk_raw  = raw[offset : offset + CHUNK_SIZE]
            chunk_id   = hashlib.sha256(chunk_raw).hexdigest()
            self._chunks[chunk_id] = chunk_raw
            chunk_records.append({"index": i, "chunk_id": chunk_id, "size": len(chunk_raw)})

        manifest = {
            "file_id":    file_id,
            "name":       name,
            "type":       type,
            "size":       len(raw),
            "chunk_size": CHUNK_SIZE,
            "chunks":     chunk_records,
            "metadata":   metadata or {},
            "created_at": _utcnow(),
        }
        self._manifests[file_id] = manifest

        return FileRef(
            file_id  = file_id,
            url      = self._url(file_id),
            name     = name,
            type     = type,
            size     = len(raw),
            chunks   = len(chunk_records),
            metadata = metadata or {},
        )

    def get(self, file_id: str) -> Optional[FileData]:
        """
        Retrieve a file by ID.

        Chunks are reassembled in order.  Returns ``None`` if the file
        (or any of its chunks) is not found.
        """
        manifest = self._manifests.get(file_id)
        if manifest is None:
            return None

        # Reassemble chunks in index order
        parts: List[bytes] = []
        for c in sorted(manifest["chunks"], key=lambda x: x["index"]):
            chunk = self._chunks.get(c["chunk_id"])
            if chunk is None:
                return None   # chunk missing (shouldn't happen locally)
            parts.append(chunk)

        raw      = b"".join(parts)
        b64      = base64.b64encode(raw).decode()
        mime     = manifest["type"]

        return FileData(
            file_id  = file_id,
            url      = self._url(file_id),
            name     = manifest["name"],
            type     = mime,
            size     = manifest["size"],
            chunks   = len(manifest["chunks"]),
            data     = b64,
            data_url = f"data:{mime};base64,{b64}",
            metadata = manifest.get("metadata", {}),
        )

    def manifest(self, file_id: str) -> Optional[dict]:
        """Return the raw manifest dict for a file."""
        return self._manifests.get(file_id)

    def list(self) -> List[FileRef]:
        """List all stored files (metadata only — no payload)."""
        out = []
        for m in self._manifests.values():
            out.append(FileRef(
                file_id  = m["file_id"],
                url      = self._url(m["file_id"]),
                name     = m["name"],
                type     = m["type"],
                size     = m["size"],
                chunks   = len(m["chunks"]),
                metadata = m.get("metadata", {}),
            ))
        return out

    def delete(self, file_id: str) -> bool:
        """
        Delete a file and all its chunks.
        Returns True if the file existed, False otherwise.
        """
        manifest = self._manifests.pop(file_id, None)
        if manifest is None:
            return False
        for c in manifest["chunks"]:
            self._chunks.pop(c["chunk_id"], None)
        return True

    # ── static helpers ─────────────────────────────────────────────────────────

    @staticmethod
    def encode(raw: bytes) -> str:
        """Encode raw bytes → base64 string (ready to pass to upload())."""
        return base64.b64encode(raw).decode()

    @staticmethod
    def decode(data: str) -> bytes:
        """Decode a base64 string → raw bytes."""
        return base64.b64decode(data)

    # ── private ────────────────────────────────────────────────────────────────

    def _url(self, file_id: str) -> str:
        return f"{self.__class__._gateway}/{file_id}"


# ── small utilities used by HiveFiles ─────────────────────────────────────────

def _to_bytes(data) -> bytes:
    """Accept raw bytes, bytearray, or a base64 string. Always return bytes."""
    if isinstance(data, (bytes, bytearray)):
        return bytes(data)
    if isinstance(data, str):
        # Strip data-URL prefix if present (data:image/png;base64,...)
        if "," in data and data.startswith("data:"):
            data = data.split(",", 1)[1]
        return base64.b64decode(data)
    raise TypeError(f"Expected bytes or base64 str, got {type(data).__name__}")


def _utcnow() -> str:
    from datetime import datetime, timezone
    return datetime.now(timezone.utc).isoformat()


# ── ctx helpers ───────────────────────────────────────────────────────────────

@dataclass
class NodeInfo:
    """Information about the miner node executing this function."""
    id: str       = "local-dev-node"
    location: str = "local"
    version: str  = "dev"


@dataclass
class RequestInfo:
    """Request metadata injected by NDSR."""
    ip: str                            = "127.0.0.1"
    headers: Dict[str, str]            = field(default_factory=dict)
    timestamp: int                     = 0   # Unix ms


class HiveLogger:
    """
    Structured logger available as ``ctx.log``.

    Logs are captured in local dev and forwarded to the NDSR telemetry
    stream when deployed.

        ctx.log.info("user created")
        ctx.log.warn("slow response")
        ctx.log.error("payment failed")
    """

    def __init__(self, module_name: str = "hivekit") -> None:
        self._module = module_name
        self.entries: List[Dict[str, str]] = []

    def info(self, message: str) -> None:
        self._emit("info", message)

    def warn(self, message: str) -> None:
        self._emit("warn", message)

    def error(self, message: str) -> None:
        self._emit("error", message)

    def debug(self, message: str) -> None:
        self._emit("debug", message)

    def _emit(self, level: str, message: str) -> None:
        entry = {"level": level, "message": message, "module": self._module,
                 "ts": int(time.time() * 1000)}
        self.entries.append(entry)
        if level != "debug":
            print(f"[{self._module}] [{level.upper()}] {message}")


@dataclass
class HiveContext:
    """
    Full execution context injected into every handler.

    Available as ``ctx`` when your handler's first parameter is named ``ctx``::

        @hive.define("getProfile")
        def get_profile(ctx):
            user_id = ctx.input["userId"]
            return ctx.db.get(f"profile:{user_id}") or {}
    """
    input:   Dict[str, Any]
    user:    str = ""                          # caller wallet address (if authenticated)
    env:     Dict[str, str] = field(default_factory=dict)   # from hive.config environment
    node:    NodeInfo       = field(default_factory=NodeInfo)
    request: RequestInfo    = field(default_factory=RequestInfo)
    log:     HiveLogger     = field(default_factory=HiveLogger)
    db:      HiveDB         = field(default_factory=HiveDB)
    files:   HiveFiles      = field(default_factory=HiveFiles)
    storage: Any            = None


# ── HiveModule ────────────────────────────────────────────────────────────────

class HiveModule:
    """
    The ``hive`` object available to every HiveKit module.

    Three handler styles are all supported — the SDK detects which one
    you're using by inspecting the first parameter name:

        @hive.define("fn")
        def fn(ctx):        ...   # ctx-style  — first param named 'ctx'

        @hive.define("fn")
        def fn(input):      ...   # input-style — anything else

        @hive.define("fn")
        def fn(input, ctx): ...   # both        — two parameters
    """

    def __init__(self) -> None:
        self._local_registry:  Dict[str, Callable]      = {}
        self._local_consensus: Optional[Dict[str, Any]] = None
        self._config:          Dict[str, Any]            = {}
        self._schedules:       Dict[str, Dict[str, Any]] = {}
        self.db    = HiveDB()
        self.files = HiveFiles()
        self.storage = _RawStorage(self.db)
        self.events: List[Dict[str, Any]] = []
        self.host_call: Optional[Callable[[str, str, str], Any]] = None

    # ── hive.config() ─────────────────────────────────────────────────────────

    def config(self, configuration: Dict[str, Any]) -> "HiveModule":
        """
        Set module-level settings. ``environment`` is exposed to handlers as
        ``ctx.env``. Nothing here is written to the manifest (HBC_SPEC §3).

            hive.config({
                "name":    "My App",
                "version": "1.0.0",
                "memory":  "512mb",
                "storage": "5gb",
                "tags":    ["depin", "ai"],
                "environment": {
                    "API_KEY": "...",
                    "NETWORK": "mainnet",
                },
            })
        """
        self._config = configuration
        global _module_config
        _module_config = configuration
        return self

    # ── hive.define() ─────────────────────────────────────────────────────────

    def define(self, name: str) -> Callable:
        """
        Register a function as a HiveKit export.

        Supports three handler signatures — detected automatically::

            @hive.define("greet")
            def greet(ctx):            # ctx-style
                return {"hi": ctx.input["name"]}

            @hive.define("add")
            def add(input):            # input-style (backwards-compat)
                return {"sum": input["a"] + input["b"]}

            @hive.define("log")
            def log(input, ctx):       # both
                ctx.log.info("called")
                return {"ok": True}
        """
        def decorator(fn: Callable) -> Callable:
            if not isinstance(name, str):
                raise TypeError("hive.define: name must be a string literal")
            if name in self._local_registry:
                raise ValueError(f"hive.define: function {name} is defined more than once")
            style = _detect_handler_style(fn)

            @functools.wraps(fn)
            def wrapper(raw_input: str) -> str:
                data = _decode(raw_input)
                if style == "ctx":
                    result = fn(self._make_ctx(data))
                elif style == "both":
                    result = fn(data, self._make_ctx(data))
                elif style == "none":
                    result = fn()
                else:
                    result = fn(data)
                return _encode(result)

            self._local_registry[name] = wrapper
            _registry[name] = wrapper
            wrapper.__hive_name__ = name
            wrapper.__hive_raw__  = fn
            return wrapper
        return decorator

    # ── hive.schedule() ───────────────────────────────────────────────────────

    def schedule(self, name: str, cron: str, handler: Callable) -> "HiveModule":
        """
        Register a recurring task.

        The cron string follows standard 5-field POSIX syntax.
        On NDSR the runtime triggers the handler network-wide on the schedule.
        In local dev, use ``invoke_schedule(name)`` to trigger it manually.

            hive.schedule("syncPrices", "*/5 * * * *", lambda: (
                hive.db.set("prices", fetch_prices())
            ))
        """
        self._schedules[name] = {"cron": cron, "handler": handler}
        _schedules[name] = {"cron": cron}   # compiler reads this (no fn ref)
        return self

    def invoke_schedule(self, name: str) -> Any:
        """Trigger a scheduled task immediately (local dev / testing only)."""
        if name not in self._schedules:
            raise KeyError(f"No schedule '{name}'. Registered: {list(self._schedules)}")
        return self._schedules[name]["handler"]()

    # ── hive.consensus() ──────────────────────────────────────────────────────

    def consensus(self, config: NRC1Config) -> Callable:
        """
        Register this module as an NRC-1 mining application.

        The decorated function receives a :class:`ConsensusContext` and must
        return a :class:`ConsensusResult`.
        """
        def decorator(fn: Callable) -> Callable:
            if self._local_consensus is not None:
                raise RuntimeError("hive.consensus() can only be called once per module")
            cfg_dict = config.to_dict()
            self._local_consensus = cfg_dict
            global _consensus_config
            _consensus_config = cfg_dict

            @functools.wraps(fn)
            def wrapper(raw_input: str) -> str:
                data = _decode(raw_input)
                if not isinstance(data, dict):
                    data = {}
                ctx = ConsensusContext(
                    input        = data,
                    round        = int(data.get("__round", 0)),
                    participants = data.get("__participants", []),
                )
                result = fn(ctx)
                units  = result.compute_units
                if units is None:
                    units = config.units_per_execution
                if config.min_units > 0 and units < config.min_units:
                    units = config.min_units
                if config.max_units > 0 and units > config.max_units:
                    units = config.max_units

                out = dict(result.output)
                out["__compute_units"] = units
                return _encode(out)

            self._local_registry["__consensus"] = wrapper
            _registry["__consensus"]            = wrapper
            wrapper.__hive_name__ = "__consensus"
            wrapper.__hive_raw__  = fn
            return wrapper
        return decorator

    # ── hive.call() ───────────────────────────────────────────────────────────

    def call(self, module_address: str, function_name: str, input_data: Any = "") -> Any:
        """Cross-module call. Returns the callee output decoded (JSON if it parses).

        Inside a node this is a synchronous host call. Locally it needs a host:
        assign ``hive.host_call = fn(address, function, input_str) -> str | int``."""
        return _decode(self.call_raw(module_address, function_name, input_data))

    def call_raw(self, module_address: str, function_name: str, input_data: Any = "") -> str:
        if self.host_call is None:
            raise NotImplementedError(
                "hive.call() needs a node; run the module with `hivekit run` (uses ndsr) "
                "or set hive.host_call for local tests.\n"
                f"Attempted: {module_address}.{function_name}({input_data!r})"
            )
        r = self.host_call(module_address, function_name, _encode(input_data))
        if isinstance(r, int):
            raise HiveCallError(r, module_address, function_name)
        return r

    def try_call(self, module_address: str, function_name: str, input_data: Any = "") -> Any:
        try:
            return self.call(module_address, function_name, input_data)
        except HiveCallError:
            return None

    def emit(self, name: str, data: Any = None) -> None:
        """Append an event ``{name, data}`` (data must be JSON without floats on a node)."""
        self.events.append({"name": str(name), "data": json.loads(json.dumps(data))})

    def hash(self, data: Any) -> str:
        """keccak256 of the UTF-8 text as ``0x`` + 64 hex."""
        from .canonical import keccak256
        return keccak256(data if isinstance(data, str) else _encode(data))

    def log(self, *args: Any) -> None:
        print(" ".join(a if isinstance(a, str) else _encode(a) for a in args))

    def fail(self, message: str) -> None:
        """Fail the call (all state changes are reverted on a node)."""
        raise HiveAbort(str(message))

    # ── introspection ─────────────────────────────────────────────────────────

    def list_functions(self) -> List[str]:
        """Return all registered function names (excludes schedules)."""
        return list(self._local_registry.keys())

    def list_schedules(self) -> Dict[str, str]:
        """Return {name: cron} for all registered schedules."""
        return {n: s["cron"] for n, s in self._schedules.items()}

    # ── local testing ─────────────────────────────────────────────────────────

    def invoke_local(self, name: str, input_data: Any = None) -> Any:
        """
        Call a registered function locally — no deployment needed.

            result = hive.invoke_local("addNumbers", {"a": 10, "b": 32})
            # {"total": 42}
        """
        if name not in self._local_registry:
            raise KeyError(
                f"No function '{name}' registered. "
                f"Available: {list(self._local_registry)}"
            )
        self.events = []
        raw = input_data if isinstance(input_data, str) else _encode(input_data)
        return _decode(self._local_registry[name](raw))

    # ── private ───────────────────────────────────────────────────────────────

    def _make_ctx(self, input_data: dict) -> HiveContext:
        """Build a HiveContext for a single handler invocation."""
        env = self._config.get("environment", {})
        return HiveContext(
            input   = input_data,
            user    = input_data.get("__user", ""),
            env     = env,
            node    = NodeInfo(),
            request = RequestInfo(),
            log     = HiveLogger(self._config.get("name", "hivekit")),
            db      = self.db,
            files   = self.files,
            storage = self.storage,
        )


# ── input/output conventions (identical to the runtime) ───────────────────────

def _decode(raw: Any) -> Any:
    """Call input -> handler input: JSON if it parses, else the raw string; {} if empty."""
    if raw is None or raw == "":
        return {}
    if not isinstance(raw, str):
        return raw
    try:
        return json.loads(raw)
    except ValueError:
        return raw


def _encode(value: Any) -> str:
    """Handler result -> call output: str as-is, None as "", else compact JSON."""
    if isinstance(value, str):
        return value
    if value is None:
        return ""
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


class HiveCallError(Exception):
    """A hive.call failed with a negative code (HBC_SPEC §6.5)."""

    REASONS = {-1: "module not found", -2: "function not found", -3: "callee failed",
               -5: "call depth exceeded", -6: "malformed address"}

    def __init__(self, code: int, address: str, function: str) -> None:
        self.code = code
        super().__init__(f"hive.call({address}, {function}) failed: {self.REASONS.get(code, 'error')} (code {code})")


class HiveAbort(Exception):
    """Raised by hive.fail(); on a node the call fails and its state changes are reverted."""


class _RawStorage:
    """String-valued view over the local store (``hive.storage`` / ``ctx.storage``)."""

    def __init__(self, db: "HiveDB") -> None:
        self._db = db

    def get(self, key: str, default: Any = None) -> Any:
        v = self._db._store.get(key)
        if v is None:
            return default
        return v if isinstance(v, str) else _encode(v)

    def set(self, key: str, value: Any) -> None:
        if not key:
            raise ValueError("storage key must not be empty")
        text = value if isinstance(value, str) else _encode(value)
        if text == "":
            self._db._store.pop(key, None)
        else:
            self._db._store[key] = text

    def delete(self, key: str) -> bool:
        return self._db.delete(key)


# ── handler-style detection ───────────────────────────────────────────────────

def _detect_handler_style(fn: Callable) -> str:
    """
    Return 'ctx', 'both', or 'input' based on the handler's parameter names.

    Rules:
      - No parameters                     → 'none'
      - First param named 'ctx'         → 'ctx'   (full HiveContext injected)
      - Two or more params               → 'both'  (input dict, then HiveContext)
      - Anything else (0 or 1 non-ctx)  → 'input' (plain dict, backwards-compat)
    """
    try:
        params = list(inspect.signature(fn).parameters.keys())
    except (ValueError, TypeError):
        return "input"

    if not params:
        return "none"
    if params[0] == "ctx":
        return "ctx"
    if len(params) >= 2:
        return "both"
    return "input"
