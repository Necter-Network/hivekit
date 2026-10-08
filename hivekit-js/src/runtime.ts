/**
 * The `hive` API for JavaScript/TypeScript modules.
 *
 * Inside a compiled module, `require('hivekit')` resolves to the engine's
 * built-in implementation (runtime-js/src/prelude.js) backed by the node's host
 * functions. In Node.js (unit tests, `hivec run --local`) this file provides the
 * same API over an in-memory store so handlers can be exercised without a node.
 * Semantics are kept identical:
 *
 * - handler input: the call input parsed as JSON; the raw string if it is not
 *   JSON; `{}` when empty.
 * - handler output: strings are returned as-is, `undefined` as "", anything
 *   else as `JSON.stringify(value)`.
 * - a handler whose first parameter is named `ctx` receives a context object
 *   (`ctx.input`, `ctx.db`, `ctx.storage`, `ctx.log`, …); otherwise it receives
 *   `(input, ctx)`.
 */

export type Handler = (input: any, ctx: HiveContext) => unknown // eslint-disable-line @typescript-eslint/no-explicit-any

export interface Storage {
  get(key: string): string | null
  set(key: string, value: unknown): void
  del(key: string): void
  delete(key: string): void
}

export interface Db {
  get<T = unknown>(key: string, dflt?: T): T | null
  set(key: string, value: unknown): void
  del(key: string): void
  delete(key: string): void
}

export interface HiveContext {
  input: unknown
  db: Db
  storage: Storage
  log: { info(...a: unknown[]): void; warn(...a: unknown[]): void; error(...a: unknown[]): void; debug(...a: unknown[]): void }
  emit(name: string, data?: unknown): void
  call(address: string, fn: string, input?: unknown): unknown
  hash(data: unknown): string
  fail(msg: string): never
}

export interface HiveEvent {
  name: string
  data: unknown
}

export class HiveCallError extends Error {
  constructor(public readonly code: number, address: string, fn: string) {
    const reasons: Record<string, string> = {
      '-1': 'module not found',
      '-2': 'function not found',
      '-3': 'callee failed',
      '-5': 'call depth exceeded',
      '-6': 'malformed address',
    }
    super(`hive.call(${address}, ${fn}) failed: ${reasons[String(code)] ?? 'error'} (code ${code})`)
    this.name = 'HiveCallError'
  }
}

export class HiveAbort extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'HiveAbort'
  }
}

function toText(v: unknown): string {
  if (typeof v === 'string') return v
  if (v === undefined) return ''
  return JSON.stringify(v)
}

function decode(s: string): unknown {
  if (s === '') return {}
  try {
    return JSON.parse(s)
  } catch {
    return s
  }
}

function wantsCtx(fn: (...a: never[]) => unknown): boolean {
  return /^\s*(async\s*)?(function\s*[\w$]*\s*)?\(?\s*ctx\s*[,)=:]/.test(Function.prototype.toString.call(fn))
}

/** Host hooks used by the in-memory runtime; replaceable for tests. */
export interface LocalHost {
  call?(address: string, fn: string, input: string): string | number
  hash?(data: string): string
  log?(msg: string): void
}

export class HiveRuntime {
  readonly handlers = new Map<string, Handler>()
  /** In-memory state used by local invocations. */
  readonly state = new Map<string, string>()
  /** Events emitted by the most recent local invocation. */
  events: HiveEvent[] = []
  host: LocalHost = {}

  readonly storage: Storage = {
    get: (key) => this.state.get(String(key)) ?? null,
    set: (key, value) => {
      const k = String(key)
      if (!k) throw new Error('storage key must not be empty')
      const v = toText(value)
      if (v === '') this.state.delete(k)
      else this.state.set(k, v)
    },
    del: (key) => void this.state.delete(String(key)),
    delete: (key) => void this.state.delete(String(key)),
  }

