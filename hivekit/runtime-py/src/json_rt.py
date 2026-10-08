# `json` for the HiveKit Python runtime: CPython-compatible dumps/loads for the
# common arguments. Parsing and string escaping are native (_hive); structure
# walking is here so `sort_keys`, `separators`, `indent` and `ensure_ascii`
# behave as in CPython.
import _hive

__all__ = ["dumps", "loads", "JSONDecodeError"]


class JSONDecodeError(ValueError):
    pass


def loads(s, **kwargs):
    if isinstance(s, (bytes, bytearray)):
        s = s.decode("utf-8")
    try:
        return _hive.json_loads(s)
    except ValueError as e:
        raise JSONDecodeError(str(e))


def _float(f):
    if f != f:
        return "NaN"
    if f == float("inf"):
        return "Infinity"
    if f == -float("inf"):
        return "-Infinity"
    return repr(f)


def dumps(obj, skipkeys=False, ensure_ascii=True, check_circular=True, allow_nan=True, cls=None,
          indent=None, separators=None, default=None, sort_keys=False, **kwargs):
    if isinstance(indent, int):
        indent = " " * indent
    if separators is not None:
        item_sep, key_sep = separators
    elif indent is not None:
        item_sep, key_sep = ",", ": "
    else:
        item_sep, key_sep = ", ", ": "
    out = []

    def enc(o, level):
        if o is None:
            out.append("null")
        elif o is True:
            out.append("true")
        elif o is False:
            out.append("false")
        elif isinstance(o, str):
            out.append(_hive.json_quote(o, ensure_ascii))
        elif isinstance(o, int):
            out.append(int.__repr__(o))
        elif isinstance(o, float):
            if not allow_nan and (o != o or o in (float("inf"), -float("inf"))):
                raise ValueError("Out of range float values are not JSON compliant")
            out.append(_float(o))
        elif isinstance(o, (list, tuple)):
            if not o:
                out.append("[]")
                return
            out.append("[")
            first = True
            for v in o:
                if not first:
                    out.append(item_sep)
                first = False
                if indent is not None:
                    out.append("\n" + indent * (level + 1))
                enc(v, level + 1)
            if indent is not None:
                out.append("\n" + indent * level)
            out.append("]")
        elif isinstance(o, dict):
            if not o:
                out.append("{}")
                return
            items = list(o.items())
            if sort_keys:
                items.sort(key=lambda kv: kv[0])
            out.append("{")
            first = True
            for k, v in items:
                if isinstance(k, str):
                    pass
                elif k is True:
                    k = "true"
                elif k is False:
                    k = "false"
                elif k is None:
                    k = "null"
                elif isinstance(k, (int, float)):
                    k = int.__repr__(k) if isinstance(k, int) else _float(k)
                elif skipkeys:
                    continue
                else:
                    raise TypeError("keys must be str, int, float, bool or None, not %s" % type(k).__name__)
                if not first:
                    out.append(item_sep)
                first = False
                if indent is not None:
                    out.append("\n" + indent * (level + 1))
                out.append(_hive.json_quote(k, ensure_ascii))
                out.append(key_sep)
                enc(v, level + 1)
            if indent is not None:
                out.append("\n" + indent * level)
            out.append("}")
        elif default is not None:
            enc(default(o), level)
        else:
            raise TypeError("Object of type %s is not JSON serializable" % type(o).__name__)

    enc(obj, 0)
    return "".join(out)
