/**
 * Embed a script into a prebuilt HiveKit interpreter runtime (`runtime-js`,
 * `runtime-py`) without recompiling it.
 *
 * The runtime contains a 24-byte `ScriptRef` (16-byte magic + u32 ptr + u32 len)
 * in an active data segment. We:
 *   1. append the script blob as a new active data segment placed at the end of
 *      the module's initial memory (page aligned),
 *   2. raise the memory minimum to cover it,
 *   3. patch ScriptRef.ptr/len in place, and
 *   4. bump the DataCount section if present.
 * The runtime's allocator obtains memory only via `memory.grow`, so these pages
 * are never handed out. The transformation is a pure function of its inputs.
 *
 * The same algorithm is implemented in the Python SDK (hivekit/embed.py); both
 * are tested to produce identical bytes.
 */

export const SCRIPT_MAGIC = new TextEncoder().encode('HIVEKIT-SCRIPT-1')
const PAGE = 65536

class Reader {
  constructor(public b: Uint8Array, public p = 0) {}
  byte(): number {
    if (this.p >= this.b.length) throw new Error('wasm: unexpected end')
    return this.b[this.p++]
  }
  u32(): number {
    let result = 0
    let shift = 0
    for (;;) {
      const x = this.byte()
      result += (x & 0x7f) * 2 ** shift
      if ((x & 0x80) === 0) return result
      shift += 7
      if (shift > 35) throw new Error('wasm: bad LEB128')
    }
  }
}

function uleb(n: number): number[] {
  const out: number[] = []
  do {
    let b = n % 128
    n = Math.floor(n / 128)
    if (n !== 0) b |= 0x80
    out.push(b)
  } while (n !== 0)
  return out
}

function sleb(n: number): number[] {
  const out: number[] = []
  for (;;) {
    const b = n & 0x7f
    n >>= 7
    if ((n === 0 && (b & 0x40) === 0) || (n === -1 && (b & 0x40) !== 0)) {
      out.push(b)
      return out
    }
    out.push(b | 0x80)
  }
}

function concat(parts: Array<Uint8Array | number[]>): Uint8Array {
  const n = parts.reduce((s, x) => s + x.length, 0)
  const out = new Uint8Array(n)
  let p = 0
  for (const x of parts) {
    out.set(x, p)
    p += x.length
  }
  return out
}

function indexOf(hay: Uint8Array, needle: Uint8Array, from = 0): number {
  outer: for (let i = from; i <= hay.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer
    return i
  }
  return -1
}

/** Script blob: sorted function names joined by `\n`, a NUL, then the UTF-8 source. */
export function scriptBlob(functions: string[], source: string): Uint8Array {
  return concat([new TextEncoder().encode(functions.join('\n')), [0], new TextEncoder().encode(source)])
}

/** Return a copy of `runtime` with `blob` embedded. */
export function embedScript(runtime: Uint8Array, blob: Uint8Array): Uint8Array {
  if (runtime.length < 8 || runtime[0] !== 0 || runtime[1] !== 0x61 || runtime[2] !== 0x73 || runtime[3] !== 0x6d) {
    throw new Error('runtime is not a wasm module')
  }
  const sections: Array<{ id: number; body: Uint8Array }> = []
  const r = new Reader(runtime, 8)
  while (r.p < runtime.length) {
    const id = r.byte()
    const size = r.u32()
    sections.push({ id, body: runtime.slice(r.p, r.p + size) })
    r.p += size
  }

  const mem = sections.find((s) => s.id === 5)
  const data = sections.find((s) => s.id === 11)
  if (!mem || !data) throw new Error('runtime has no memory or data section')

  // Memory section: exactly one 32-bit memory.
  const mr = new Reader(mem.body)
  if (mr.u32() !== 1) throw new Error('runtime must define exactly one memory')
  const flags = mr.byte()
  if (flags !== 0 && flags !== 1) throw new Error('runtime memory must be a plain 32-bit memory')
  const minPages = mr.u32()
  const maxPages = flags === 1 ? mr.u32() : undefined

  // Data section: locate ScriptRef inside an active segment and the data end.
  const dr = new Reader(data.body)
  const count = dr.u32()
  const countLen = dr.p
  let dataEnd = 0
  let patchAt = -1
  for (let i = 0; i < count; i++) {
    const kind = dr.u32()
    let offset = -1
    if (kind === 0 || kind === 2) {
      if (kind === 2) dr.u32() // memory index
      if (dr.byte() !== 0x41) throw new Error('runtime data segment offset must be i32.const')
      // signed LEB128 i32
      let result = 0
      let shift = 0
      let b: number
      do {
        b = dr.byte()
        result |= (b & 0x7f) << shift
        shift += 7
      } while (b & 0x80)
      if (shift < 32 && b & 0x40) result |= ~0 << shift
      offset = result >>> 0
      if (dr.byte() !== 0x0b) throw new Error('runtime data segment offset must be a constant expression')
    } else if (kind !== 1) {
      throw new Error(`unsupported data segment kind ${kind}`)
    }
    const len = dr.u32()
    const start = dr.p
    dr.p += len
    if (offset >= 0) {
      dataEnd = Math.max(dataEnd, offset + len)
      const seg = data.body.subarray(start, start + len)
      const at = indexOf(seg, SCRIPT_MAGIC)
      if (at >= 0) {
        if (patchAt >= 0 || indexOf(seg, SCRIPT_MAGIC, at + 1) >= 0) throw new Error('runtime contains ScriptRef more than once')
        patchAt = start + at + SCRIPT_MAGIC.length
      }
    }
  }
  if (patchAt < 0) throw new Error('runtime has no ScriptRef (not a HiveKit interpreter runtime)')
  const dv = new DataView(data.body.buffer, data.body.byteOffset)
  if (dv.getUint32(patchAt, true) !== 0xffffffff || dv.getUint32(patchAt + 4, true) !== 0xffffffff) {
    throw new Error('runtime already has a script embedded')
  }

  const place = Math.max(minPages * PAGE, Math.ceil(dataEnd / PAGE) * PAGE)
  const newMin = place / PAGE + Math.ceil(Math.max(blob.length, 1) / PAGE)
  if (maxPages !== undefined && newMin > maxPages) throw new Error('script does not fit in the runtime memory limit')
  if (newMin > 1024) throw new Error('script too large: initial memory would exceed 64 MiB')

  // Patch ScriptRef and append the new segment.
  const newData = concat([
    uleb(count + 1),
    data.body.subarray(countLen),
    [0x00, 0x41, ...sleb(place | 0), 0x0b],
    uleb(blob.length),
    blob,
  ])
  const shift = uleb(count + 1).length - countLen
  const pdv = new DataView(newData.buffer)
  pdv.setUint32(patchAt + shift, place, true)
  pdv.setUint32(patchAt + shift + 4, blob.length, true)
  data.body = newData

  mem.body = concat([[1, flags], uleb(newMin), maxPages !== undefined ? uleb(maxPages) : []])

  const dc = sections.find((s) => s.id === 12)
  if (dc) dc.body = new Uint8Array(uleb(count + 1))

  return concat([runtime.subarray(0, 8), ...sections.flatMap((s) => [[s.id], uleb(s.body.length), s.body])])
}
