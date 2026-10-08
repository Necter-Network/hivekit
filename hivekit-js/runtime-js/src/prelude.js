// HiveKit JavaScript runtime prelude (hive-wasm-v1).
// Evaluated before the user's script on every call. Provides `require('hivekit')`,
// the `hive` / `storage` / `db` API, and the dispatcher used by the Rust host glue.
(function () {
  'use strict'
  var N = {
    get: globalThis.__hk_get, set: globalThis.__hk_set, del: globalThis.__hk_del,
    emit: globalThis.__hk_emit, call: globalThis.__hk_call, hash: globalThis.__hk_hash,
    log: globalThis.__hk_log, abort: globalThis.__hk_abort,
  }
  ;['__hk_get', '__hk_set', '__hk_del', '__hk_emit', '__hk_call', '__hk_hash', '__hk_log', '__hk_abort']
    .forEach(function (k) { delete globalThis[k] })

  // ── Determinism: no clocks, no randomness ───────────────────────────────────
  Math.random = function () {
    throw new Error('Math.random() is not available on hive-wasm-v1 (non-deterministic); derive values from input or hive.hash()')
  }
  var RealDate = Date
  function HiveDate() {
    if (arguments.length === 0) {
      throw new Error('new Date() without arguments is not available on hive-wasm-v1 (no clock); pass a timestamp from the input')
    }
    if (!new.target) throw new Error('Date() as a function reads the clock and is not available on hive-wasm-v1')
    var a = Array.prototype.slice.call(arguments)
    return Reflect.construct(RealDate, a, new.target)
  }
  HiveDate.prototype = RealDate.prototype
  HiveDate.UTC = RealDate.UTC
  HiveDate.parse = RealDate.parse
  HiveDate.now = function () {
    throw new Error('Date.now() is not available on hive-wasm-v1 (no clock); pass a timestamp from the input')
  }
  Object.defineProperty(RealDate.prototype, 'constructor', { value: HiveDate, writable: true, configurable: true })
  globalThis.Date = HiveDate

  function toText(v) {
    if (typeof v === 'string') return v
    if (v === undefined) return ''
    return JSON.stringify(v)
  }
  function decode(s) {
    if (s === '') return {}
    try { return JSON.parse(s) } catch (e) { return s }
  }
  function errText(e) {
    if (e && typeof e === 'object' && 'message' in e) return (e.name || 'Error') + ': ' + e.message
    return String(e)
  }

  var registry = new Map()

  function HiveCallError(code, address, fn) {
    var reasons = { '-1': 'module not found', '-2': 'function not found', '-3': 'callee failed',
      '-5': 'call depth exceeded', '-6': 'malformed address' }
    var e = new Error('hive.call(' + address + ', ' + fn + ') failed: ' + (reasons[String(code)] || 'error') + ' (code ' + code + ')')
    e.name = 'HiveCallError'
    e.code = code
    return e
  }

  var storage = {
    get: function (key) { return N.get(String(key)) },
    set: function (key, value) { N.set(String(key), toText(value)) },
    del: function (key) { N.del(String(key)) },
    delete: function (key) { N.del(String(key)) },
  }
  var db = {
    get: function (key, dflt) {
      var s = N.get(String(key))
      if (s === null) return dflt === undefined ? null : dflt
      return JSON.parse(s)
    },
    set: function (key, value) { N.set(String(key), JSON.stringify(value === undefined ? null : value)) },
    del: function (key) { N.del(String(key)) },
    delete: function (key) { N.del(String(key)) },
  }
  function log() {
    var parts = []
    for (var i = 0; i < arguments.length; i++) {
      var a = arguments[i]
      parts.push(typeof a === 'string' ? a : JSON.stringify(a))
    }
    N.log(parts.join(' '))
  }
  var hive = {
    define: function (name, handler) {
      if (typeof name !== 'string') throw new TypeError('hive.define: name must be a string literal')
      if (typeof handler !== 'function') throw new TypeError('hive.define(' + name + '): handler must be a function')
      if (registry.has(name)) throw new Error('hive.define: function ' + name + ' is defined more than once')
      registry.set(name, handler)
      return hive
    },
    emit: function (name, data) { N.emit(String(name), JSON.stringify(data === undefined ? null : data)) },
    callRaw: function (address, fn, input) {
      var r = N.call(String(address), String(fn), toText(input === undefined ? '' : input))
      if (typeof r === 'number') throw HiveCallError(r, address, fn)
      return r
    },
    call: function (address, fn, input) { return decode(hive.callRaw(address, fn, input)) },
    tryCall: function (address, fn, input) {
      var r = N.call(String(address), String(fn), toText(input === undefined ? '' : input))
      return typeof r === 'number' ? null : decode(r)
    },
    hash: function (data) { return N.hash(typeof data === 'string' ? data : toText(data)) },
    log: log,
    fail: function (msg) { N.abort(String(msg)) },
    storage: storage,
    db: db,
    // Accepted for source compatibility; they have no effect inside a module.
    config: function () { return hive },
    schedule: function () { return hive },
  }

  var api = { hive: hive, storage: storage, db: db, default: hive }
  var exportsObj = {}
  globalThis.module = { exports: exportsObj }
  globalThis.exports = exportsObj
  globalThis.require = function (name) {
    if (name === 'hivekit' || name === 'hivekit/runtime' || name === '@necter/hivekit') return api
    throw new Error("require('" + name + "'): only 'hivekit' is available inside a hive-wasm-v1 module; bundle other dependencies into the source")
  }
  globalThis.hive = hive
  globalThis.storage = storage
  globalThis.db = db
  globalThis.console = { log: log, info: log, warn: log, error: log, debug: log }

  function makeCtx(input) {
    return { input: input, db: db, storage: storage, log: { info: log, warn: log, error: log, debug: log },
      emit: hive.emit, call: hive.call, hash: hive.hash, fail: hive.fail }
  }
  function wantsCtx(fn) {
    var src = Function.prototype.toString.call(fn)
    return /^\s*(async\s*)?(function\s*[\w$]*\s*)?\(?\s*ctx\s*[,)=:]/.test(src)
  }

  var pending = null
  // Called by the host glue: start function `name` with the raw input string.
  globalThis.__hive_dispatch = function (name, raw) {
    var handler = registry.get(name)
    if (!handler) N.abort('function ' + name + ' is in the manifest but was not registered with hive.define')
    var input = decode(raw)
    var result
    try {
      result = wantsCtx(handler) ? handler(makeCtx(input)) : handler(input, makeCtx(input))
    } catch (e) {
      N.abort(errText(e))
    }
    pending = { state: 'done', value: result }
    if (result && typeof result.then === 'function') {
      pending = { state: 'pending' }
      var p = pending
      result.then(function (v) { p.state = 'done'; p.value = v }, function (e) { p.state = 'error'; p.error = e })
    }
  }
  // Called after the job queue has been drained: the output string.
  globalThis.__hive_finish = function () {
    var p = pending
    if (!p || p.state === 'pending') N.abort('handler returned a promise that never settled (only hive.* calls may be awaited)')
    if (p.state === 'error') N.abort(errText(p.error))
    return toText(p.value)
  }
})();
