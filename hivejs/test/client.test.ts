import { describe, expect, it, vi } from 'vitest'
import { Hive, HiveExecutionError, HiveNetworkError, HiveRequestError, normalizeAddress } from '../src/index'

const ADDR = '0x8a9321e60b20d30e14ebb65002b6cec307fdf8a93b9f6e23279c6a1b8ee1b454'
const GW = 'https://ccs.example.test'

// Shape returned by an NDSR node's POST /execute, proxied unchanged by CCS.
const okBody = {
  success: true,
  output: '{"count":3}',
  receipt_hash: '0xabc',
  gas_used: 557,
  gas_limit: 1000000,
  events: [{ name: 'incremented', data: { count: 3 } }],
  node_id: 'ndsr-9246dafcd8aa80da',
  timestamp: 1760000000,
  receipt: { receipt: { success: true } },
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
}

function mockFetch(...responses: Array<Response | Error>) {
  const fn = vi.fn(async (_url: string | URL | Request, _init?: RequestInit) => {
    const next = responses.shift()
    if (!next) throw new Error('unexpected fetch')
    if (next instanceof Error) throw next
    return next
  })
  return fn as unknown as typeof fetch & typeof fn
}

describe('construction', () => {
  it('requires a gateway URL (no hardcoded default host)', () => {
    expect(() => new Hive(ADDR, {} as never)).toThrow(/gatewayUrl is required/)
    expect(() => new Hive(ADDR, { gatewayUrl: 'not a url' })).toThrow(/not a valid URL/)
  })
  it('normalizes addresses', () => {
    expect(normalizeAddress(`hive:${ADDR.toUpperCase().replace('0X', '0x')}`)).toBe(ADDR)
    expect(() => normalizeAddress('0x1234')).toThrow(/invalid module address/)
    expect(new Hive(`hive:${ADDR}`, { gatewayUrl: `${GW}/`, fetch: mockFetch() }).gatewayUrl).toBe(GW)
  })
})

describe('call', () => {
  it('POSTs the CCS /api/execute body and maps the node response', async () => {
    const f = mockFetch(json(200, okBody))
    const hive = new Hive(ADDR, { gatewayUrl: GW, fetch: f, gasLimit: 5_000_000 })
    const res = await hive.call<{ count: number }>('increment', { by: 2 })
    expect(f).toHaveBeenCalledTimes(1)
    const [url, init] = f.mock.calls[0]
    expect(url).toBe(`${GW}/api/execute`)
    expect(init?.method).toBe('POST')
    expect(JSON.parse(init?.body as string)).toEqual({ module: ADDR, function: 'increment', input: { by: 2 }, gas_limit: 5_000_000 })
    const headers = init?.headers as Record<string, string>
    expect(Object.keys(headers).map((h) => h.toLowerCase())).not.toContain('x-wallet-address')
    expect(res.data).toEqual({ count: 3 })
    expect(res.output).toBe('{"count":3}')
    expect(res.receiptHash).toBe('0xabc')
    expect(res.gasUsed).toBe(557)
    expect(res.events).toEqual(okBody.events)
    expect(res.nodeId).toBe(okBody.node_id)
    expect(res.timestamp).toBe(1760000000)
  })

  it('omits gas_limit unless configured and passes strings through', async () => {
    const f = mockFetch(json(200, { ...okBody, output: 'plain text' }))
    const res = await new Hive(ADDR, { gatewayUrl: GW, fetch: f }).call('echo', 'raw input')
    expect(JSON.parse(f.mock.calls[0][1]?.body as string)).toEqual({ module: ADDR, function: 'echo', input: 'raw input' })
    expect(res.data).toBe('plain text')
  })

  it('PostData maps action to the function name', async () => {
    const f = mockFetch(json(200, okBody))
    await new Hive(ADDR, { gatewayUrl: GW, fetch: f }).PostData({ action: 'increment', by: 1 })
    expect(JSON.parse(f.mock.calls[0][1]?.body as string)).toMatchObject({ function: 'increment', input: { by: 1 } })
    await expect(new Hive(ADDR, { gatewayUrl: GW, fetch: f }).PostData({ by: 1 })).rejects.toThrow(/action/)
  })

  it('throws HiveExecutionError with the receipt for failed executions', async () => {
    const f = mockFetch(json(200, { ...okBody, success: false, output: '', error: 'guest abort: boom' }))
    const err = await new Hive(ADDR, { gatewayUrl: GW, fetch: f }).call('boom').catch((e) => e)
    expect(err).toBeInstanceOf(HiveExecutionError)
    expect(err.message).toContain('guest abort: boom')
    expect(err.receipt).toEqual(okBody.receipt)
  })

  it('maps CCS 4xx errors to HiveRequestError', async () => {
    const f = mockFetch(json(404, { error: 'module not found — deploy it first via POST /api/modules' }))
    const err = await new Hive(ADDR, { gatewayUrl: GW, fetch: f }).call('x').catch((e) => e)
    expect(err).toBeInstanceOf(HiveRequestError)
    expect(err.status).toBe(404)
    expect(err.message).toContain('module not found')
  })

  it('never retries POST /api/execute', async () => {
    const f = mockFetch(json(503, { error: 'no approved NDSR execution nodes available' }), json(200, okBody))
    const err = await new Hive(ADDR, { gatewayUrl: GW, fetch: f, retries: 5 }).call('x').catch((e) => e)
    expect(err).toBeInstanceOf(HiveNetworkError)
    expect(f).toHaveBeenCalledTimes(1)

    const g = mockFetch(new TypeError('fetch failed'), json(200, okBody))
    await expect(new Hive(ADDR, { gatewayUrl: GW, fetch: g, retries: 5 }).call('x')).rejects.toBeInstanceOf(HiveNetworkError)
    expect(g).toHaveBeenCalledTimes(1)
  })

  it('times out', async () => {
    const slow = vi.fn(
      (_u: string, init?: RequestInit) =>
        new Promise<Response>((_res, rej) => init?.signal?.addEventListener('abort', () => rej(Object.assign(new Error('aborted'), { name: 'AbortError' })))),
    ) as unknown as typeof fetch
    await expect(new Hive(ADDR, { gatewayUrl: GW, fetch: slow, timeout: 20 }).call('x')).rejects.toThrow(/timed out/)
  })
})

