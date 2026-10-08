/**
 * Hive — browser/Node client for modules deployed through a CCS gateway.
 *
 * Talks to the CCS gateway API:
 *   POST {gateway}/api/execute        {module, function, input, gas_limit?}
 *   POST {gateway}/api/files/upload   multipart (operator-authenticated)
 *   GET  {gateway}/api/modules/{addr} module metadata
 *   GET  {gateway}/files/{file_id}    public download
 *
 * No identity is attached to calls: CCS does not authenticate callers of
 * `/api/execute`, and an unsigned wallet header would be a spoofable claim.
 */

import { HiveExecutionError, HiveNetworkError, HiveRequestError } from './errors.js'
import type { HiveEvent, HiveOptions, HiveResponse, ModuleAddress, ModuleInfo, UploadOptions, UploadResponse } from './types.js'

/** Canonical module address: `0x` + 64 lowercase hex. Accepts `hive:` prefixes and upper case. */
export function normalizeAddress(address: string): string {
  let a = String(address).trim()
  if (a.toLowerCase().startsWith('hive:')) a = a.slice(5)
  a = a.toLowerCase()
  if (!/^0x[0-9a-f]{64}$/.test(a)) {
    throw new TypeError(`HiveJS: invalid module address ${JSON.stringify(address)} (expected 0x + 64 hex characters)`)
  }
  return a
}

