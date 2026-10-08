import * as fs from 'fs'
import * as path from 'path'
import { beforeAll, describe, expect, it } from 'vitest'
import { compile, compileFile } from '../src/compiler'
import { loadJsRuntime } from '../src/compile-js'
import { embedScript, scriptBlob, SCRIPT_MAGIC } from '../src/embed'
import { readHbc, MAX_WASM_BYTES } from '../src/hbc'
import { validateAbi } from '../src/abi'
import { keccak256 } from '../src/canonical'
import { LocalNode } from '../src/local'
import { NDSR, examples, inspect, install, run, tmpdir } from './helpers'

const parse = (s: string): unknown => JSON.parse(s)

describe('script embedding', () => {
  const runtime = loadJsRuntime()

  it('the prebuilt runtime is import-clean and within the size limit', () => {
    const r = validateAbi(runtime)
    expect(r.imports.every((i) => !i.startsWith('wasi'))).toBe(true)
    expect(runtime.length).toBeLessThan(MAX_WASM_BYTES)
  })

  it('is deterministic and patches ScriptRef', () => {
    const blob = scriptBlob(['a', 'b'], 'hive.define("a", () => 1)')
    const a = embedScript(runtime, blob)
    const b = embedScript(runtime, blob)
    expect(Buffer.compare(Buffer.from(a), Buffer.from(b))).toBe(0)
    validateAbi(a)
    const buf = Buffer.from(a)
    const at = buf.indexOf(Buffer.from(SCRIPT_MAGIC))
    const ptr = buf.readUInt32LE(at + 16)
    const len = buf.readUInt32LE(at + 20)
    expect(len).toBe(blob.length)
    expect(ptr % 65536).toBe(0)
    expect(buf.subarray(buf.length - blob.length).equals(Buffer.from(blob))).toBe(true)
  })

  it('refuses to embed twice', () => {
    const once = embedScript(runtime, scriptBlob(['a'], 'x'))
    expect(() => embedScript(once, scriptBlob(['a'], 'y'))).toThrow(/already/)
  })
})