  readonly db: Db = {
    get: <T>(key: string, dflt?: T): T | null => {
      const s = this.state.get(String(key))
      if (s === undefined) return dflt === undefined ? null : dflt
      return JSON.parse(s) as T
    },
    set: (key, value) => this.storage.set(key, JSON.stringify(value === undefined ? null : value)),
    del: (key) => this.storage.del(key),
    delete: (key) => this.storage.del(key),
  }

  /** Register `handler` as the exported function `name`. */
  define(name: string, handler: Handler): this {
    if (typeof name !== 'string') throw new TypeError('hive.define: name must be a string literal')
    if (typeof handler !== 'function') throw new TypeError(`hive.define(${name}): handler must be a function`)
    if (this.handlers.has(name)) throw new Error(`hive.define: function ${name} is defined more than once`)
    this.handlers.set(name, handler)
    return this
  }

  /** Append an event; `data` must be JSON without floats on a real node. */
  emit(name: string, data?: unknown): void {
    this.events.push({ name: String(name), data: data === undefined ? null : JSON.parse(JSON.stringify(data)) })
  }

  /** Synchronous cross-module call; returns the callee output decoded like handler input. */
  call(address: string, fn: string, input?: unknown): unknown {
    return decode(this.callRaw(address, fn, input))
  }

  callRaw(address: string, fn: string, input?: unknown): string {
    if (!this.host.call) throw new HiveCallError(-1, address, fn)
    const r = this.host.call(String(address), String(fn), toText(input === undefined ? '' : input))
    if (typeof r === 'number') throw new HiveCallError(r, address, fn)
    return r
  }

  tryCall(address: string, fn: string, input?: unknown): unknown {
    try {
      return this.call(address, fn, input)
    } catch (e) {
      if (e instanceof HiveCallError) return null
      throw e
    }
  }

  /** keccak256 of the UTF-8 text as `0x` + hex. */
  hash(data: unknown): string {
    const text = typeof data === 'string' ? data : toText(data)
    if (this.host.hash) return this.host.hash(text)
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { keccak256 } = require('./canonical') as typeof import('./canonical')
    return keccak256(text)
  }

  log(...args: unknown[]): void {
    const msg = args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ')
    if (this.host.log) this.host.log(msg)
  }

  /** Fail the call; all state changes of the call are reverted. */
  fail(msg: string): never {
    throw new HiveAbort(String(msg))
  }

  /** Accepted for source compatibility; no effect. */
  config(_cfg?: unknown): this {
    return this
  }

  /** Accepted for source compatibility; no effect. */
  schedule(..._args: unknown[]): this {
    return this
  }

  listFunctions(): string[] {
    return [...this.handlers.keys()].sort()
  }

  private ctx(input: unknown): HiveContext {
    const log = (...a: unknown[]): void => this.log(...a)
    return {
      input,
      db: this.db,
      storage: this.storage,
      log: { info: log, warn: log, error: log, debug: log },
      emit: (n, d) => this.emit(n, d),
      call: (a, f, i) => this.call(a, f, i),
      hash: (d) => this.hash(d),
      fail: (m) => this.fail(m),
    }
  }

  /**
   * Run a function in-process with the same input/output conventions as a node.
   * State changes are rolled back if the handler throws.
   */
  async invoke(name: string, rawInput: unknown = ''): Promise<{ output: string; events: HiveEvent[] }> {
    const handler = this.handlers.get(name)
    if (!handler) throw new Error(`function ${name} is not defined (available: ${this.listFunctions().join(', ')})`)
    const snapshot = new Map(this.state)
    this.events = []
    const input = decode(typeof rawInput === 'string' ? rawInput : toText(rawInput))
    try {
      const ctx = this.ctx(input)
      const result = wantsCtx(handler as never) ? (handler as unknown as (c: HiveContext) => unknown)(ctx) : handler(input, ctx)
      const value = await result
      return { output: toText(value), events: this.events }
    } catch (e) {
      this.state.clear()
      for (const [k, v] of snapshot) this.state.set(k, v)
      this.events = []
      throw e
    }
  }
}

/** The shared `hive` object (also what `require('hivekit')` returns in Node). */
export const hive = new HiveRuntime()
export const storage = hive.storage
export const db = hive.db
