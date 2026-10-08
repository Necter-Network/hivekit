import { describe, expect, it } from 'vitest'
import vectors from './fixtures/test-vectors.json'
import { MAX_HBC_BYTES, MAX_WASM_BYTES, buildManifest, packageHbc, readHbc, zipStored } from '../src/hbc'
import { canonicalBytes } from '../src/canonical'

/** `wasm` plus one custom section so the result is exactly `total` bytes. */
function padTo(wasm: Uint8Array, total: number): Uint8Array {
  const size = total - wasm.length - 5 // section id + 4-byte LEB size
  const out = new Uint8Array(total)
  out.set(wasm)
  out.set([0, (size & 0x7f) | 0x80, ((size >>> 7) & 0x7f) | 0x80, ((size >>> 14) & 0x7f) | 0x80, size >>> 21, 3, 0x70, 0x61, 0x64], wasm.length)
  return out
}

describe('size limits (HBC_SPEC §2)', () => {
  const base = Uint8Array.from(Buffer.from(vectors.manifest_address.wasm_hex, 'hex'))
  const manifest = buildManifest({ name: 'big', language: 'wat', compiler: 'limits-test/1', functions: ['echo'] })

  it('matches the spec', () => {
    expect(MAX_WASM_BYTES).toBe(12 * 1024 * 1024)
    expect(MAX_HBC_BYTES).toBe(16 * 1024 * 1024)
  })

  it('accepts module.wasm exactly at the limit', () => {
    const wasm = padTo(base, MAX_WASM_BYTES)
    const p = packageHbc(manifest, wasm)
    expect(p.hbc.length).toBeLessThanOrEqual(MAX_HBC_BYTES)
    const r = readHbc(p.hbc)
    expect(r.manifestAddress).toBe(p.manifestAddress)
    expect(r.wasm.length).toBe(MAX_WASM_BYTES)
  })

  it('rejects module.wasm one byte over the limit', () => {
    const wasm = padTo(base, MAX_WASM_BYTES + 1)
    expect(() => packageHbc(manifest, wasm)).toThrow(/limit/)
    const hbc = zipStored([
      ['manifest.json', canonicalBytes(manifest)],
      ['module.wasm', wasm],
    ])
    expect(() => readHbc(hbc)).toThrow(/module.wasm exceeds/)
  })
})
