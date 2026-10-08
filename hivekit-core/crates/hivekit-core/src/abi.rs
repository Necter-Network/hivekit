//! Static checks of a `module.wasm` against the `hive-wasm-v1` guest ABI
//! (HBC_SPEC.md §6.1 exports, §6.4 imports), mirroring what NDSR enforces at load time.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use wasmparser::{ExternalKind, FuncType, Parser, Payload, TypeRef, ValType};

/// Name of the custom section the HiveKit Rust SDK writes into every module:
/// the exported function names, one per line, sorted exactly like the guest's
/// dispatch table.
pub const FUNCTIONS_SECTION: &str = "hivekit.functions";

/// Maximum size of `module.wasm` (HBC_SPEC §2, consensus parameter; raised from 8 MiB).
pub const MAX_WASM_BYTES: usize = 12 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum T {
    I32,
    I64,
}

/// (module, name, params, results) of every allowed host import.
const IMPORTS: &[(&str, &str, &[T], &[T])] = &[
    (
        "hive",
        "call",
        &[T::I32, T::I32, T::I32, T::I32, T::I32, T::I32],
        &[T::I64],
    ),
    ("hive", "emit", &[T::I32, T::I32, T::I32, T::I32], &[]),
    ("hive", "abort", &[T::I32, T::I32], &[]),
    ("storage", "get", &[T::I32, T::I32], &[T::I64]),
    ("storage", "set", &[T::I32, T::I32, T::I32, T::I32], &[]),
    ("storage", "del", &[T::I32, T::I32], &[]),
    ("console", "log", &[T::I32, T::I32], &[]),
    ("crypto", "hash", &[T::I32, T::I32], &[T::I64]),
    ("env", "abort", &[T::I32, T::I32, T::I32, T::I32], &[]),
];

/// Every allowed host import as `module.name`.
pub fn allowed_imports() -> Vec<String> {
    IMPORTS
        .iter()
        .map(|(m, n, _, _)| format!("{m}.{n}"))
        .collect()
}

/// What [`inspect_wasm`] found in a module.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WasmInfo {
    /// Every import as `module.name`.
    pub imports: Vec<String>,
    /// Every export name.
    pub exports: Vec<String>,
    /// Contents of the [`FUNCTIONS_SECTION`] custom section, if present.
    pub declared_functions: Option<Vec<String>>,
    /// `None` if the module satisfies the hive-wasm-v1 ABI, else the reason it does not.
    pub abi_error: Option<String>,
}

fn same(vals: &[ValType], want: &[T]) -> bool {
    vals.len() == want.len()
        && vals
            .iter()
            .zip(want)
            .all(|(v, w)| matches!((v, w), (ValType::I32, T::I32) | (ValType::I64, T::I64)))
}

