/**
 * Local execution of `.hbc` artifacts.
 *
 * `findNdsr()` locates the NDSR binary (the ground-truth runtime); `runWithNdsr`
 * delegates to `ndsr run`. When no binary is available, `LocalNode` executes the
 * module in Node's WebAssembly engine with an in-memory implementation of the
 * hive-wasm-v1 host functions (no gas metering, no receipts) — useful for quick
 * iteration, not a substitute for testing against NDSR.
 */

import { spawnSync } from 'child_process'
import * as fs from 'fs'
import * as path from 'path'
import { keccak256 } from './canonical'
import { normalizeAddress, readHbc, type Manifest } from './hbc'

/** `$NDSR_BIN`, `ndsr` on PATH, or a `tools/ndsr` in an ancestor directory. */
export function findNdsr(start: string = process.cwd()): string | null {
  const env = process.env.NDSR_BIN
  if (env) return fs.existsSync(env) ? env : null
  for (const dir of (process.env.PATH ?? '').split(path.delimiter)) {
    const p = path.join(dir, 'ndsr')
    if (dir && fs.existsSync(p)) return p
  }
  for (const base of [start, __dirname]) {
    let dir = path.resolve(base)
    for (;;) {
      const p = path.join(dir, 'tools', 'ndsr')
      if (fs.existsSync(p) && fs.statSync(p).isFile()) return p
      const up = path.dirname(dir)
      if (up === dir) break
      dir = up
    }
  }
  return null
}

export interface RunResult {
  success: boolean
  output: string
  error: string | null
  gasUsed?: number
  events: Array<{ name: string; data: unknown }>
  receipt?: unknown
}

