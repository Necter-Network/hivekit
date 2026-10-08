// Node smoke test of the wasm-bindgen package against NDSR's test vectors.
//   cargo build -p hivekit-js --target wasm32-unknown-unknown --release
//   wasm-bindgen --target nodejs --out-dir pkg target/wasm32-unknown-unknown/release/hivekit_js.wasm
//   node crates/hivekit-js/tests/smoke.cjs pkg
const path = require('path');
const fs = require('fs');
const hk = require(path.resolve(process.argv[2] || 'pkg', 'hivekit_js.js'));
const v = JSON.parse(fs.readFileSync(path.join(__dirname, '../../hivekit-core/tests/data/test-vectors.json')));
const ma = v.manifest_address;
const wasm = Buffer.from(ma.wasm_hex, 'hex');
const r = hk.packageWasm(wasm, ma.manifest.name, ma.manifest.language, ma.manifest.functions, ma.manifest.compiler, ma.manifest.version, undefined);
if (r.manifestAddress !== ma.manifest_address) throw new Error('address');
if (Buffer.from(r.hbcBytes).toString('base64') !== ma.hbc_base64) throw new Error('hbc bytes');
const i = hk.inspect(r.hbcBytes);
if (i.manifestAddress !== ma.manifest_address || !i.executable) throw new Error('inspect');
if (hk.computeAddress(ma.canonical_manifest, wasm) !== ma.manifest_address) throw new Error('computeAddress');
for (const c of v.canonical_json) if (hk.canonicalJson(JSON.stringify(c.input)) !== c.canonical) throw new Error('canon');
try { hk.inspect(new Uint8Array([1,2,3])); throw 0 } catch (e) { if (!(e instanceof Error)) throw new Error('error type'); }
console.log('js ok', r.manifestAddress, i.imports, hk.detectFunctions('#[hive_export]\nfn add_numbers(x: Value) -> Value { x }', 'rust'));
