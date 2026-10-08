"""HiveKit Python example: storage, events and hive.call.

Build:  hivec build examples/counter.py
Run:    hivec run examples/counter.py increment '{"by": 2}'
"""
from hivekit import hive


@hive.define("increment")
def increment(input):
    by = input.get("by", 1) if isinstance(input, dict) else 1
    if not isinstance(by, int) or by <= 0:
        raise ValueError("increment must be a positive integer")
    count = hive.db.get("count", 0) + by
    hive.db.set("count", count)
    hive.emit("incremented", {"by": by, "count": count})
    return {"count": count}


@hive.define("get")
def get():
    return {"count": hive.db.get("count", 0)}


@hive.define("relay")
def relay(input):
    """Call a function on another module (any language) and return its output."""
    out = hive.call(input["address"], input["function"], input.get("input", ""))
    hive.emit("relayed", {"function": input["function"]})
    return {"relayed": out}


@hive.define("note")
def note(ctx):
    ctx.storage.set("note", ctx.input["text"])
    ctx.log.info("note saved")
    return {"saved": True, "hash": hive.hash(ctx.input["text"])}