/** Run `ndsr run <hbc> <fn> --input <input>` and parse its JSON report. */
export function runWithNdsr(
  ndsr: string,
  hbcPath: string,
  fn: string,
  input: string,
  opts: { gas?: number; dataDir?: string } = {},
): RunResult {
  const args = ['run', hbcPath, fn, `--input=${input}`, '--gas', String(opts.gas ?? 1_000_000_000)]
  if (opts.dataDir) args.push('--data-dir', opts.dataDir)
  const r = spawnSync(ndsr, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
  if (r.error) throw r.error
  let report: Record<string, unknown>
  try {
    report = JSON.parse(r.stdout)
  } catch {
    throw new Error(`ndsr run failed (exit ${r.status}): ${r.stderr || r.stdout}`)
  }
  const receipt = (report.receipt as { receipt?: { events?: RunResult['events'] } } | undefined)?.receipt
  return {
    success: report.success === true,
    output: String(report.output ?? ''),
    error: (report.error as string | null) ?? null,
    gasUsed: report.gas_used as number,
    events: receipt?.events ?? [],
    receipt: report.receipt,
  }
}

interface Artifact {
  manifest: Manifest
  module: WebAssembly.Module
  address: string
}

class Abort extends Error {}

/** In-memory hive-wasm-v1 host for Node (see module docs for limitations). */
export class LocalNode {
  private readonly modules = new Map<string, Artifact>()
  /** Committed state: address -> key -> value. */
  readonly state = new Map<string, Map<string, string>>()
  logs: string[] = []
  maxDepth = 8

  /** Register an artifact; returns its address. */
  load(hbc: Uint8Array): string {
    const { manifest, wasm, manifestAddress } = readHbc(hbc)
    this.modules.set(manifestAddress, { manifest, module: new WebAssembly.Module(wasm as Uint8Array<ArrayBuffer>), address: manifestAddress })
    return manifestAddress
  }

  execute(address: string, fn: string, input: string): RunResult {
    const events: RunResult['events'] = []
    const journal = new Map<string, Map<string, string | null>>()
    try {
      const out = this.invoke(normalizeAddress(address), fn, input, 0, journal, events)
      for (const [addr, writes] of journal) {
        const s = this.state.get(addr) ?? new Map<string, string>()
        for (const [k, v] of writes) {
          if (v === null) s.delete(k)
          else s.set(k, v)
        }
        this.state.set(addr, s)
      }
      return { success: true, output: out, error: null, events }
    } catch (e) {
      return { success: false, output: '', error: (e as Error).message, events: [] }
    }
  }

  private invoke(
    address: string,
    fn: string,
    input: string,
    depth: number,
    journal: Map<string, Map<string, string | null>>,
    events: RunResult['events'],
  ): string {
    const art = this.modules.get(address)
    if (!art) throw new Error(`module ${address} not found`)
    const funcId = art.manifest.functions.indexOf(fn)
    if (funcId < 0) throw new Error(`function ${fn} not found in module ${address}`)
    const enc = new TextEncoder()
    const dec = new TextDecoder('utf-8', { fatal: true })
    // eslint-disable-next-line prefer-const -- assigned after the import closures that capture it
    let inst: WebAssembly.Instance
    const mem = (): Uint8Array => new Uint8Array((inst.exports.memory as WebAssembly.Memory).buffer)
    const read = (p: number, l: number): Uint8Array => mem().slice(p >>> 0, (p >>> 0) + (l >>> 0))
    const text = (p: number, l: number): string => dec.decode(read(p, l))
    const give = (bytes: Uint8Array): bigint => {
      if (bytes.length === 0) return 0n
      const p = (inst.exports.__alloc as (n: number) => number)(bytes.length) >>> 0
      mem().set(bytes, p)
      return (BigInt(p) << 32n) | BigInt(bytes.length)
    }
    const local = new Map<string, string | null>()
    const lookup = (k: string): string | null => {
      if (local.has(k)) return local.get(k) as string | null
      const j = journal.get(address)
      if (j?.has(k)) return j.get(k) as string | null
      return this.state.get(address)?.get(k) ?? null
    }
    const myEvents: RunResult['events'] = []
    const imports = {
      hive: {
        call: (ap: number, al: number, fp: number, fl: number, ip: number, il: number): bigint => {
          let addr: string
          try {
            addr = normalizeAddress(text(ap, al))
          } catch {
            return -6n
          }
          if (depth + 1 > this.maxDepth) return -5n
          const callee = this.modules.get(addr)
          if (!callee) return -1n
          const f = text(fp, fl)
          if (!callee.manifest.functions.includes(f)) return -2n
          const childJournal = new Map<string, Map<string, string | null>>()
          // The callee sees the caller's pending writes.
          for (const [a, w] of journal) childJournal.set(a, new Map(w))
          const mine = childJournal.get(address) ?? new Map<string, string | null>()
          for (const [k, v] of local) mine.set(k, v)
          childJournal.set(address, mine)
          const childEvents: RunResult['events'] = []
          let out: string
          try {
            out = this.invoke(addr, f, text(ip, il), depth + 1, childJournal, childEvents)
          } catch {
            return -3n
          }
          journal.clear()
          for (const [a, w] of childJournal) journal.set(a, w)
          local.clear()
          myEvents.push(...childEvents)
          return give(enc.encode(out))
        },
        emit: (np: number, nl: number, dp: number, dl: number): void => {
          myEvents.push({ name: text(np, nl), data: JSON.parse(text(dp, dl)) })
        },
        abort: (p: number, l: number): void => {
          throw new Abort(`guest abort: ${text(p, l)}`)
        },
      },
      storage: {
        get: (kp: number, kl: number): bigint => {
          const v = lookup(text(kp, kl))
          return v === null ? 0n : give(enc.encode(v))
        },
        set: (kp: number, kl: number, vp: number, vl: number): void => {
          const k = text(kp, kl)
          if (!k) throw new Abort('storage.set with an empty key')
          local.set(k, vl === 0 ? null : text(vp, vl))
        },
        del: (kp: number, kl: number): void => void local.set(text(kp, kl), null),
      },
      console: { log: (p: number, l: number): void => void this.logs.push(text(p, l)) },
      crypto: { hash: (p: number, l: number): bigint => give(enc.encode(keccak256(read(p, l)))) },
      env: {
        abort: (_m: number, _f: number, line: number, col: number): void => {
          throw new Abort(`guest abort (AssemblyScript) at line ${line}, column ${col}`)
        },
      },
    }
    inst = new WebAssembly.Instance(art.module, imports as unknown as WebAssembly.Imports)
    const bytes = enc.encode(input)
    const p = bytes.length ? give(bytes) : 0n
    const ptr = Number(p >> 32n)
    const r = (inst.exports.__hive_entry as (a: number, b: number, c: number) => bigint)(funcId, ptr, bytes.length)
    if (r < 0n) throw new Error(`guest returned error code ${r}`)
    const out = text(Number(r >> 32n), Number(r & 0xffffffffn))
    const j = journal.get(address) ?? new Map<string, string | null>()
    for (const [k, v] of local) j.set(k, v)
    journal.set(address, j)
    events.push(...myEvents)
    return out
  }
}