/** Normalize and validate a gateway base URL. */
export function normalizeGateway(url: string | undefined): string {
  if (!url) {
    throw new TypeError('HiveJS: gatewayUrl is required (the base URL of your CCS gateway, e.g. https://ccs.example.org)')
  }
  let u: URL
  try {
    u = new URL(url)
  } catch {
    throw new TypeError(`HiveJS: gatewayUrl ${JSON.stringify(url)} is not a valid URL`)
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') throw new TypeError('HiveJS: gatewayUrl must be http(s)')
  return u.toString().replace(/\/+$/, '')
}

function decodeOutput(output: string): unknown {
  if (output === '') return output
  try {
    return JSON.parse(output)
  } catch {
    return output
  }
}

async function errorBody(res: Response): Promise<{ message: string; body: unknown }> {
  const text = await res.text().catch(() => '')
  try {
    const body = JSON.parse(text) as { error?: unknown }
    return { message: typeof body.error === 'string' ? body.error : text, body }
  } catch {
    return { message: text || res.statusText || String(res.status), body: text }
  }
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

export class Hive {
  readonly address: string
  readonly gatewayUrl: string
  private readonly opts: HiveOptions
  private readonly fetchImpl: typeof fetch

  /**
   * @param moduleAddress the module's `manifest_address` (from `hivec build`)
   * @param opts          `gatewayUrl` is required
   */
  constructor(moduleAddress: ModuleAddress, opts: HiveOptions) {
    this.address = normalizeAddress(moduleAddress)
    this.gatewayUrl = normalizeGateway(opts?.gatewayUrl)
    this.opts = opts
    const f = opts.fetch ?? (typeof globalThis.fetch === 'function' ? globalThis.fetch.bind(globalThis) : undefined)
    if (!f) throw new TypeError('HiveJS: no fetch implementation available; pass opts.fetch')
    this.fetchImpl = f
  }

  /**
   * Call `functionName` with `input`. Objects are sent as JSON (the node passes
   * them to the module as canonical JSON; floats are rejected); strings are
   * passed through byte-for-byte.
   *
   * Never retried automatically. On a network error or timeout the call may or
   * may not have executed.
   */
  async call<T = unknown>(functionName: string, input: unknown = {}, opts: { gasLimit?: number } = {}): Promise<HiveResponse<T>> {
    if (typeof functionName !== 'string' || !functionName) throw new TypeError('HiveJS: function name must be a non-empty string')
    const gas = opts.gasLimit ?? this.opts.gasLimit
    const body: Record<string, unknown> = { module: this.address, function: functionName, input }
    if (gas !== undefined) body.gas_limit = gas
    const res = await this.request('/api/execute', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })
    if (!res.ok) {
      const { message, body: errBody } = await errorBody(res)
      // A node that executed and failed still returns its receipt.
      if (errBody && typeof errBody === 'object' && 'receipt' in errBody) {
        const b = errBody as { error?: string; receipt: unknown; gas_used?: number }
        throw new HiveExecutionError(`HiveJS: execution failed: ${b.error ?? message}`, b.receipt, b.gas_used ?? 0)
      }
      if (res.status >= 500) throw new HiveNetworkError(`HiveJS: gateway error ${res.status}: ${message}`, res.status)
      throw new HiveRequestError(`HiveJS: request rejected (${res.status}): ${message}`, res.status, errBody)
    }
    const raw = (await res.json()) as {
      success?: boolean
      output?: string
      receipt_hash: string
      gas_used: number
      gas_limit: number
      events?: HiveEvent[]
      node_id: string
      timestamp: number
      receipt: unknown
      error?: string | null
    }
    if (raw.success === false || (raw.error !== undefined && raw.error !== null)) {
      throw new HiveExecutionError(`HiveJS: execution failed: ${raw.error ?? 'unknown error'}`, raw.receipt, raw.gas_used)
    }
    const output = raw.output ?? ''
    return {
      data: decodeOutput(output) as T,
      output,
      receiptHash: raw.receipt_hash,
      gasUsed: raw.gas_used,
      gasLimit: raw.gas_limit,
      events: raw.events ?? [],
      nodeId: raw.node_id,
      timestamp: raw.timestamp,
      receipt: raw.receipt,
    }
  }

  /** `PostData({ action: 'fn', ...input })` is `call('fn', input)`. */
  async PostData<T = unknown>(data: Record<string, unknown>): Promise<HiveResponse<T>> {
    const { action, ...input } = data ?? {}
    if (typeof action !== 'string' || !action) {
      throw new TypeError('HiveJS: PostData() needs an "action" field naming a hive.define() function')
    }
    return this.call<T>(action, input)
  }

  /** Module metadata from the gateway registry (GET; retried on transient errors). */
  async getModule(): Promise<ModuleInfo> {
    const res = await this.getWithRetry(`/api/modules/${this.address}`)
    return (await res.json()) as ModuleInfo
  }

  /**
   * Upload a file to the CCS file store. CCS only accepts uploads from an
   * authenticated operator: either a same-origin operator session (pass
   * `credentials: 'include'` and an `X-CSRF-Token` header in the client
   * options) or a bearer token on a trusted server. Never ship an operator
   * token to browsers.
   */
  async upload(file: Blob, opts: UploadOptions & { authToken?: string } = {}): Promise<UploadResponse> {
    const form = new FormData()
    const fileName = opts.name ?? (typeof File !== 'undefined' && file instanceof File ? file.name : 'upload')
    form.append('file', file, fileName)
    form.append('name', fileName)
    if (opts.type) form.append('type', opts.type)
    if (opts.metadata) form.append('metadata', JSON.stringify(opts.metadata))
    const headers: Record<string, string> = {}
    if (opts.authToken) headers.Authorization = `Bearer ${opts.authToken}`
    const res = await this.request('/api/files/upload', { method: 'POST', body: form, headers })
    if (!res.ok) {
      const { message, body } = await errorBody(res)
      if (res.status >= 500) throw new HiveNetworkError(`HiveJS: upload failed (${res.status}): ${message}`, res.status)
      throw new HiveRequestError(`HiveJS: upload rejected (${res.status}): ${message}`, res.status, body)
    }
    const m = (await res.json()) as Omit<UploadResponse, 'url'>
    return { ...m, url: this.fileUrl(m.file_id) }
  }

  /** Public download URL for an uploaded file. */
  fileUrl(fileId: string): string {
    if (!/^[0-9a-f]{64}$/.test(fileId)) throw new TypeError('HiveJS: file id must be 64 lowercase hex characters')
    return `${this.gatewayUrl}/files/${fileId}`
  }

  private async request(path: string, init: RequestInit): Promise<Response> {
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), this.opts.timeout ?? 30_000)
    try {
      return await this.fetchImpl(this.gatewayUrl + path, {
        ...init,
        headers: { ...(this.opts.headers ?? {}), ...((init.headers as Record<string, string>) ?? {}) },
        credentials: this.opts.credentials,
        signal: controller.signal,
      })
    } catch (e) {
      const msg = (e as Error)?.name === 'AbortError' ? 'request timed out' : (e as Error)?.message ?? String(e)
      throw new HiveNetworkError(`HiveJS: ${msg}`)
    } finally {
      clearTimeout(timer)
    }
  }

  private async getWithRetry(path: string): Promise<Response> {
    const retries = this.opts.retries ?? 2
    for (let attempt = 0; ; attempt++) {
      try {
        const res = await this.request(path, { method: 'GET' })
        if (res.ok) return res
        const { message, body } = await errorBody(res)
        if (res.status < 500) throw new HiveRequestError(`HiveJS: request rejected (${res.status}): ${message}`, res.status, body)
        throw new HiveNetworkError(`HiveJS: gateway error ${res.status}: ${message}`, res.status)
      } catch (e) {
        if (!(e instanceof HiveNetworkError) || attempt >= retries) throw e
        await sleep(250 * 2 ** attempt)
      }
    }
  }
}
