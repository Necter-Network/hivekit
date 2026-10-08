/**
 * HiveJS — React example (e.g. a Next.js client component).
 *
 * Configuration that is safe to expose to browsers:
 *   NEXT_PUBLIC_HIVE_GATEWAY_URL   base URL of your CCS gateway
 *   NEXT_PUBLIC_HIVE_MODULE        the module's manifest_address
 *
 * Never put secrets (CCS admin/deployer tokens, API keys) in NEXT_PUBLIC_*
 * variables: they are inlined into the browser bundle. Calls to /api/execute
 * need no credentials; operations that do (file uploads, deploys) belong in a
 * server route that reads its token from a non-public environment variable.
 *
 * The module (examples/counter.js in hivekit-js):
 *   hive.define('increment', (input) => { ...; return { count } })
 *   hive.define('get', () => ({ count: db.get('count', 0) }))
 */

import React, { useEffect, useState } from 'react'
import { Hive, HiveExecutionError } from '@necter/hivejs'

const hive = new Hive(process.env.NEXT_PUBLIC_HIVE_MODULE as string, {
  gatewayUrl: process.env.NEXT_PUBLIC_HIVE_GATEWAY_URL as string,
  // JavaScript-engine modules need more gas than the CCS default of 1,000,000.
  gasLimit: 50_000_000,
})

export function Counter() {
  const [count, setCount] = useState<number | null>(null)
  const [status, setStatus] = useState('')

  useEffect(() => {
    hive
      .call<{ count: number }>('get')
      .then((res) => setCount(res.data.count))
      .catch((e: Error) => setStatus(e.message))
  }, [])

  async function increment() {
    try {
      const res = await hive.call<{ count: number }>('increment', { by: 1 })
      setCount(res.data.count)
      setStatus(`receipt ${res.receiptHash.slice(0, 12)}… (gas ${res.gasUsed})`)
    } catch (e) {
      // A failed execution still produced a signed receipt.
      setStatus(e instanceof HiveExecutionError ? `failed: ${e.message}` : (e as Error).message)
    }
  }

  return (
    <div>
      <p>Count: {count ?? '…'}</p>
      <button onClick={increment}>Increment</button>
      <p>{status}</p>
    </div>
  )
}

/*
 * Server-side upload (Next.js route handler). The operator token stays on the server.
 *
 * // app/api/upload/route.ts
 * import { Hive } from '@necter/hivejs'
 * export async function POST(req: Request) {
 *   const form = await req.formData()
 *   const file = form.get('file') as File
 *   const hive = new Hive(process.env.HIVE_MODULE!, { gatewayUrl: process.env.HIVE_GATEWAY_URL! })
 *   const r = await hive.upload(file, { name: file.name, authToken: process.env.CCS_ADMIN_TOKEN })
 *   return Response.json({ url: r.url })
 * }
 */
