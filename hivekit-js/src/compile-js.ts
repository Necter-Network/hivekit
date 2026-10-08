/**
 * JavaScript / full TypeScript → hive-wasm-v1 via the embedded Boa interpreter.
 *
 * The prebuilt runtime (`runtime/hivekit-js-runtime.wasm`, built reproducibly by
 * `runtime-js/build.sh`) is a JavaScript engine compiled to
 * `wasm32-unknown-unknown`: no WASI, only the hive-wasm-v1 host imports. The
 * user's script (TypeScript is transpiled to plain JavaScript first) is embedded
 * into a copy of it; see `embed.ts`.
 */

import * as fs from 'fs'
import * as path from 'path'
import * as ts from 'typescript'
import { validateAbi } from './abi'
import { embedScript, scriptBlob } from './embed'
import { buildManifest, packageHbc, sortFunctions, type Packaged } from './hbc'
import { extractDefinedFunctions } from './source'
import { VERSION } from './version'

/** Engine identifier recorded in the manifest's `compiler` field. */
export const JS_ENGINE = 'boa@0.22.0'

export function jsCompilerId(): string {
  return `hivekit-js/${VERSION}+${JS_ENGINE}`
}

export function runtimeWasmPath(): string {
  return path.join(__dirname, '..', 'runtime', 'hivekit-js-runtime.wasm')
}

export function loadJsRuntime(): Uint8Array {
  const p = runtimeWasmPath()
  if (!fs.existsSync(p)) {
    throw new Error(`HiveKit JavaScript runtime not found at ${p}; rebuild it with \`npm run build:runtime\``)
  }
  return new Uint8Array(fs.readFileSync(p))
}

/**
 * Turn module source (ESM or CommonJS, JavaScript or TypeScript) into a plain
 * CommonJS script for the engine. Type errors are not checked here (run `tsc`
 * for that); syntax errors are reported with line numbers.
 */
export function toEngineScript(source: string, fileName: string): string {
  const out = ts.transpileModule(source, {
    fileName,
    reportDiagnostics: true,
    compilerOptions: {
      target: ts.ScriptTarget.ES2020,
      module: ts.ModuleKind.CommonJS,
      allowJs: true,
      esModuleInterop: false,
      removeComments: false,
      sourceMap: false,
      inlineSourceMap: false,
    },
  })
  const errors = (out.diagnostics ?? []).filter((d) => d.category === ts.DiagnosticCategory.Error)
  if (errors.length) {
    const msgs = errors.map((d) => {
      const text = ts.flattenDiagnosticMessageText(d.messageText, '\n')
      if (d.file && d.start !== undefined) {
        const { line, character } = d.file.getLineAndCharacterOfPosition(d.start)
        return `${fileName}(${line + 1},${character + 1}): ${text}`
      }
      return text
    })
    throw new Error(`syntax error:\n${msgs.join('\n')}`)
  }
  return out.outputText
}

export interface CompileJsOptions {
  name: string
  language?: 'javascript' | 'typescript'
  version?: string
  description?: string
  file?: string
  /** Override the runtime (tests). */
  runtime?: Uint8Array
}

/** Compile a HiveKit JavaScript/TypeScript module to a `.hbc` using the embedded engine. */
export function compileJavaScript(source: string, opts: CompileJsOptions): Packaged & { wasm: Uint8Array } {
  const language = opts.language ?? 'javascript'
  const file = opts.file ?? (language === 'typescript' ? 'module.ts' : 'module.js')
  const names = sortFunctions(extractDefinedFunctions(source))
  const script = toEngineScript(source, file)
  const wasm = embedScript(opts.runtime ?? loadJsRuntime(), scriptBlob(names, script))
  validateAbi(wasm)
  const manifest = buildManifest({
    name: opts.name,
    language,
    compiler: jsCompilerId(),
    functions: names,
    version: opts.version,
    description: opts.description,
  })
  return { ...packageHbc(manifest, wasm), wasm }
}