/// Parse a module, list its imports/exports and check it against the ABI.
/// Only a malformed binary is an `Err`; ABI violations are reported in `abi_error`.
pub fn inspect_wasm(wasm: &[u8]) -> Result<WasmInfo> {
    if wasm.len() > MAX_WASM_BYTES {
        bail!(
            "module.wasm is {} bytes; the limit is {MAX_WASM_BYTES}",
            wasm.len()
        );
    }
    if wasm.len() < 8 || &wasm[..4] != b"\0asm" {
        bail!("module.wasm is not a WebAssembly binary");
    }
    let mut types: Vec<Option<FuncType>> = Vec::new();
    let mut funcs: Vec<u32> = Vec::new(); // type index per function (imports first)
    let mut memories: Vec<(bool, bool)> = Vec::new(); // (memory64, shared)
    let mut info = WasmInfo::default();
    let mut errors: Vec<String> = Vec::new();
    let mut exports: Vec<(String, ExternalKind, u32)> = Vec::new();

    for payload in Parser::new(0).parse_all(wasm) {
        match payload.context("invalid wasm binary")? {
            Payload::TypeSection(r) => {
                for rec in r {
                    for st in rec.context("type section")?.into_types() {
                        types.push(match st.composite_type.inner {
                            wasmparser::CompositeInnerType::Func(f) => Some(f),
                            _ => None,
                        });
                    }
                }
            }
            Payload::ImportSection(r) => {
                for imp in r.into_imports() {
                    let imp = imp.context("import section")?;
                    let (m, n) = (imp.module, imp.name);
                    info.imports.push(format!("{m}.{n}"));
                    match imp.ty {
                        TypeRef::Func(idx) => {
                            funcs.push(idx);
                            let Some((_, _, p, rs)) =
                                IMPORTS.iter().find(|(im, iname, _, _)| *im == m && *iname == n)
                            else {
                                if m.starts_with("wasi") {
                                    errors.push(format!(
                                        "import {m}.{n}: WASI modules are not supported (hive-wasm-v1 only)"
                                    ));
                                } else {
                                    errors.push(format!(
                                        "import {m}.{n} is not part of the hive-wasm-v1 host interface"
                                    ));
                                }
                                continue;
                            };
                            let ok = types
                                .get(idx as usize)
                                .and_then(|t| t.as_ref())
                                .is_some_and(|ft| same(ft.params(), p) && same(ft.results(), rs));
                            if !ok {
                                errors.push(format!("import {m}.{n} has the wrong signature"));
                            }
                        }
                        _ => errors.push(format!(
                            "import {m}.{n}: only function imports are allowed (no imported memory/table/global)"
                        )),
                    }
                }
            }
            Payload::FunctionSection(r) => {
                for t in r {
                    funcs.push(t.context("function section")?);
                }
            }
            Payload::MemorySection(r) => {
                for mt in r {
                    let mt = mt.context("memory section")?;
                    memories.push((mt.memory64, mt.shared));
                }
            }
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e.context("export section")?;
                    info.exports.push(e.name.to_string());
                    exports.push((e.name.to_string(), e.kind, e.index));
                }
            }
            Payload::CustomSection(c) if c.name() == FUNCTIONS_SECTION => {
                let text = std::str::from_utf8(c.data())
                    .with_context(|| format!("custom section {FUNCTIONS_SECTION} is not UTF-8"))?;
                let list = info.declared_functions.get_or_insert_with(Vec::new);
                list.extend(text.split('\n').filter(|s| !s.is_empty()).map(String::from));
            }
            _ => {}
        }
    }

    let fn_type = |idx: u32| -> Option<&FuncType> {
        funcs
            .get(idx as usize)
            .and_then(|t| types.get(*t as usize))
            .and_then(|t| t.as_ref())
    };
    let (mut has_mem, mut has_alloc, mut has_entry) = (false, false, false);
    for (name, kind, index) in &exports {
        match (name.as_str(), kind) {
            ("memory", ExternalKind::Memory) => {
                match memories.get(*index as usize) {
                    Some((false, false)) => {}
                    _ => errors.push("exported memory must be a 32-bit, non-shared memory".into()),
                }
                has_mem = true;
            }
            ("__alloc", ExternalKind::Func) => {
                if !fn_type(*index)
                    .is_some_and(|f| same(f.params(), &[T::I32]) && same(f.results(), &[T::I32]))
                {
                    errors.push("__alloc must have signature (i32) -> i32".into());
                }
                has_alloc = true;
            }
            ("__hive_entry", ExternalKind::Func) => {
                if !fn_type(*index).is_some_and(|f| {
                    same(f.params(), &[T::I32, T::I32, T::I32]) && same(f.results(), &[T::I64])
                }) {
                    errors.push("__hive_entry must have signature (i32, i32, i32) -> i64".into());
                }
                has_entry = true;
            }
            ("memory" | "__alloc" | "__hive_entry", _) => {
                errors.push(format!("export {name} has the wrong kind"))
            }
            _ => {}
        }
    }
    if !has_mem {
        errors.push("module must export its linear memory as \"memory\"".into());
    }
    if !has_alloc {
        errors.push("module must export __alloc(i32) -> i32".into());
    }
    if !has_entry {
        if info.exports.iter().any(|e| e == "__hive_entry_str") {
            errors.push("module uses the legacy __hive_entry_str ABI; rebuild with a hive-wasm-v1 toolchain".into());
        } else {
            errors.push("module must export __hive_entry(i32, i32, i32) -> i64".into());
        }
    }
    if !errors.is_empty() {
        info.abi_error = Some(errors.join("; "));
    }
    Ok(info)
}

/// Like [`inspect_wasm`] but an ABI violation is an error.
pub fn validate_wasm(wasm: &[u8]) -> Result<WasmInfo> {
    let info = inspect_wasm(wasm)?;
    if let Some(e) = &info.abi_error {
        bail!("module.wasm violates the hive-wasm-v1 ABI: {e}");
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `vector_echo` module from test-vectors.json (imports hive.emit).
    const ECHO_HEX: &str = "0061736d0100000001140360047f7f7f7f0060017f017f60037f7f7f017e020d01046869766504656d69740000030302010205030100010607017f014180080b072303066d656d6f72790200075f5f616c6c6f6300010c5f5f686976655f656e74727900020a2a021101017f23002101230020006a240020010b1600411041062001200210002001ad4220862002ad840b0b0c010041100b066563686f6564002c046e616d650107010004656d6974021502010200016e01017002030002696401017002016c07050100026870";

    #[test]
    fn vector_module_is_valid() {
        let wasm = hex::decode(ECHO_HEX).unwrap();
        let info = validate_wasm(&wasm).unwrap();
        assert_eq!(info.imports, ["hive.emit"]);
        assert!(info.exports.contains(&"__hive_entry".to_string()));
        assert_eq!(info.declared_functions, None);
    }

    #[test]
    fn rejects_garbage_and_missing_exports() {
        assert!(inspect_wasm(b"not wasm").is_err());
        // Empty module: valid binary, but no exports.
        let info = inspect_wasm(b"\0asm\x01\0\0\0").unwrap();
        assert!(info.abi_error.unwrap().contains("memory"));
    }
}
