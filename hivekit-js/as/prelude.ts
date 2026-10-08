// ---------------------------------------------------------------------------
// HiveKit AssemblyScript prelude for the hive-wasm-v1 ABI (docs/HBC_SPEC.md).
// Injected by `ndsr compile`. Identifiers starting with `__hive_` are reserved.
// ---------------------------------------------------------------------------
@external("hive", "call") declare function __hive_imp_call(ap: usize, al: usize, fp: usize, fl: usize, ip: usize, il: usize): i64;
@external("hive", "emit") declare function __hive_imp_emit(np: usize, nl: usize, dp: usize, dl: usize): void;
@external("hive", "abort") declare function __hive_imp_abort(p: usize, l: usize): void;
@external("storage", "get") declare function __hive_imp_storage_get(kp: usize, kl: usize): i64;
@external("storage", "set") declare function __hive_imp_storage_set(kp: usize, kl: usize, vp: usize, vl: usize): void;
@external("storage", "del") declare function __hive_imp_storage_del(kp: usize, kl: usize): void;
@external("console", "log") declare function __hive_imp_log(p: usize, l: usize): void;
@external("crypto", "hash") declare function __hive_imp_hash(p: usize, l: usize): i64;

const __hive_registry = new Map<string, (input: string) => string>();

function __hive_utf8(s: string): ArrayBuffer {
  return String.UTF8.encode(s);
}

function __hive_unpack(v: i64): string {
  if (v <= 0) return "";
  const p = <usize>(v >>> 32);
  const l = <usize>(v & 0xffffffff);
  return String.UTF8.decodeUnsafe(p, l);
}

function __hive_pack(s: string): i64 {
  const b = __hive_utf8(s);
  return (<i64>changetype<usize>(b) << 32) | <i64>b.byteLength;
}

namespace hive {
  /** Register `f` as the handler for the exported function `name`. */
  export function define(name: string, f: (input: string) => string): void {
    __hive_registry.set(name, f);
  }

  /** Call `fn` on the module at `address`; aborts the whole call on failure. */
  export function call(address: string, fn: string, input: string): string {
    const r = tryCallRaw(address, fn, input);
    if (r < 0) {
      fail("hive.call failed with code " + r.toString());
      unreachable();
    }
    return __hive_unpack(r);
  }

  /** Like `call` but returns null on failure (child state is reverted). */
  export function tryCall(address: string, fn: string, input: string): string | null {
    const r = tryCallRaw(address, fn, input);
    if (r < 0) return null;
    return __hive_unpack(r);
  }

  /** Raw result: >= 0 packed output, < 0 error code (HBC_SPEC.md §6.3). */
  export function tryCallRaw(address: string, fn: string, input: string): i64 {
    const a = __hive_utf8(address);
    const f = __hive_utf8(fn);
    const i = __hive_utf8(input);
    return __hive_imp_call(
      changetype<usize>(a), a.byteLength,
      changetype<usize>(f), f.byteLength,
      changetype<usize>(i), i.byteLength);
  }

  /** Emit an event; `dataJson` must be JSON without floats. */
  export function emit(name: string, dataJson: string): void {
    const n = __hive_utf8(name);
    const d = __hive_utf8(dataJson);
    __hive_imp_emit(changetype<usize>(n), n.byteLength, changetype<usize>(d), d.byteLength);
  }

  /** Debug log (node-local, not part of consensus). */
  export function log(msg: string): void {
    const m = __hive_utf8(msg);
    __hive_imp_log(changetype<usize>(m), m.byteLength);
  }

  /** keccak256 of the UTF-8 bytes of `data`, as "0x" + 64 hex. */
  export function hash(data: string): string {
    const d = __hive_utf8(data);
    return __hive_unpack(__hive_imp_hash(changetype<usize>(d), d.byteLength));
  }

  /** Fail the call with a message (all state changes are reverted). */
  export function fail(msg: string): void {
    const m = __hive_utf8(msg);
    __hive_imp_abort(changetype<usize>(m), m.byteLength);
  }
}

namespace storage {
  /** Value for `key`, or "" if unset. */
  export function get(key: string): string {
    const k = __hive_utf8(key);
    return __hive_unpack(__hive_imp_storage_get(changetype<usize>(k), k.byteLength));
  }

  /** Set `key` (setting "" deletes it). */
  export function set(key: string, value: string): void {
    const k = __hive_utf8(key);
    const v = __hive_utf8(value);
    __hive_imp_storage_set(changetype<usize>(k), k.byteLength, changetype<usize>(v), v.byteLength);
  }

  export function del(key: string): void {
    const k = __hive_utf8(key);
    __hive_imp_storage_del(changetype<usize>(k), k.byteLength);
  }
}
// ----------------------------- end of prelude ------------------------------