describe('JavaScript engine target: examples/counter.js', () => {
  let dir: string
  let hbcPath: string
  let address: string
  let hbc: Uint8Array

  beforeAll(() => {
    dir = tmpdir()
    const r = compileFile(path.join(examples, 'counter.js'), { outDir: dir })
    hbcPath = r.hbcPath
    address = r.manifestAddress
    hbc = r.hbc
  })

  it('produces a conformant artifact', () => {
    const art = readHbc(hbc)
    expect(art.manifest).toEqual({
      compiler: 'hivekit-js/1.0.0+boa@0.22.0',
      functions: ['get', 'increment', 'note', 'relay'],
      language: 'javascript',
      manifest_address: address,
      name: 'counter',
      runtime: 'hive-wasm-v1',
    })
    expect(art.wasm.length).toBeLessThan(MAX_WASM_BYTES)
  })

  it('runs in the local host', () => {
    const node = new LocalNode()
    const a = node.load(hbc)
    const r = node.execute(a, 'increment', '{"by":2}')
    expect(r.error).toBeNull()
    expect(parse(r.output)).toEqual({ count: 2 })
    expect(r.events).toEqual([{ name: 'incremented', data: { by: 2, count: 2 } }])
  })

  describe.skipIf(!NDSR)('under ndsr', () => {
    it('passes ndsr inspect', () => {
      const r = inspect(hbcPath)
      expect(r.abi_valid).toBe(true)
      expect(r.abi_error).toBeNull()
      expect(r.manifest_address).toBe(address)
    })

    it('persists storage, emits events, reverts on failure', () => {
      const data = tmpdir()
      const a = run(hbcPath, 'increment', '{"by":2}', data)
      expect(a.error).toBeNull()
      expect(parse(a.output)).toEqual({ count: 2 })
      expect(a.events).toEqual([{ name: 'incremented', data: { by: 2, count: 2 } }])
      const b = run(hbcPath, 'increment', '', data)
      expect(parse(b.output)).toEqual({ count: 3 })
      // The pre-initialized engine keeps calls well within the CCS default gas cap (50M).
      expect(b.gasUsed).toBeLessThan(50_000_000)
      const bad = run(hbcPath, 'increment', '{"by":-1}', data)
      expect(bad.success).toBe(false)
      expect(bad.error).toContain('Error: increment must be a positive integer')
      expect(parse(run(hbcPath, 'get', '', data).output)).toEqual({ count: 3 })
    })

    it('ctx-style handlers and hive.hash', () => {
      const r = run(hbcPath, 'note', '{"text":"héllo"}')
      expect(r.error).toBeNull()
      expect(parse(r.output)).toEqual({ saved: true, hash: keccak256('héllo') })
    })

    it('hive.call into an AssemblyScript module and back', () => {
      const data = tmpdir()
      const as = compileFile(path.join(examples, 'counter.ts'), { outDir: tmpdir() })
      install(data, as.manifestAddress, as.hbc)
      const r = run(hbcPath, 'relay', JSON.stringify({ address: as.manifestAddress, function: 'increment', input: '5' }), data)
      expect(r.error).toBeNull()
      expect(parse(r.output)).toEqual({ relayed: 5 })
      expect(r.events).toEqual([
        { name: 'incremented', data: { by: 5, count: 5 } },
        { name: 'relayed', data: { function: 'increment' } },
      ])
      // The AS module's state was committed under its own address.
      expect(run(as.hbcPath, 'get', '', data).output).toBe('5')
      // Unknown module → hive.call error surfaces as a failed call.
      const missing = run(hbcPath, 'relay', JSON.stringify({ address: '0x' + '1'.repeat(64), function: 'get', input: '' }), data)
      expect(missing.success).toBe(false)
      expect(missing.error).toContain('module not found (code -1)')
    })

    it('is deterministic: clocks and randomness are unavailable', () => {
      const src = `
        const { hive } = require('hivekit')
        hive.define('rnd', () => Math.random())
        hive.define('now', () => Date.now())
        hive.define('date', () => new Date())
        hive.define('fixed', () => new Date(0).toISOString())
        hive.define('nested', () => ({ b: [1, 'é'], a: null }))
        hive.define('empty', () => undefined)
        hive.define('text', (input) => typeof input === 'string' ? 'raw:' + input : 'json')
      `
      const d = tmpdir()
      const p = path.join(d, 'det.hbc')
      fs.writeFileSync(p, compile(src, 'det.js').hbc)
      expect(run(p, 'rnd', '').error).toContain('Math.random() is not available')
      expect(run(p, 'now', '').error).toContain('Date.now() is not available')
      expect(run(p, 'date', '').error).toContain('without arguments is not available')
      expect(run(p, 'fixed', '').output).toBe('1970-01-01T00:00:00.000Z')
      expect(run(p, 'nested', '').output).toBe('{"b":[1,"é"],"a":null}')
      expect(run(p, 'empty', '').output).toBe('')
      expect(run(p, 'text', 'not json').output).toBe('raw:not json')
      expect(run(p, 'text', '{"a":1}').output).toBe('json')
      const a = run(p, 'nested', '')
      const b = run(p, 'nested', '')
      expect(a.gasUsed).toBe(b.gasUsed)
    })

    it('reports script errors and require() of other modules clearly', () => {
      const d = tmpdir()
      const p = path.join(d, 'req.hbc')
      fs.writeFileSync(p, compile("const _ = require('lodash'); require('hivekit').hive.define('a', () => 1)", 'req.js').hbc)
      expect(run(p, 'a', '').error).toContain("require('lodash'): only 'hivekit' is available")
    })
  })
})

describe('JavaScript engine target: full TypeScript (examples/profile.ts --target js)', () => {
  it.skipIf(!NDSR)('classes, generics, async handlers, spread', () => {
    const d = tmpdir()
    const r = compileFile(path.join(examples, 'profile.ts'), { outDir: d, target: 'js' })
    expect(r.manifest.language).toBe('typescript')
    expect(inspect(r.hbcPath).abi_valid).toBe(true)
    const data = tmpdir()
    const saved = run(r.hbcPath, 'saveProfile', JSON.stringify({ id: 'u1', profile: { name: 'Ada', tags: ['b', 'a', 'b'] } }), data)
    expect(saved.error).toBeNull()
    expect(parse(saved.output)).toEqual({ name: 'Ada', tags: ['a', 'b'] })
    expect(saved.events).toEqual([{ name: 'profile_saved', data: { id: 'u1', tags: 2 } }])
    expect(parse(run(r.hbcPath, 'getProfile', '{"id":"u1"}', data).output)).toEqual({ name: 'Ada', tags: ['a', 'b'] })
    expect(run(r.hbcPath, 'getProfile', '{"id":"nobody"}', data).error).toContain('no profile nobody')
  })

  it('reports TypeScript syntax errors with positions', () => {
    expect(() => compile('hive.define("a", () => {', 'bad.ts', { target: 'js' })).toThrow(/bad\.ts\(1,/)
  })
})
