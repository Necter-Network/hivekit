/**
 * `.hbc` artifacts (HBC_SPEC.md §2–§5): manifest schema, content address and
 * deterministic ZIP packaging.
 */

import { inflateRawSync } from 'zlib'
import { canonicalBytes, keccak256 } from './canonical'

export const RUNTIME = 'hive-wasm-v1'
export const MAX_WASM_BYTES = 12 * 1024 * 1024 // HBC_SPEC §2 (raised from 8 MiB)
export const MAX_MANIFEST_BYTES = 64 * 1024
export const MAX_HBC_BYTES = 16 * 1024 * 1024

export interface Manifest {
  name: string
  language: string
  compiler: string
  runtime: typeof RUNTIME
  functions: string[]
  version?: string
  description?: string
  manifest_address?: string
}

const ALLOWED_KEYS = new Set(['name', 'language', 'compiler', 'runtime', 'functions', 'version', 'description', 'manifest_address'])
const FN_RE = /^[A-Za-z_][A-Za-z0-9_]{0,63}$/
const LANG_RE = /^[a-z0-9_+-]{1,32}$/

const utf8Len = (s: string): number => new TextEncoder().encode(s).length

export function isFunctionName(name: string): boolean {
  return FN_RE.test(name)
}

/** Byte-wise ascending comparison of UTF-8 strings (function names are ASCII). */
function byteCompare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0
}

/** Sort, de-duplicate check and validate function names. */
export function sortFunctions(names: string[]): string[] {
  const sorted = [...names].sort(byteCompare)
  for (let i = 1; i < sorted.length; i++) {
    if (sorted[i] === sorted[i - 1]) throw new Error(`function "${sorted[i]}" is defined more than once`)
  }
  for (const n of sorted) {
    if (!isFunctionName(n)) throw new Error(`invalid function name "${n}" (must match [A-Za-z_][A-Za-z0-9_]{0,63})`)
  }
  return sorted
}

/** Throw if `m` violates the manifest schema (HBC_SPEC §3). */
export function validateManifest(m: Record<string, unknown>): asserts m is Manifest & Record<string, unknown> {
  for (const k of Object.keys(m)) {
    if (!ALLOWED_KEYS.has(k)) throw new Error(`manifest: key "${k}" is not allowed`)
  }
  const str = (k: string, min: number, max: number, required: boolean): void => {
    const v = m[k]
    if (v === undefined) {
      if (required) throw new Error(`manifest: "${k}" is required`)
      return
    }
    if (typeof v !== 'string') throw new Error(`manifest: "${k}" must be a string`)
    const n = utf8Len(v)
    if (n < min || n > max) throw new Error(`manifest: "${k}" must be ${min}-${max} bytes`)
  }
  str('name', 1, 128, true)
  str('language', 1, 32, true)
  str('compiler', 1, 128, true)
  str('version', 0, 1024, false)
  str('description', 0, 1024, false)
  if (!LANG_RE.test(m.language as string)) throw new Error('manifest: "language" must match [a-z0-9_+-]{1,32}')
  if (m.runtime !== RUNTIME) throw new Error(`manifest: "runtime" must be "${RUNTIME}"`)
  const fns = m.functions
  if (!Array.isArray(fns) || fns.length < 1 || fns.length > 256) throw new Error('manifest: "functions" must have 1-256 entries')
  for (let i = 0; i < fns.length; i++) {
    if (typeof fns[i] !== 'string' || !isFunctionName(fns[i])) throw new Error(`manifest: invalid function name ${JSON.stringify(fns[i])}`)
    if (i > 0 && byteCompare(fns[i - 1], fns[i]) >= 0) throw new Error('manifest: "functions" must be sorted ascending and unique')
  }
  if (m.manifest_address !== undefined && typeof m.manifest_address !== 'string') {
    throw new Error('manifest: "manifest_address" must be a string')
  }
}

/** Build a validated manifest (without `manifest_address`). */
export function buildManifest(opts: {
  name: string
  language: string
  compiler: string
  functions: string[]
  version?: string
  description?: string
}): Manifest {
  const m: Manifest = {
    name: opts.name,
    language: opts.language,
    compiler: opts.compiler,
    runtime: RUNTIME,
    functions: sortFunctions(opts.functions),
  }
  if (opts.version !== undefined) m.version = opts.version
  if (opts.description !== undefined) m.description = opts.description
  validateManifest(m as unknown as Record<string, unknown>)
  return m
}

