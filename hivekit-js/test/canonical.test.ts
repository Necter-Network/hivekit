import { describe, expect, it } from 'vitest'
import vectors from './fixtures/test-vectors.json'
import { canonicalJson, keccak256 } from '../src/canonical'
import { manifestAddress, packageHbc, readHbc, validateManifest, buildManifest } from '../src/hbc'

describe('test-vectors.json', () => {
  it('canonical_json', () => {
    for (const v of vectors.canonical_json) expect(canonicalJson(v.input)).toBe(v.canonical)
  })

  it('keccak256 (not SHA3-256)', () => {
    for (const v of vectors.keccak256) expect(keccak256(v.input_utf8)).toBe(v.hash)
  })

  it('manifest_address', () => {
    const v = vectors.manifest_address
    expect(canonicalJson(v.manifest)).toBe(v.canonical_manifest)
    const wasm = Uint8Array.from(Buffer.from(v.wasm_hex, 'hex'))
    expect(manifestAddress(v.manifest, wasm)).toBe(v.manifest_address)
    // manifest_address itself is excluded from the hash
    expect(manifestAddress({ ...v.manifest, manifest_address: '0xdead' }, wasm)).toBe(v.manifest_address)
  })

  it('reads the vector .hbc and verifies its address', () => {
    const v = vectors.manifest_address
    const art = readHbc(Uint8Array.from(Buffer.from(v.hbc_base64, 'base64')))
    expect(art.manifestAddress).toBe(v.manifest_address)
    expect(Buffer.from(art.wasm).toString('hex')).toBe(v.wasm_hex)
  })

  it('packaging the vector reproduces its address', () => {
    const v = vectors.manifest_address
    const wasm = Uint8Array.from(Buffer.from(v.wasm_hex, 'hex'))
    const p = packageHbc(buildManifest({ ...v.manifest, functions: v.manifest.functions }), wasm)
    expect(p.manifestAddress).toBe(v.manifest_address)
  })

  it('event and receipt canonical JSON vectors', () => {
    const r = vectors.execution_receipt
    expect(canonicalJson(r.events)).toBe(r.events_canonical_json)
    const { receipt } = r.signed_envelope
    const consensus = {
      v: receipt.v,
      module_address: receipt.module_address,
      function: receipt.function,
      input_hash: receipt.input_hash,
      output_hash: receipt.output_hash,
      events_hash: receipt.events_hash,
      gas_used: receipt.gas_used,
      success: receipt.success,
    }
    expect(canonicalJson(consensus)).toBe(r.consensus_canonical_json)
    expect(keccak256(r.consensus_canonical_json)).toBe(r.receipt_hash)
    expect(keccak256(r.events_canonical_json)).toBe(receipt.events_hash)
  })
})

describe('canonicalJson', () => {
  it('sorts nested keys (regression: a replacer array dropped them)', () => {
    expect(canonicalJson({ b: { z: 1, a: { y: 2, x: [{ d: 1, c: 2 }] } }, a: 0 })).toBe(
      '{"a":0,"b":{"a":{"x":[{"c":2,"d":1}],"y":2},"z":1}}',
    )
  })
  it('does not escape non-ASCII', () => {
    expect(canonicalJson({ s: 'héllo ✓ 𝄞' })).toBe('{"s":"héllo ✓ 𝄞"}')
  })
  it('orders keys by code point', () => {
    expect(canonicalJson({ '𝄞': 1, '￿': 2, a: 3 })).toBe('{"a":3,"￿":2,"𝄞":1}')
  })
  it('rejects floats and unsafe integers', () => {
    expect(() => canonicalJson({ x: 1.5 })).toThrow(/float/)
    expect(() => canonicalJson([2 ** 53])).toThrow(/range/)
    expect(() => canonicalJson({ x: undefined })).toThrow(/undefined/)
  })
})

describe('manifest schema', () => {
  const base = { name: 'm', language: 'javascript', compiler: 'x/1', runtime: 'hive-wasm-v1', functions: ['a', 'b'] }
  it('accepts a valid manifest', () => expect(() => validateManifest({ ...base })).not.toThrow())
  it('rejects extra keys', () => {
    for (const k of ['created_at', 'nrc1', 'consensus', 'wasm_ready', 'schedules', 'config']) {
      expect(() => validateManifest({ ...base, [k]: 1 })).toThrow(/not allowed/)
    }
  })
  it('rejects unsorted or duplicate functions', () => {
    expect(() => validateManifest({ ...base, functions: ['b', 'a'] })).toThrow(/sorted/)
    expect(() => validateManifest({ ...base, functions: ['a', 'a'] })).toThrow(/sorted/)
  })
  it('rejects a wrong runtime', () => expect(() => validateManifest({ ...base, runtime: 'wasm32-wasi' })).toThrow(/runtime/))
  it('buildManifest sorts by byte value', () => {
    expect(buildManifest({ name: 'm', language: 'js', compiler: 'c', functions: ['b', 'B', '_a', 'a'] }).functions).toEqual(['B', '_a', 'a', 'b'])
  })
})
