//! Packaging a compiled `hive-wasm-v1` module into a `.hbc` artifact.
//!
//! This crate never compiles source code and never packages source text: a
//! `.hbc` holds exactly `manifest.json` + `module.wasm`. Source is only used to
//! discover exported function names (see [`crate::registry`]).

use crate::abi::validate_wasm;
use crate::hbc::write_hbc;
use crate::manifest::{default_compiler, Manifest};
use crate::registry::FunctionRegistry;
use anyhow::{bail, Result};

/// Options for [`package_wasm`].
#[derive(Debug, Clone, Default)]
pub struct PackageWasmOptions {
    /// Module name (`manifest.name`).
    pub name: String,
    /// Source language (`manifest.language`), e.g. `"rust"`.
    pub language: String,
    /// Compiled `hive-wasm-v1` module bytes.
    pub wasm_bytes: Vec<u8>,
    /// Exported function names. `None` = read them from the module's
    /// `hivekit.functions` custom section (written by the HiveKit Rust SDK).
    /// When both are available they must agree.
    pub functions: Option<Vec<String>>,
    /// `manifest.compiler`; defaults to `hivekit-core/<version>`.
    pub compiler: Option<String>,
    /// Optional `manifest.version`.
    pub version: Option<String>,
    /// Optional `manifest.description`.
    pub description: Option<String>,
}

/// Output of [`package_wasm`] / [`compile`].
#[derive(Debug, Clone)]
pub struct CompileResult {
    /// The manifest, with `manifest_address` set.
    pub manifest: Manifest,
    /// `0x` + 64 lowercase hex.
    pub manifest_address: String,
    /// Sorted function names; `func_id` = index into this list.
    pub functions: Vec<String>,
    /// The `.hbc` archive bytes.
    pub hbc_bytes: Vec<u8>,
    /// Canonical `manifest.json` bytes stored in the archive (includes `manifest_address`).
    pub manifest_bytes: Vec<u8>,
    /// `module.wasm` bytes stored in the archive.
    pub module_bytes: Vec<u8>,
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// Validate a compiled module against the ABI, build the canonical manifest,
/// compute the content address and write a deterministic `.hbc`.
pub fn package_wasm(opts: PackageWasmOptions) -> Result<CompileResult> {
    let info = validate_wasm(&opts.wasm_bytes)?;
    let functions = match (opts.functions, info.declared_functions) {
        (Some(given), Some(declared)) => {
            let (g, d) = (sorted(given), sorted(declared));
            if g != d {
                bail!(
                    "function list {g:?} does not match the module's dispatch table {d:?}; \
                     func_id is an index into the sorted list, so they must be identical"
                );
            }
            g
        }
        (Some(given), None) => given,
        (None, Some(declared)) => declared,
        (None, None) => bail!(
            "no function list: pass the exported function names \
             (the module has no hivekit.functions section)"
        ),
    };
    let compiler = opts.compiler.unwrap_or_else(default_compiler);
    let mut manifest = Manifest::new(&opts.name, &opts.language, &compiler, functions)?;
    manifest.version = opts.version;
    manifest.description = opts.description;
    manifest.validate()?;
    let manifest_address = manifest.seal(&opts.wasm_bytes)?;
    let manifest_bytes = manifest.to_canonical_json()?;
    let hbc_bytes = write_hbc(&manifest_bytes, &opts.wasm_bytes)?;
    Ok(CompileResult {
        functions: manifest.functions.clone(),
        manifest,
        manifest_address,
        hbc_bytes,
        manifest_bytes,
        module_bytes: opts.wasm_bytes,
    })
}

/// Options passed to [`compile`].
#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// Module name.
    pub name: String,
    /// Source language: `"rust"`, `"go"`, `"assemblyscript"`, `"javascript"`, ...
    pub language: String,
    /// Module source, used only to discover exported function names.
    pub source: String,
    /// The module compiled by that language's hive-wasm-v1 toolchain.
    pub wasm_bytes: Vec<u8>,
    /// Optional `manifest.version`.
    pub version: Option<String>,
    /// Optional `manifest.description`.
    pub description: Option<String>,
}

/// Package compiled wasm, discovering the function list from `source`.
///
/// Source text is never packaged: an `.hbc` must contain a compiled
/// `module.wasm`, so `wasm_bytes` is required. If the module also declares its
/// functions (Rust SDK), the two lists must agree.
pub fn compile(opts: CompileOptions) -> Result<CompileResult> {
    if opts.wasm_bytes.is_empty() {
        bail!(
            "compile() needs the compiled module.wasm: NDSR executes only hive-wasm-v1 wasm, \
             never source text. Build the module with the {} toolchain first.",
            opts.language
        );
    }
    let functions = FunctionRegistry::collect(&opts.source, &opts.language);
    if functions.is_empty() {
        bail!("no exported functions found in {} source", opts.language);
    }
    package_wasm(PackageWasmOptions {
        name: opts.name,
        language: opts.language,
        wasm_bytes: opts.wasm_bytes,
        functions: Some(functions),
        compiler: None,
        version: opts.version,
        description: opts.description,
    })
}
