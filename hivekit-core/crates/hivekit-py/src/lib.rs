//! `hivekit_core`: Python bindings for hivekit-core.
//!
//! ```python
//! import hivekit_core
//!
//! wasm = open("target/wasm32-unknown-unknown/release/my_module.wasm", "rb").read()
//! r = hivekit_core.package_wasm(wasm, name="my_module", language="rust")
//! r.save("dist/my_module.hbc")
//! info = hivekit_core.inspect(open("dist/my_module.hbc", "rb").read())
//! assert info["manifest_address"] == r.manifest_address
//! ```

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

fn err(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(format!("{e:#}"))
}

fn to_py<'py>(py: Python<'py>, v: &serde_json::Value) -> PyResult<Bound<'py, PyAny>> {
    let s = serde_json::to_string(v).map_err(err)?;
    py.import("json")?.call_method1("loads", (s,))
}

/// Result of packaging a module.
#[pyclass(name = "CompileResult", frozen)]
pub struct PyCompileResult {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    manifest_address: String,
    /// Sorted function names; func_id = index into this list.
    #[pyo3(get)]
    functions: Vec<String>,
    /// Canonical manifest.json text stored in the archive.
    #[pyo3(get)]
    manifest_json: String,
    hbc: Vec<u8>,
    wasm: Vec<u8>,
}

#[pymethods]
impl PyCompileResult {
    /// The .hbc archive bytes.
    #[getter]
    fn hbc_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.hbc)
    }

    /// The module.wasm bytes stored in the archive.
    #[getter]
    fn wasm_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.wasm)
    }

    /// Write the .hbc to `path`.
    fn save(&self, path: &str) -> PyResult<()> {
        std::fs::write(path, &self.hbc).map_err(err)
    }

    fn __repr__(&self) -> String {
        format!(
            "CompileResult(name={:?}, manifest_address={}, functions={:?})",
            self.name, self.manifest_address, self.functions
        )
    }
}

impl From<hk::CompileResult> for PyCompileResult {
    fn from(r: hk::CompileResult) -> Self {
        Self {
            name: r.manifest.name.clone(),
            manifest_address: r.manifest_address,
            functions: r.functions,
            manifest_json: String::from_utf8(r.manifest_bytes).unwrap_or_default(),
            hbc: r.hbc_bytes,
            wasm: r.module_bytes,
        }
    }
}

/// Package a compiled hive-wasm-v1 module into a .hbc.
///
/// `functions` defaults to the module's `hivekit.functions` section (Rust SDK);
/// if both are present they must agree.
#[pyfunction]
#[pyo3(signature = (wasm, name, language="rust", functions=None, compiler=None, version=None, description=None))]
#[allow(clippy::too_many_arguments)]
fn package_wasm(
    wasm: &[u8],
    name: &str,
    language: &str,
    functions: Option<Vec<String>>,
    compiler: Option<String>,
    version: Option<String>,
    description: Option<String>,
) -> PyResult<PyCompileResult> {
    hk::package_wasm(hk::PackageWasmOptions {
        name: name.into(),
        language: language.into(),
        wasm_bytes: wasm.to_vec(),
        functions,
        compiler,
        version,
        description,
    })
    .map(Into::into)
    .map_err(err)
}

/// Package compiled wasm, discovering the function list from `source`.
/// Source text is never packaged.
#[pyfunction]
#[pyo3(signature = (source, wasm, name, language="rust", version=None, description=None))]
fn compile(
    source: &str,
    wasm: &[u8],
    name: &str,
    language: &str,
    version: Option<String>,
    description: Option<String>,
) -> PyResult<PyCompileResult> {
    hk::compile(hk::CompileOptions {
        name: name.into(),
        language: language.into(),
        source: source.into(),
        wasm_bytes: wasm.to_vec(),
        version,
        description,
    })
    .map(Into::into)
    .map_err(err)
}

/// Load and verify a .hbc (container rules, strict manifest, address).
/// Returns a dict: manifest, manifest_address, imports, exports,
/// declared_functions, abi_error, executable.
#[pyfunction]
fn inspect<'py>(py: Python<'py>, hbc: &[u8]) -> PyResult<Bound<'py, PyAny>> {
    let a = hk::load_hbc(hbc).map_err(err)?;
    let v = serde_json::json!({
        "manifest": a.manifest,
        "manifest_address": a.manifest_address,
        "imports": a.wasm_info.imports,
        "exports": a.wasm_info.exports,
        "declared_functions": a.wasm_info.declared_functions,
        "abi_error": a.wasm_info.abi_error,
        "executable": a.wasm_info.abi_error.is_none(),
    });
    to_py(py, &v)
}

/// Inspect a bare module.wasm: imports, exports, declared functions, abi_error.
#[pyfunction]
fn inspect_wasm<'py>(py: Python<'py>, wasm: &[u8]) -> PyResult<Bound<'py, PyAny>> {
    let i = hk::inspect_wasm(wasm).map_err(err)?;
    to_py(py, &serde_json::to_value(i).map_err(err)?)
}

/// Content address of a manifest (JSON text; any manifest_address key is
/// ignored) and wasm bytes. The manifest is validated strictly.
#[pyfunction]
fn compute_address(manifest_json: &str, wasm: &[u8]) -> PyResult<String> {
    let m = hk::Manifest::from_json(manifest_json.as_bytes()).map_err(err)?;
    m.compute_address(wasm).map_err(err)
}

/// Canonical JSON (sorted keys, no whitespace, UTF-8, integers only) of a JSON text.
#[pyfunction]
fn canonical_json(json_text: &str) -> PyResult<String> {
    hk::canonicalize_str(json_text).map_err(err)
}

/// Keccak-256 (Ethereum) of bytes as "0x" + 64 lowercase hex.
#[pyfunction]
fn keccak256_hex(data: &[u8]) -> String {
    hk::keccak_hex(data)
}

/// Canonical form of a module address, or None if malformed.
#[pyfunction]
fn normalize_address(address: &str) -> Option<String> {
    hk::normalize_module_address(address)
}

/// Exported function names found in source (sorted, unique).
#[pyfunction]
#[pyo3(signature = (source, language="rust"))]
fn detect_functions(source: &str, language: &str) -> Vec<String> {
    hk::FunctionRegistry::collect(source, language)
}

#[pymodule]
fn hivekit_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(package_wasm, m)?)?;
    m.add_function(wrap_pyfunction!(compile, m)?)?;
    m.add_function(wrap_pyfunction!(inspect, m)?)?;
    m.add_function(wrap_pyfunction!(inspect_wasm, m)?)?;
    m.add_function(wrap_pyfunction!(compute_address, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_json, m)?)?;
    m.add_function(wrap_pyfunction!(keccak256_hex, m)?)?;
    m.add_function(wrap_pyfunction!(normalize_address, m)?)?;
    m.add_function(wrap_pyfunction!(detect_functions, m)?)?;
    m.add_class::<PyCompileResult>()?;
    m.add("RUNTIME_ID", hk::RUNTIME_ID)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
