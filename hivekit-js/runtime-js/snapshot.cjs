#!/usr/bin/env node
/**
 * Build-time pre-initialization ("snapshot") of a HiveKit interpreter runtime.
 *
 *   node snapshot.cjs <in.wasm> <out.wasm>
 *
 * Instantiates the runtime with host imports that trap, calls its
 * `__hive_preinit` export (which builds the interpreter and evaluates the
 * runtime prelude without touching the host), then writes a new module whose
 * data segments are the resulting linear memory, whose globals hold their
 * post-initialization values, and whose memory minimum covers the snapshot.
 * `__hive_preinit` is removed from the exports. Every call on a node then
 * starts from the initialized state instead of paying for engine start-up.
 *
 * Used by runtime-js/build.sh and hivekit/runtime-py/build.sh.
 */
'use strict'
const fs = require('fs')

const PAGE = 65536
const MAX_GAP = 16 // zero bytes tolerated inside one data segment

function uleb(n) {
  const out = []
  do {
    let b = n % 128
    n = Math.floor(n / 128)
    if (n !== 0) b |= 0x80
    out.push(b)
  } while (n !== 0)
  return out
}

function sleb(n) {
  // n: BigInt
  const out = []
  for (;;) {
    const b = Number(n & 0x7fn)
    n >>= 7n
    if ((n === 0n && (b & 0x40) === 0) || (n === -1n && (b & 0x40) !== 0)) {
      out.push(b)
      return out
    }
    out.push(b | 0x80)
  }
}

function readU(b, p) {
  let r = 0
  let s = 0
  for (;;) {
    const x = b[p++]
    r += (x & 0x7f) * 2 ** s
    if (!(x & 0x80)) return [r, p]
    s += 7
  }
}

function parseSections(wasm) {
  const sections = []
  let p = 8
  while (p < wasm.length) {
    const id = wasm[p]
    const [size, q] = readU(wasm, p + 1)
    sections.push({ id, body: wasm.subarray(q, q + size) })
    p = q + size
  }
  return sections
}

function emit(header, sections) {
  const parts = [header]
  for (const s of sections) parts.push(Buffer.from([s.id, ...uleb(s.body.length)]), Buffer.from(s.body))
  return Buffer.concat(parts)
}

/** Skip a constant expression; returns the position after `end`. */
function skipConstExpr(b, p) {
  for (;;) {
    const op = b[p++]
    if (op === 0x0b) return p
    if (op === 0x41 || op === 0x42 || op === 0x23 || op === 0xd2) {
      while (b[p++] & 0x80);
    } else if (op === 0x43) p += 4
    else if (op === 0x44) p += 8
    else if (op === 0xd0) p += 1
    else throw new Error(`unsupported const expr opcode 0x${op.toString(16)}`)
  }
}

