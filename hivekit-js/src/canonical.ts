/**
 * Canonical JSON and Keccak-256 (docs/HBC_SPEC.md §4).
 *
 * Canonical JSON: object keys sorted by code point at every depth, no
 * whitespace, non-ASCII emitted as UTF-8 (never `\u` escaped), integers only
 * (|n| <= 2^53 - 1). Floats, NaN, Infinity, undefined, functions and symbols
 * are rejected rather than silently dropped.
 */

import { keccak256 as keccakHex } from 'js-sha3'

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json }

const MAX_SAFE = Number.MAX_SAFE_INTEGER

function compareCodePoints(a: string, b: string): number {
  // Code-point order (not UTF-16 unit order), matching Rust's and Python's sort.
  const ai = a[Symbol.iterator]()
  const bi = b[Symbol.iterator]()
  for (;;) {
    const x = ai.next()
    const y = bi.next()
    if (x.done) return y.done ? 0 : -1
    if (y.done) return 1
    const cx = x.value.codePointAt(0) as number
    const cy = y.value.codePointAt(0) as number
    if (cx !== cy) return cx < cy ? -1 : 1
  }
}

function encode(value: unknown, path: string): string {
  if (value === null) return 'null'
  switch (typeof value) {
    case 'boolean':
      return value ? 'true' : 'false'
    case 'number':
      if (!Number.isInteger(value)) throw new TypeError(`canonical JSON: float not allowed at ${path}`)
      if (Math.abs(value) > MAX_SAFE) throw new TypeError(`canonical JSON: integer out of range at ${path}`)
      return Object.is(value, -0) ? '0' : String(value)
    case 'bigint':
      if (value > BigInt(MAX_SAFE) || value < -BigInt(MAX_SAFE)) {
        throw new TypeError(`canonical JSON: integer out of range at ${path}`)
      }
      return value.toString()
    case 'string':
      // JSON.stringify escapes only `"`, `\\` and control characters (and lone
      // surrogates); everything else is emitted verbatim.
      return JSON.stringify(value)
    case 'object': {
      if (Array.isArray(value)) {
        return '[' + value.map((v, i) => encode(v, `${path}[${i}]`)).join(',') + ']'
      }
      const obj = value as Record<string, unknown>
      const keys = Object.keys(obj).sort(compareCodePoints)
      const parts: string[] = []
      for (const k of keys) {
        const v = obj[k]
        if (v === undefined) throw new TypeError(`canonical JSON: undefined value at ${path}.${k}`)
        parts.push(JSON.stringify(k) + ':' + encode(v, `${path}.${k}`))
      }
      return '{' + parts.join(',') + '}'
    }
    default:
      throw new TypeError(`canonical JSON: unsupported ${typeof value} at ${path}`)
  }
}

/** Canonical JSON text of `value`. */
export function canonicalJson(value: unknown): string {
  return encode(value, '$')
}

/** Canonical JSON as UTF-8 bytes. */
export function canonicalBytes(value: unknown): Uint8Array {
  return new TextEncoder().encode(canonicalJson(value))
}

/** Keccak-256 (original Keccak, as used by Ethereum; NOT FIPS SHA3-256) as `0x` + hex. */
export function keccak256(data: Uint8Array | string): string {
  const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data
  return '0x' + keccakHex(bytes)
}
