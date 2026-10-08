/** A module address: `0x` + 64 hex (a `hive:` prefix is accepted and stripped). */
export type ModuleAddress = string

export interface HiveOptions {
  /**
   * Base URL of the CCS gateway, e.g. `https://ccs.example.org`. Required:
   * there is no built-in default network.
   */
  gatewayUrl: string
  /**
   * Default gas limit per call. When unset the gateway's default applies
   * (CCS: 1_000_000). Modules built with the JavaScript or Python engines need
   * considerably more gas per call than AssemblyScript/Rust/Go modules.
   */
  gasLimit?: number
  /** Request timeout in ms (default 30_000). */
  timeout?: number
  /**
   * Retries for idempotent GET requests on network errors / 5xx (default 2).
   * `execute` is never retried automatically: a call may have run even if the
   * response was lost.
   */
  retries?: number
  /** Extra headers for every request (e.g. `X-CSRF-Token` for operator sessions). */
  headers?: Record<string, string>
  /** `fetch` credentials mode (e.g. `'include'` to send the CCS session cookie). */
  credentials?: RequestCredentials
  /** Custom fetch implementation (tests, SSR). Defaults to `globalThis.fetch`. */
  fetch?: typeof fetch
}

export interface HiveEvent {
  name: string
  data: unknown
}

/** Result of a successful call. */
export interface HiveResponse<T = unknown> {
  /** The function output parsed as JSON, or the raw string if it is not JSON. */
  data: T
  /** The function output exactly as returned by the module. */
  output: string
  receiptHash: string
  gasUsed: number
  gasLimit: number
  events: HiveEvent[]
  nodeId: string
  /** Unix seconds at which the node signed the receipt. */
  timestamp: number
  /** The signed receipt envelope (docs/HBC_SPEC.md §11). */
  receipt: unknown
}

export interface UploadOptions {
  name?: string
  type?: string
  metadata?: Record<string, unknown>
}

/** File manifest returned by `POST /api/files/upload`. */
export interface UploadResponse {
  file_id: string
  name: string
  type: string
  size: number
  chunk_size: number
  chunks: number
  metadata: Record<string, unknown>
  created_at: string | null
  /** Public download URL (`<gateway>/files/<file_id>`). */
  url: string
}

export interface ModuleInfo {
  manifest_address: string
  name: string
  language: string
  functions: string[]
  [key: string]: unknown
}