function main() {
  const [input, output] = process.argv.slice(2)
  if (!input || !output) {
    console.error('usage: node snapshot.cjs <in.wasm> <out.wasm>')
    process.exit(2)
  }
  const wasm = fs.readFileSync(input)
  const header = wasm.subarray(0, 8)
  const sections = parseSections(wasm)
  const byId = (id) => sections.find((s) => s.id === id)

  // Globals: all defined (no imported globals in these runtimes).
  const globals = []
  const gsec = byId(6)
  if (gsec) {
    let [n, p] = readU(gsec.body, 0)
    for (let i = 0; i < n; i++) {
      const type = gsec.body[p]
      const mutable = gsec.body[p + 1]
      const start = p
      p = skipConstExpr(gsec.body, p + 2)
      globals.push({ type, mutable, raw: gsec.body.subarray(start, p) })
    }
  }
  for (const imp of WebAssembly.Module.imports(new WebAssembly.Module(wasm))) {
    if (imp.kind !== 'function') throw new Error(`unexpected ${imp.kind} import ${imp.module}.${imp.name}`)
  }

  // Instrumented copy: export every global so its final value can be read.
  const esec = byId(7)
  let [ecount, ep] = readU(esec.body, 0)
  const extra = []
  globals.forEach((g, i) => {
    const name = Buffer.from(`__snapshot_global_${i}`)
    extra.push(...uleb(name.length), ...name, 0x03, ...uleb(i))
  })
  const instrumented = emit(
    header,
    sections.map((s) =>
      s.id === 7 ? { id: 7, body: Buffer.concat([Buffer.from(uleb(ecount + globals.length)), s.body.subarray(ep), Buffer.from(extra)]) } : s,
    ),
  )

  const module = new WebAssembly.Module(instrumented)
  const imports = {}
  for (const imp of WebAssembly.Module.imports(module)) {
    imports[imp.module] ??= {}
    imports[imp.module][imp.name] = () => {
      throw new Error(`host function ${imp.module}.${imp.name} called during pre-initialization`)
    }
  }
  const inst = new WebAssembly.Instance(module, imports)
  if (typeof inst.exports.__hive_preinit !== 'function') throw new Error('runtime has no __hive_preinit export')
  inst.exports.__hive_preinit()
  const mem = new Uint8Array(inst.exports.memory.buffer)

  // New global section with post-initialization values.
  const gparts = [Buffer.from(uleb(globals.length))]
  globals.forEach((g, i) => {
    if (!g.mutable) return gparts.push(Buffer.from(g.raw))
    const v = inst.exports[`__snapshot_global_${i}`].value
    let init
    if (g.type === 0x7f) init = [0x41, ...sleb(BigInt(v))]
    else if (g.type === 0x7e) init = [0x42, ...sleb(BigInt.asIntN(64, v))]
    else if (g.type === 0x7d) init = [0x43, ...new Uint8Array(new Float32Array([v]).buffer)]
    else if (g.type === 0x7c) init = [0x44, ...new Uint8Array(new Float64Array([v]).buffer)]
    else throw new Error(`unsupported global type 0x${g.type.toString(16)}`)
    gparts.push(Buffer.from([g.type, g.mutable, ...init, 0x0b]))
  })

  // Data segments covering the non-zero bytes of memory.
  const dsec = byId(11)
  if (dsec) {
    let [n, p] = readU(dsec.body, 0)
    for (let i = 0; i < n; i++) {
      const [kind] = readU(dsec.body, p)
      if (kind !== 0) throw new Error('passive or multi-memory data segments are not supported')
      p = skipConstExpr(dsec.body, p + 1)
      const [len, q] = readU(dsec.body, p)
      p = q + len
    }
  }
  const segs = []
  let i = 0
  while (i < mem.length) {
    while (i < mem.length && mem[i] === 0) i++
    if (i >= mem.length) break
    const start = i
    let end = i
    let zeros = 0
    while (i < mem.length) {
      if (mem[i] === 0) {
        if (++zeros > MAX_GAP) break
      } else {
        zeros = 0
        end = i + 1
      }
      i++
    }
    segs.push([start, end])
    i = end
  }
  const dparts = [Buffer.from(uleb(segs.length))]
  for (const [s, e] of segs) {
    dparts.push(Buffer.from([0x00, 0x41, ...sleb(BigInt(s | 0)), 0x0b, ...uleb(e - s)]), Buffer.from(mem.subarray(s, e)))
  }

  // Memory section: same flags/max, minimum = current size.
  const msec = byId(5)
  const [mcount, mp] = readU(msec.body, 0)
  if (mcount !== 1) throw new Error('expected exactly one memory')
  const flags = msec.body[mp]
  const [, mq] = readU(msec.body, mp + 1)
  const maxPart = flags & 1 ? msec.body.subarray(mq) : Buffer.alloc(0)
  const pages = mem.length / PAGE

  // Exports without __hive_preinit.
  const kept = []
  let p = ep
  for (let k = 0; k < ecount; k++) {
    const start = p
    const [nlen, q] = readU(esec.body, p)
    const name = Buffer.from(esec.body.subarray(q, q + nlen)).toString()
    p = q + nlen + 1
    ;[, p] = readU(esec.body, p)
    if (name !== '__hive_preinit') kept.push(esec.body.subarray(start, p))
  }

  const out = []
  let wroteData = false
  for (const s of sections) {
    if (s.id === 5) out.push({ id: 5, body: Buffer.concat([Buffer.from([1, flags, ...uleb(pages)]), maxPart]) })
    else if (s.id === 6) out.push({ id: 6, body: Buffer.concat(gparts) })
    else if (s.id === 7) out.push({ id: 7, body: Buffer.concat([Buffer.from(uleb(kept.length)), ...kept]) })
    else if (s.id === 12) out.push({ id: 12, body: Buffer.from(uleb(segs.length)) })
    else if (s.id === 11) {
      out.push({ id: 11, body: Buffer.concat(dparts) })
      wroteData = true
    } else {
      if (s.id === 0 && !wroteData && sections.indexOf(s) > sections.findIndex((x) => x.id === 10)) {
        out.push({ id: 11, body: Buffer.concat(dparts) })
        wroteData = true
      }
      out.push(s)
    }
  }
  if (!wroteData) out.push({ id: 11, body: Buffer.concat(dparts) })
  const result = emit(header, out)
  new WebAssembly.Module(result) // validate
  fs.writeFileSync(output, result)
  console.log(`snapshot: ${pages} pages, ${segs.length} data segments, ${wasm.length} -> ${result.length} bytes`)
}

main()
