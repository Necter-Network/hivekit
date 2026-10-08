/**
 * HiveKit JS/TS compiler: source → spec-conformant `.hbc` (HBC_SPEC.md).
 *
 * Targets:
 * - `as` (default for `.ts`): AssemblyScript, same ABI and output as `ndsr compile`.
 *   Small, fast modules; a strict TypeScript subset.
 * - `js` (default for `.js`/`.mjs`/`.cjs`; opt-in for `.ts`): the source runs in an
 *   embedded JavaScript engine (Boa) compiled to hive-wasm-v1. Full JavaScript and
 *   TypeScript, larger modules and higher gas per call.
 */

import * as fs from 'fs'
import * as path from 'path'
import { compileAssemblyScript } from './compile-as'
import { compileJavaScript } from './compile-js'
import type { Manifest } from './hbc'

export type Target = 'as' | 'js'

export interface CompileOptions {
  /** Module name (default: file stem). */
  name?: string
  version?: string
  description?: string
  /** Compilation target (default: by file extension). */
  target?: Target
}

export interface CompileResult {
  name: string
  manifestAddress: string
  functions: string[]
  manifest: Manifest
  hbc: Uint8Array
  wasm: Uint8Array
  target: Target
}

export function defaultTarget(file: string): Target {
  const ext = path.extname(file).toLowerCase()
  if (ext === '.ts' || ext === '.as') return 'as'
  if (ext === '.js' || ext === '.mjs' || ext === '.cjs' || ext === '.mts' || ext === '.cts') return 'js'
  throw new Error(`cannot infer the target for ${file}; use a .ts/.js extension or pass a target`)
}

/** Compile source text. `file` is used for the default target and error messages. */
export function compile(source: string, file: string, opts: CompileOptions = {}): CompileResult {
  const target = opts.target ?? defaultTarget(file)
  const ext = path.extname(file).toLowerCase()
  const name = opts.name ?? path.basename(file, path.extname(file))
  const common = { name, version: opts.version, description: opts.description, file: path.basename(file) }
  const r =
    target === 'as'
      ? compileAssemblyScript(source, common)
      : compileJavaScript(source, {
          ...common,
          language: ext === '.ts' || ext === '.mts' || ext === '.cts' ? 'typescript' : 'javascript',
        })
  return {
    name,
    manifestAddress: r.manifestAddress,
    functions: r.manifest.functions,
    manifest: r.manifest,
    hbc: r.hbc,
    wasm: r.wasm,
    target,
  }
}

/** Compile a file and write `<outDir>/<name>.hbc`. */
export function compileFile(file: string, opts: CompileOptions & { outDir?: string } = {}): CompileResult & { hbcPath: string } {
  const source = fs.readFileSync(file, 'utf8')
  const r = compile(source, file, opts)
  const outDir = opts.outDir ?? 'dist'
  fs.mkdirSync(outDir, { recursive: true })
  const hbcPath = path.join(outDir, `${r.name}.hbc`)
  fs.writeFileSync(hbcPath, r.hbc)
  return { ...r, hbcPath }
}
