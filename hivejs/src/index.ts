/**
 * HiveJS — call HiveKit modules from browsers and Node through a CCS gateway.
 *
 * ```ts
 * import { Hive } from 'hivejs'
 * const hive = new Hive('0x8a93…b454', { gatewayUrl: 'https://ccs.example.org' })
 * const res = await hive.call('increment', { by: 2 })
 * res.data        // parsed output
 * res.receiptHash // signed execution receipt hash
 * ```
 */

export { Hive, normalizeAddress, normalizeGateway } from './client.js'
export { HiveRequestError, HiveNetworkError, HiveExecutionError } from './errors.js'
export type { HiveOptions, HiveResponse, HiveEvent, UploadOptions, UploadResponse, ModuleInfo, ModuleAddress } from './types.js'
