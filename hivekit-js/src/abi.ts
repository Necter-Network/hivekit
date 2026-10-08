/**
 * Static check of a module against the hive-wasm-v1 guest ABI (HBC_SPEC §6.1, §6.4).
 * The node performs the authoritative validation; this catches SDK/toolchain
 * mistakes before an artifact is handed out.
 */

const I32 = 0x7f
const I64 = 0x7e

type Sig = { params: number[]; results: number[] }

const sig = (params: number[], results: number[]): Sig => ({ params, results })
const i32s = (n: number): number[] => Array(n).fill(I32)

export const ALLOWED_IMPORTS: Record<string, Sig> = {
  'hive.call': sig(i32s(6), [I64]),
  'hive.emit': sig(i32s(4), []),
  'hive.abort': sig(i32s(2), []),
  'storage.get': sig(i32s(2), [I64]),
  'storage.set': sig(i32s(4), []),
  'storage.del': sig(i32s(2), []),
  'console.log': sig(i32s(2), []),
  'crypto.hash': sig(i32s(2), [I64]),
  'env.abort': sig(i32s(4), []),
}

const REQUIRED_EXPORTS: Record<string, Sig> = {
  __alloc: sig([I32], [I32]),
  __hive_entry: sig(i32s(3), [I64]),
}

const sameSig = (a: Sig, b: Sig): boolean =>
  a.params.length === b.params.length &&
  a.results.length === b.results.length &&
  a.params.every((x, i) => x === b.params[i]) &&
  a.results.every((x, i) => x === b.results[i])

function fmt(s: Sig): string {
  const n = (t: number): string => (t === I32 ? 'i32' : t === I64 ? 'i64' : t === 0x7d ? 'f32' : t === 0x7c ? 'f64' : `0x${t.toString(16)}`)
  return `(${s.params.map(n).join(', ')}) -> (${s.results.map(n).join(', ')})`
}

export interface AbiReport {
  imports: string[]
  exports: string[]
}

/** Throw a descriptive error if `wasm` violates the hive-wasm-v1 ABI. */
export function validateAbi(wasm: Uint8Array): AbiReport {
  let p = 8
  const u32 = (): number => {
    let r = 0
    let s = 0
    for (;;) {
      const b = wasm[p++]
      if (b === undefined) throw new Error('wasm: unexpected end')
      r += (b & 0x7f) * 2 ** s
      if (!(b & 0x80)) return r
      s += 7
    }
  }
  const name = (): string => {
    const n = u32()
    const s = new TextDecoder().decode(wasm.subarray(p, p + n))
    p += n
    return s
  }
  if (wasm.length < 8 || wasm[0] !== 0 || wasm[1] !== 0x61 || wasm[2] !== 0x73 || wasm[3] !== 0x6d) {
    throw new Error('not a wasm module')
  }
  const types: Sig[] = []
  const funcTypes: number[] = [] // type index per function (imports first)
  const imports: string[] = []
  const exportMap = new Map<string, { kind: number; index: number }>()
  let hasMemory = false
  while (p < wasm.length) {
    const id = wasm[p++]
    const size = u32()
    const end = p + size
    if (id === 1) {
      const n = u32()
      for (let i = 0; i < n; i++) {
        const form = wasm[p++]
        if (form !== 0x60) throw new Error('wasm: unsupported type form (GC/function references are disabled)')
        const np = u32()
        const params = Array.from(wasm.subarray(p, p + np))
        p += np
        const nr = u32()
        const results = Array.from(wasm.subarray(p, p + nr))
        p += nr
        types.push({ params, results })
      }
    } else if (id === 2) {
      const n = u32()
      for (let i = 0; i < n; i++) {
        const mod = name()
        const field = name()
        const kind = wasm[p++]
        const key = `${mod}.${field}`
        if (kind !== 0) throw new Error(`import ${key}: only function imports are allowed (no imported memory/table/global)`)
        const t = u32()
        const allowed = ALLOWED_IMPORTS[key]
        if (!allowed) {
          const hint = mod.startsWith('wasi') ? ' (WASI is not available on hive-wasm-v1)' : ''
          throw new Error(`import ${key} is not part of hive-wasm-v1${hint}`)
        }
        if (!sameSig(types[t], allowed)) throw new Error(`import ${key} has signature ${fmt(types[t])}, expected ${fmt(allowed)}`)
        imports.push(key)
        funcTypes.push(t)
      }
    } else if (id === 3) {
      const n = u32()
      for (let i = 0; i < n; i++) funcTypes.push(u32())
    } else if (id === 5) {
      const n = u32()
      if (n !== 1) throw new Error('module must define exactly one memory')
      const flags = wasm[p]
      if (flags & ~1) throw new Error('memory must be a non-shared 32-bit memory')
      hasMemory = true
    } else if (id === 7) {
      const n = u32()
      for (let i = 0; i < n; i++) {
        const nm = name()
        const kind = wasm[p++]
        exportMap.set(nm, { kind, index: u32() })
      }
    }
    p = end
  }
  const mem = exportMap.get('memory')
  if (!mem || mem.kind !== 2 || !hasMemory) throw new Error('module must export its memory as "memory"')
  for (const [nm, want] of Object.entries(REQUIRED_EXPORTS)) {
    const e = exportMap.get(nm)
    if (!e || e.kind !== 0) throw new Error(`module must export function ${nm}`)
    const t = types[funcTypes[e.index]]
    if (!sameSig(t, want)) throw new Error(`export ${nm} has signature ${fmt(t)}, expected ${fmt(want)}`)
  }
  if (exportMap.has('__hive_entry_str')) {
    // Legacy ABI marker; NDSR rejects modules that rely on it.
    if (!exportMap.has('__hive_entry')) throw new Error('legacy __hive_entry_str ABI is not supported')
  }
  return { imports, exports: [...exportMap.keys()] }
}
