# @necter/hivejs

Call Necter modules from browsers and Node through a CCS gateway. No
dependencies; works anywhere `fetch` exists (or pass your own).

```bash
npm install https://github.com/Necter-Network/hivekit/releases/download/v1.0.0/necter-hivejs-1.0.0.tgz
```

Distributed from GitHub releases; the unrelated npm package named `hivejs` is not part of Necter.

```ts
import { Hive, HiveExecutionError } from '@necter/hivejs'

const hive = new Hive('0x8a93…b454', {          // the module's manifest_address
  gatewayUrl: 'https://testnet-rpc.necter.network', // required: RPC host or your gateway
  gasLimit: 50_000_000,                         // optional; CCS default is 1,000,000
})

const res = await hive.call('increment', { by: 2 })
res.data         // output parsed as JSON (or the raw string)
res.output       // output exactly as returned by the module
res.events       // [{ name, data }, …]
res.receiptHash  // keccak256 receipt hash; res.receipt is the signed envelope
res.gasUsed, res.nodeId, res.timestamp

await hive.PostData({ action: 'increment', by: 2 })   // same as call('increment', { by: 2 })
```

## API

| Method | Request |
|---|---|
| `call(fn, input?, { gasLimit? })` | `POST {gateway}/api/execute` with `{module, function, input, gas_limit?}` |
| `PostData({ action, ...input })` | same as `call(action, input)` |
| `getModule()` | `GET {gateway}/api/modules/{address}` |
| `upload(blob, { name?, type?, metadata?, authToken? })` | `POST {gateway}/api/files/upload` (multipart) |
| `fileUrl(fileId)` | `{gateway}/files/{fileId}` (public download) |

Inputs: objects are sent as JSON and handed to the module as canonical JSON
(the node rejects floats in object inputs; send such data as a string);
strings are passed through byte-for-byte.

Errors:

- `HiveExecutionError`: the module ran and failed (trap, `hive.fail`, out of
  gas). `.receipt` holds the signed receipt, `.gasUsed` the gas charged.
- `HiveRequestError`: the gateway rejected the request (4xx: unknown module,
  bad input, rate limit, auth). `.status`, `.body`.
- `HiveNetworkError`: gateway/node unavailable (5xx), network failure or timeout.

**Retries.** `call` (POST `/api/execute`) is never retried automatically: a
call can have executed even if its response was lost, so retrying could run a
state-changing function twice. Only idempotent GETs (`getModule`) are retried
(`retries`, default 2, with backoff).

**Identity.** Calls carry no caller identity. CCS does not authenticate
`/api/execute`, and an unsigned wallet-address header would be a claim anyone
can forge, so hivejs does not send one. If your module needs to know who is
calling, have the user sign a message (EIP-191) and verify it in a trusted
service before acting on it.

**Uploads.** CCS accepts uploads only from an authenticated operator: a
same-origin operator session (`credentials: 'include'` plus an
`X-CSRF-Token` header via the `headers` option) or a bearer token
(`authToken`) used from a server. Never expose operator or deployer tokens to
browsers (for example through `NEXT_PUBLIC_*` variables); see
`examples/react-app.tsx` for a server route that keeps the token server-side.

## Options

```ts
new Hive(address, {
  gatewayUrl: string,                 // required
  gasLimit?: number,                  // default gas_limit for call()
  timeout?: number,                   // ms, default 30000
  retries?: number,                   // GET retries, default 2
  headers?: Record<string, string>,   // added to every request
  credentials?: RequestCredentials,   // fetch credentials mode
  fetch?: typeof fetch,               // custom fetch (tests, SSR)
})
```

Addresses are normalized (`hive:` prefix and upper case accepted) and must be
`0x` + 64 hex characters.

## Development

```bash
npm ci
npm test          # vitest with a mocked fetch
npm run typecheck && npm run lint
npm run build     # dist/cjs (CommonJS + .d.ts) and dist/esm
```

## License

Apache-2.0. See [LICENSE](../LICENSE) and [NOTICE](../NOTICE).
