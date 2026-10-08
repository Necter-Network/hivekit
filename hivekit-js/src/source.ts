/** Source scanning helpers shared by the compilers. */

/**
 * Replace comments (and, optionally, string/template literal contents) with
 * spaces while preserving newlines, so regex checks see only code and still
 * report correct line numbers.
 */
export function maskSource(src: string, opts: { strings: boolean }): string {
  const out = src.split('')
  let i = 0
  const blank = (from: number, to: number): void => {
    for (let k = from; k < to; k++) if (out[k] !== '\n') out[k] = ' '
  }
  while (i < src.length) {
    const c = src[i]
    const n = src[i + 1]
    if (c === '/' && n === '/') {
      const end = src.indexOf('\n', i)
      const stop = end < 0 ? src.length : end
      blank(i, stop)
      i = stop
    } else if (c === '/' && n === '*') {
      const end = src.indexOf('*/', i + 2)
      const stop = end < 0 ? src.length : end + 2
      blank(i, stop)
      i = stop
    } else if (c === '"' || c === "'" || c === '`') {
      let j = i + 1
      while (j < src.length && src[j] !== c) {
        if (src[j] === '\\') j++
        else if (c !== '`' && src[j] === '\n') break
        j++
      }
      if (opts.strings) blank(i + 1, j)
      i = j + 1
    } else {
      i++
    }
  }
  return out.join('')
}

export function lineOf(src: string, index: number): number {
  let line = 1
  for (let i = 0; i < index && i < src.length; i++) if (src[i] === '\n') line++
  return line
}

/**
 * `hive.define("name", …)` names, exactly like `ndsr compile`: every call must
 * take a string literal as its first argument. Comments are ignored.
 */
export function extractDefinedFunctions(source: string): string[] {
  const code = maskSource(source, { strings: false })
  const any = code.match(/hive\s*\.\s*define\s*\(/g) ?? []
  const names: string[] = []
  const lit = /hive\s*\.\s*define\s*\(\s*(["'])([^"'\\\n]*)\1/g
  let m: RegExpExecArray | null
  while ((m = lit.exec(code)) !== null) names.push(m[2])
  if (names.length !== any.length) {
    throw new Error('every hive.define(...) must take a string literal name as its first argument')
  }
  if (names.length === 0) throw new Error('no hive.define("name", handler) calls found')
  return names
}
