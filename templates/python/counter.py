from hivekit import hive


@hive.define("increment")
def increment(input):
    by = input.get("by", 1) if isinstance(input, dict) else 1
    if not isinstance(by, int) or by <= 0:
        raise ValueError("by must be a positive integer")
    count = hive.db.get("count", 0) + by
    hive.db.set("count", count)
    hive.emit("incremented", {"by": by, "count": count})
    return {"count": count}


@hive.define("get")
def get():
    return {"count": hive.db.get("count", 0)}
