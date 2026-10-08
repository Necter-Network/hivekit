import { spawnSync } from 'child_process'
import * as fs from 'fs'
import * as path from 'path'
import { beforeAll, describe, expect, it } from 'vitest'
import { compileFile } from '../src/compiler'
import { compileAssemblyScript, prepareAsSource, UnsupportedFeatureError } from '../src/compile-as'
import { readHbc } from '../src/hbc'
import { validateAbi } from '../src/abi'
import { keccak256 } from '../src/canonical'
import { LocalNode } from '../src/local'
import { NDSR, examples, inspect, install, run, tmpdir } from './helpers'

describe('AssemblyScript target: source checks', () => {
  const bad: Array<[string, RegExp]> = [
    ['hive.define("a", async (i: string): string => i)', /async/],
    ['let x: any = 1', /'any'/],
    ['let x = undefined', /undefined/],
    ['try { f() } catch (e) {}', /try\/catch/],
    ['const o = JSON.parse("{}")', /JSON/],
    ["import { x } from 'lodash'", /only `import/],
    ['const t = Date.now()', /clocks and randomness/],
    ['const r = Math.random()', /clocks and randomness/],
    ['function __hive_x(): void {}', /reserved/],
  ]
  for (const [src, re] of bad) {
    it(`rejects: ${src}`, () => {
      expect(() => prepareAsSource(src, 'm.ts')).toThrow(UnsupportedFeatureError)
      expect(() => prepareAsSource(src, 'm.ts')).toThrow(re)
    })
  }

  it('reports the line number and suggests the JS target', () => {
    try {
      prepareAsSource('// ok\nconst a = 1\nlet b: any = 2\n', 'm.ts')
      expect.unreachable()
    } catch (e) {
      expect((e as Error).message).toContain('m.ts:3')
      expect((e as Error).message).toContain('--target js')
    }
  })

  it('ignores comments and strings; strips hivekit imports', () => {
    const src = "import { hive, storage } from 'hivekit'\n// async any undefined\nconst s = \"JSON.parse any\"\nhive.define('x', (i: string): string => i)\n"
    const out = prepareAsSource(src, 'm.ts')
    expect(out.split('\n')).toHaveLength(src.split('\n').length)
    expect(out).not.toContain('import')
  })

  it('maps asc errors to user lines', () => {
    expect(() => compileAssemblyScript('hive.define("a", (i: string): string => i)\nconst x: i32 = nope;\n', { name: 'bad', file: 'bad.ts' })).toThrow(
      /bad\.ts\(2,/,
    )
  })

  it('requires literal names', () => {
    expect(() => compileAssemblyScript('const n = "a"; hive.define(n, (i: string): string => i)', { name: 'x' })).toThrow(/string literal/)
  })
})

describe('AssemblyScript target: examples/counter.ts', () => {
  let dir: string
  let hbcPath: string
  let address: string
  let hbc: Uint8Array

  beforeAll(() => {
    dir = tmpdir()
    const r = compileFile(path.join(examples, 'counter.ts'), { outDir: dir })
    hbcPath = r.hbcPath
    address = r.manifestAddress
    hbc = r.hbc
  })

  it('produces a conformant artifact', () => {
    const art = readHbc(hbc)
    expect(art.manifestAddress).toBe(address)
    expect(art.manifest).toEqual({
      compiler: 'hivekit-js/1.0.0+assemblyscript@0.28.20',
      functions: ['fingerprint', 'get', 'increment', 'relay'],
      language: 'assemblyscript',
      manifest_address: address,
      name: 'counter',
      runtime: 'hive-wasm-v1',
    })
    validateAbi(art.wasm)
  })

  it('is reproducible', () => {
    const again = compileFile(path.join(examples, 'counter.ts'), { outDir: tmpdir() })
    expect(again.manifestAddress).toBe(address)
    expect(Buffer.compare(Buffer.from(again.hbc), Buffer.from(hbc))).toBe(0)
  })

  it('runs in the local host', () => {
    const node = new LocalNode()
    const a = node.load(hbc)
    expect(node.execute(a, 'increment', '2').output).toBe('2')
    const r = node.execute(a, 'increment', '3')
    expect(r.output).toBe('5')
    expect(r.events).toEqual([{ name: 'incremented', data: { by: 3, count: 5 } }])
    const bad = node.execute(a, 'increment', '-1')
    expect(bad.success).toBe(false)
    expect(bad.error).toContain('increment must be positive')
    expect(node.execute(a, 'get', '').output).toBe('5')
    expect(node.execute(a, 'fingerprint', 'abc').output).toBe(keccak256('abc'))
  })

  describe.skipIf(!NDSR)('under ndsr', () => {
    it('passes ndsr inspect', () => {
      const r = inspect(hbcPath)
      expect(r.abi_valid).toBe(true)
      expect(r.manifest_address).toBe(address)
      expect(r.functions.map((f: { name: string }) => f.name)).toEqual(['fingerprint', 'get', 'increment', 'relay'])
    })

    it('persists storage, emits events, reverts on failure', () => {
      const data = tmpdir()
      const a = run(hbcPath, 'increment', '2', data)
      expect(a.success).toBe(true)
      expect(a.output).toBe('2')
      expect(a.events).toEqual([{ name: 'incremented', data: { by: 2, count: 2 } }])
      expect(run(hbcPath, 'increment', '', data).output).toBe('3')
      const bad = run(hbcPath, 'increment', '-4', data)
      expect(bad.success).toBe(false)
      expect(bad.error).toContain('increment must be positive, got -4')
      expect(bad.events).toEqual([])
      expect(run(hbcPath, 'get', '', data).output).toBe('3')
    })

    it('hive.call to itself and error codes', () => {
      const ok = run(hbcPath, 'relay', `${address}|fingerprint|abc`)
      expect(ok.success).toBe(true)
      expect(ok.output).toBe(keccak256('abc'))
      expect(ok.events).toEqual([{ name: 'relayed', data: { function: 'fingerprint' } }])
      const missing = run(hbcPath, 'relay', `${address}|nope|x`)
      expect(missing.success).toBe(false)
      expect(missing.error).toContain('code -2')
      const malformed = run(hbcPath, 'relay', `0x12|get|`)
      expect(malformed.error).toContain('code -6')
    })

    it('callee state changes are merged on success', () => {
      const data = tmpdir()
      install(data, address, hbc)
      const r = run(hbcPath, 'relay', `${address}|increment|7`, data)
      expect(r.success).toBe(true)
      expect(r.output).toBe('7')
      expect(r.events.map((e) => e.name)).toEqual(['incremented', 'relayed'])
      expect(run(hbcPath, 'get', '', data).output).toBe('7')
    })

    it('module.wasm is byte-identical to `ndsr compile` for the same source', () => {
      const d = tmpdir()
      const src = fs.readFileSync(path.join(examples, 'counter.ts'), 'utf8').replace(/^import .*$/m, '')
      fs.writeFileSync(path.join(d, 'c.ts'), src)
      const r = spawnSync(NDSR as string, ['compile', path.join(d, 'c.ts'), '-o', path.join(d, 'c.hbc')], { encoding: 'utf8' })
      expect(r.status, r.stderr).toBe(0)
      const ours = compileAssemblyScript(src, { name: 'c' })
      const theirs = readHbc(new Uint8Array(fs.readFileSync(path.join(d, 'c.hbc'))))
      expect(Buffer.compare(Buffer.from(ours.wasm), Buffer.from(theirs.wasm))).toBe(0)
    })
  })
})