describe('getModule', () => {
  it('retries idempotent GETs on 5xx', async () => {
    const f = mockFetch(json(502, { error: 'bad gateway' }), json(200, { manifest_address: ADDR, name: 'counter', language: 'javascript', functions: ['get'] }))
    const m = await new Hive(ADDR, { gatewayUrl: GW, fetch: f, retries: 2 }).getModule()
    expect(m.name).toBe('counter')
    expect(f).toHaveBeenCalledTimes(2)
    expect(f.mock.calls[1][0]).toBe(`${GW}/api/modules/${ADDR}`)
  })
  it('does not retry 404', async () => {
    const f = mockFetch(json(404, { error: 'not found' }))
    await expect(new Hive(ADDR, { gatewayUrl: GW, fetch: f }).getModule()).rejects.toBeInstanceOf(HiveRequestError)
    expect(f).toHaveBeenCalledTimes(1)
  })
})

describe('upload', () => {
  const manifest = {
    file_id: 'a'.repeat(64),
    name: 'avatar.png',
    type: 'image/png',
    size: 3,
    chunk_size: 524288,
    chunks: 1,
    metadata: { kind: 'avatar' },
    created_at: '2026-10-07T00:00:00',
  }

  it('POSTs multipart to /api/files/upload and returns the public URL', async () => {
    const f = mockFetch(json(201, manifest))
    const hive = new Hive(ADDR, { gatewayUrl: GW, fetch: f, credentials: 'include', headers: { 'X-CSRF-Token': 't' } })
    const r = await hive.upload(new Blob([new Uint8Array([1, 2, 3])], { type: 'image/png' }), { name: 'avatar.png', type: 'image/png', metadata: { kind: 'avatar' } })
    const [url, init] = f.mock.calls[0]
    expect(url).toBe(`${GW}/api/files/upload`)
    expect(init?.credentials).toBe('include')
    expect((init?.headers as Record<string, string>)['X-CSRF-Token']).toBe('t')
    const form = init?.body as FormData
    expect(form.get('name')).toBe('avatar.png')
    expect(form.get('type')).toBe('image/png')
    expect(JSON.parse(form.get('metadata') as string)).toEqual({ kind: 'avatar' })
    expect(form.get('file')).toBeInstanceOf(Blob)
    expect(r.url).toBe(`${GW}/files/${'a'.repeat(64)}`)
    expect(r.file_id).toBe(manifest.file_id)
  })

  it('surfaces the operator-auth requirement', async () => {
    const f = mockFetch(json(401, { error: 'operator authentication required' }))
    const err = await new Hive(ADDR, { gatewayUrl: GW, fetch: f }).upload(new Blob(['x'])).catch((e) => e)
    expect(err).toBeInstanceOf(HiveRequestError)
    expect(err.message).toContain('operator authentication required')
  })
})
