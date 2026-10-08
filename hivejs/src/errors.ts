/** The gateway rejected the request (4xx): bad input, unknown module, auth, rate limit. */
export class HiveRequestError extends Error {
  constructor(message: string, public readonly status: number, public readonly body: unknown) {
    super(message)
    this.name = 'HiveRequestError'
  }
}

/** The gateway or execution node was unavailable (5xx, network failure, timeout). */
export class HiveNetworkError extends Error {
  constructor(message: string, public readonly status?: number) {
    super(message)
    this.name = 'HiveNetworkError'
  }
}

/** The module ran and failed (trap, abort, out of gas). Carries the signed receipt. */
export class HiveExecutionError extends Error {
  constructor(message: string, public readonly receipt: unknown, public readonly gasUsed: number) {
    super(message)
    this.name = 'HiveExecutionError'
  }
}
