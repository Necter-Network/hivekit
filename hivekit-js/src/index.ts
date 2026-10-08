/**
 * HiveKit JS/TS SDK — build hive-wasm-v1 modules for NDSR.
 *
 * ```ts
 * import { compile } from 'hivekit'
 * const { hbc, manifestAddress } = compile(source, 'counter.ts')
 * ```
 *
 * Module authors use `hive`, `storage` and `db` (see README). The frontend
 * client lives in `hivejs` and is re-exported here as `HiveClient`.
 */

export { hive, storage, db, HiveRuntime, HiveCallError, HiveAbort } from './runtime'
export type { Handler, HiveContext, HiveEvent, Storage, Db } from './runtime'
export { compile, compileFile, defaultTarget } from './compiler'
export type { CompileOptions, CompileResult, Target } from './compiler'
export { compileAssemblyScript, ASC_VERSION, UnsupportedFeatureError } from './compile-as'
export { compileJavaScript, JS_ENGINE } from './compile-js'
export { canonicalJson, canonicalBytes, keccak256 } from './canonical'
export { buildManifest, manifestAddress, packageHbc, readHbc, validateManifest, normalizeAddress, RUNTIME } from './hbc'
export type { Manifest, Packaged } from './hbc'
export { validateAbi } from './abi'
export { embedScript, scriptBlob } from './embed'
export { extractDefinedFunctions } from './source'
export { findNdsr, runWithNdsr, LocalNode } from './local'
export type { RunResult } from './local'
export { VERSION } from './version'

// Frontend client: one implementation, maintained in hivejs.
export { Hive, Hive as HiveClient, HiveRequestError, HiveNetworkError, HiveExecutionError } from '@necter/hivejs'
export type { HiveOptions, HiveResponse } from '@necter/hivejs'