/** keccak256(canonical_json(manifest - manifest_address) || wasm), `0x` + 64 lowercase hex. */
export function manifestAddress(manifest: Record<string, unknown>, wasm: Uint8Array): string {
  const rest: Record<string, unknown> = { ...manifest }
  delete rest.manifest_address
  const m = canonicalBytes(rest)
  const buf = new Uint8Array(m.length + wasm.length)
  buf.set(m, 0)
  buf.set(wasm, m.length)
  return keccak256(buf)
}

/** Normalize `hive:0x…` / upper-case addresses to the canonical `0x` + lowercase form. */
export function normalizeAddress(address: string): string {
  let a = address.trim()
  if (a.toLowerCase().startsWith('hive:')) a = a.slice(5)
  a = a.toLowerCase()
  if (!/^0x[0-9a-f]{64}$/.test(a)) throw new Error(`invalid module address: ${address}`)
  return a
}

// ── deterministic ZIP (stored, zero timestamps) ─────────────────────────────

const CRC_TABLE = (() => {
  const t = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c >>> 0
  }
  return t
})()

export function crc32(data: Uint8Array): number {
  let c = 0xffffffff
  for (let i = 0; i < data.length; i++) c = CRC_TABLE[(c ^ data[i]) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

/** Deterministic stored ZIP of the given entries (no validation; see packageHbc). */
export function zipStored(entries: Array<[string, Uint8Array]>): Uint8Array {
  const chunks: Uint8Array[] = []
  const central: Uint8Array[] = []
  let offset = 0
  for (const [name, data] of entries) {
    const nameBytes = new TextEncoder().encode(name)
    const crc = crc32(data)
    const local = new DataView(new ArrayBuffer(30))
    local.setUint32(0, 0x04034b50, true)
    local.setUint16(4, 10, true) // version needed
    local.setUint16(6, 0, true) // flags
    local.setUint16(8, 0, true) // stored
    local.setUint16(10, 0, true) // time
    local.setUint16(12, 0x21, true) // date 1980-01-01
    local.setUint32(14, crc, true)
    local.setUint32(18, data.length, true)
    local.setUint32(22, data.length, true)
    local.setUint16(26, nameBytes.length, true)
    local.setUint16(28, 0, true)
    chunks.push(new Uint8Array(local.buffer), nameBytes, data)

    const c = new DataView(new ArrayBuffer(46))
    c.setUint32(0, 0x02014b50, true)
    c.setUint16(4, 0x031e, true) // made by: unix, 3.0
    c.setUint16(6, 10, true)
    c.setUint16(8, 0, true)
    c.setUint16(10, 0, true)
    c.setUint16(12, 0, true)
    c.setUint16(14, 0x21, true)
    c.setUint32(16, crc, true)
    c.setUint32(20, data.length, true)
    c.setUint32(24, data.length, true)
    c.setUint16(28, nameBytes.length, true)
    c.setUint16(30, 0, true)
    c.setUint16(32, 0, true)
    c.setUint16(34, 0, true)
    c.setUint16(36, 0, true)
    c.setUint32(38, 0o100644 << 16, true)
    c.setUint32(42, offset, true)
    central.push(new Uint8Array(c.buffer), nameBytes)
    offset += 30 + nameBytes.length + data.length
  }
  const cdSize = central.reduce((n, b) => n + b.length, 0)
  const end = new DataView(new ArrayBuffer(22))
  end.setUint32(0, 0x06054b50, true)
  end.setUint16(8, entries.length, true)
  end.setUint16(10, entries.length, true)
  end.setUint32(12, cdSize, true)
  end.setUint32(16, offset, true)
  const all = [...chunks, ...central, new Uint8Array(end.buffer)]
  const out = new Uint8Array(all.reduce((n, b) => n + b.length, 0))
  let p = 0
  for (const b of all) {
    out.set(b, p)
    p += b.length
  }
  return out
}

export interface Packaged {
  hbc: Uint8Array
  manifest: Manifest
  manifestAddress: string
}

/** Validate, address and package `manifest.json` + `module.wasm`. */
export function packageHbc(manifest: Manifest, wasm: Uint8Array): Packaged {
  if (wasm.length > MAX_WASM_BYTES) {
    throw new Error(`module.wasm is ${wasm.length} bytes; the hive-wasm-v1 limit is ${MAX_WASM_BYTES}`)
  }
  const base: Record<string, unknown> = { ...manifest }
  delete base.manifest_address
  validateManifest(base)
  const address = manifestAddress(base, wasm)
  const full = { ...(base as unknown as Manifest), manifest_address: address }
  const manifestBytes = canonicalBytes(full)
  if (manifestBytes.length > MAX_MANIFEST_BYTES) throw new Error('manifest.json exceeds 64 KiB')
  const hbc = zipStored([
    ['manifest.json', manifestBytes],
    ['module.wasm', wasm],
  ])
  return { hbc, manifest: full, manifestAddress: address }
}

/** Read a `.hbc`, enforcing the container rules and verifying the address. */
export function readHbc(bytes: Uint8Array): { manifest: Manifest; wasm: Uint8Array; manifestAddress: string } {
  if (bytes.length > MAX_HBC_BYTES) throw new Error('.hbc exceeds 16 MiB')
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  let eocd = -1
  for (let i = bytes.length - 22; i >= Math.max(0, bytes.length - 22 - 65535); i--) {
    if (dv.getUint32(i, true) === 0x06054b50) {
      eocd = i
      break
    }
  }
  if (eocd < 0) throw new Error('not a ZIP archive')
  const count = dv.getUint16(eocd + 10, true)
  let p = dv.getUint32(eocd + 16, true)
  const files = new Map<string, Uint8Array>()
  for (let i = 0; i < count; i++) {
    if (dv.getUint32(p, true) !== 0x02014b50) throw new Error('corrupt ZIP central directory')
    const method = dv.getUint16(p + 10, true)
    const csize = dv.getUint32(p + 20, true)
    const nlen = dv.getUint16(p + 28, true)
    const xlen = dv.getUint16(p + 30, true)
    const clen = dv.getUint16(p + 32, true)
    const lho = dv.getUint32(p + 42, true)
    const name = new TextDecoder().decode(bytes.subarray(p + 46, p + 46 + nlen))
    p += 46 + nlen + xlen + clen
    if (files.has(name)) throw new Error(`duplicate entry ${name}`)
    if (name !== 'manifest.json' && name !== 'module.wasm') throw new Error(`unexpected entry "${name}" (only manifest.json and module.wasm are allowed)`)
    const lnlen = dv.getUint16(lho + 26, true)
    const lxlen = dv.getUint16(lho + 28, true)
    const start = lho + 30 + lnlen + lxlen
    const raw = bytes.subarray(start, start + csize)
    let data: Uint8Array
    if (method === 0) data = raw
    else if (method === 8) data = new Uint8Array(inflateRawSync(raw, { maxOutputLength: MAX_WASM_BYTES + 1 }))
    else throw new Error(`unsupported ZIP compression method ${method}`)
    files.set(name, data)
  }
  const mBytes = files.get('manifest.json')
  const wasm = files.get('module.wasm')
  if (!mBytes || !wasm) throw new Error('.hbc must contain manifest.json and module.wasm')
  if (mBytes.length > MAX_MANIFEST_BYTES) throw new Error('manifest.json exceeds 64 KiB')
  if (wasm.length > MAX_WASM_BYTES) throw new Error(`module.wasm exceeds ${MAX_WASM_BYTES} bytes`)
  const manifest = JSON.parse(new TextDecoder().decode(mBytes)) as Record<string, unknown>
  validateManifest(manifest)
  const address = manifestAddress(manifest, wasm)
  if (manifest.manifest_address !== undefined && normalizeAddress(manifest.manifest_address) !== address) {
    throw new Error(`manifest_address mismatch: declared ${manifest.manifest_address}, computed ${address}`)
  }
  return { manifest: manifest as Manifest, wasm, manifestAddress: address }
}
