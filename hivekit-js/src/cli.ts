#!/usr/bin/env node
/**
 * hivec — HiveKit JS/TS compiler CLI
 *
 *   hivec build <file> [-o dir] [--target as|js]   compile to .hbc
 *   hivec inspect <file.hbc>                       validate and show the manifest
 *   hivec run <file|.hbc> <fn> [input]             build (if needed) and execute
 *   hivec functions <file>                         list hive.define() names
 */

import { Command } from 'commander'
import { spawnSync } from 'child_process'
import * as fs from 'fs'
import * as os from 'os'
import * as path from 'path'
import { validateAbi } from './abi'
import { compileFile, type Target } from './compiler'
import { readHbc } from './hbc'
import { findNdsr, LocalNode, runWithNdsr } from './local'
import { extractDefinedFunctions } from './source'
import { VERSION } from './version'

const program = new Command()
program.name('hivec').description('HiveKit JS/TS compiler for hive-wasm-v1 (NDSR)').version(VERSION)

function fail(e: unknown): never {
  console.error(`error: ${e instanceof Error ? e.message : String(e)}`)
  process.exit(1)
}

function parseTarget(t?: string): Target | undefined {
  if (t === undefined) return undefined
  if (t !== 'as' && t !== 'js') fail(`--target must be "as" or "js"`)
  return t
}

program
  .command('build <source>')
  .description('compile a module to a .hbc artifact')
  .option('-o, --out <dir>', 'output directory', 'dist')
  .option('-n, --name <name>', 'module name (default: file stem)')
  .option('--module-version <version>', 'manifest version')
  .option('--description <text>', 'manifest description')
  .option('-t, --target <target>', '"as" (AssemblyScript, default for .ts) or "js" (JavaScript engine, default for .js)')
  .action((source: string, o: { out: string; name?: string; moduleVersion?: string; description?: string; target?: string }) => {
    try {
      const r = compileFile(source, {
        outDir: o.out,
        name: o.name,
        version: o.moduleVersion,
        description: o.description,
        target: parseTarget(o.target),
      })
      console.log(
        JSON.stringify(
          { hbc: r.hbcPath, manifest_address: r.manifestAddress, functions: r.functions, wasm_bytes: r.wasm.length, compiler: r.manifest.compiler },
          null,
          2,
        ),
      )
    } catch (e) {
      fail(e)
    }
  })

program
  .command('inspect <hbc>')
  .description('validate a .hbc (with `ndsr inspect` when available) and print its manifest')
  .action((file: string) => {
    try {
      const ndsr = findNdsr()
      if (ndsr) {
        const r = spawnSync(ndsr, ['inspect', file], { stdio: 'inherit' })
        process.exit(r.status ?? 1)
      }
      const { manifest, wasm, manifestAddress } = readHbc(new Uint8Array(fs.readFileSync(file)))
      let abiError: string | null = null
      try {
        validateAbi(wasm)
      } catch (e) {
        abiError = (e as Error).message
      }
      console.log(
        JSON.stringify(
          {
            manifest_address: manifestAddress,
            manifest,
            functions: manifest.functions.map((name, func_id) => ({ func_id, name })),
            wasm_bytes: wasm.length,
            abi_valid: abiError === null,
            abi_error: abiError,
          },
          null,
          2,
        ),
      )
    } catch (e) {
      fail(e)
    }
  })

program
  .command('run <source> <fn> [input]')
  .description('build if needed, then execute one function with `ndsr run` (or an in-process host with --local)')
  .option('--gas <n>', 'gas limit', '1000000000')
  .option('--data-dir <dir>', 'persist state between runs (ndsr only)')
  .option('-t, --target <target>', 'compilation target for sources')
  .option('--local', 'use the in-process Node host even if ndsr is available')
  .action((source: string, fn: string, input: string | undefined, o: { gas: string; dataDir?: string; target?: string; local?: boolean }) => {
    try {
      let hbcPath = source
      let tmp: string | null = null
      if (!source.endsWith('.hbc')) {
        tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'hivec-run-'))
        hbcPath = compileFile(source, { outDir: tmp, target: parseTarget(o.target) }).hbcPath
      }
      const ndsr = o.local ? null : findNdsr()
      const result = ndsr
        ? runWithNdsr(ndsr, hbcPath, fn, input ?? '', { gas: Number(o.gas), dataDir: o.dataDir })
        : (() => {
            const node = new LocalNode()
            const addr = node.load(new Uint8Array(fs.readFileSync(hbcPath)))
            return node.execute(addr, fn, input ?? '')
          })()
      if (tmp) fs.rmSync(tmp, { recursive: true, force: true })
      console.log(JSON.stringify({ runner: ndsr ? 'ndsr' : 'local', ...result }, null, 2))
      if (!result.success) process.exit(2)
    } catch (e) {
      fail(e)
    }
  })

program
  .command('functions <source>')
  .description('list the hive.define() functions in a source file (sorted; func_id = index)')
  .action((source: string) => {
    try {
      const names = extractDefinedFunctions(fs.readFileSync(source, 'utf8')).sort()
      names.forEach((n, i) => console.log(`${i}\t${n}`))
    } catch (e) {
      fail(e)
    }
  })

program.parse()
