import { spawnSync } from 'child_process'
import * as fs from 'fs'
import * as os from 'os'
import * as path from 'path'
import { findNdsr, runWithNdsr, type RunResult } from '../src/local'

export const NDSR = findNdsr(__dirname)
export const examples = path.join(__dirname, '..', 'examples')

export function tmpdir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'hivekit-test-'))
}

export function inspect(hbcPath: string): Record<string, any> { // eslint-disable-line @typescript-eslint/no-explicit-any
  const r = spawnSync(NDSR as string, ['inspect', hbcPath], { encoding: 'utf8' })
  if (r.status !== 0) throw new Error(`ndsr inspect failed: ${r.stderr}`)
  return JSON.parse(r.stdout)
}

export function run(hbcPath: string, fn: string, input: string, dataDir?: string, gas = 2_000_000_000): RunResult {
  return runWithNdsr(NDSR as string, hbcPath, fn, input, { gas, dataDir })
}

/** Place an artifact where `ndsr run --data-dir` resolves hive.call targets. */
export function install(dataDir: string, address: string, hbc: Uint8Array): void {
  fs.mkdirSync(path.join(dataDir, 'modules'), { recursive: true })
  fs.writeFileSync(path.join(dataDir, 'modules', `${address}.hbc`), hbc)
}
