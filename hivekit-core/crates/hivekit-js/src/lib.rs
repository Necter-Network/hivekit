//! wasm-bindgen bindings for hivekit-core.
//!
//! ```js
//! import init, { packageWasm, inspect } from "hivekit-core-wasm";
//! await init();
//! const r = packageWasm(wasmBytes, "my_module", "rust");
//! console.log(r.manifestAddress, r.functions);
//! const info = inspect(r.hbcBytes);
//! ```
//!
//! Every binding is a thin wrapper over a plain Rust function in [`api`], so the
//! behaviour is unit-tested natively.

use wasm_bindgen::prelude::*;

/// Plain-Rust implementations behind the JS bindings.
pub mod api {
    use serde::Serialize;

    /// Inspection result returned by [`inspect`].
    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Inspection {
        pub manifest: hivekit_core::Manifest,
        pub manifest_address: String,
        pub imports: Vec<String>,
        pub exports: Vec<String>,
        pub declared_functions: Option<Vec<String>>,
        pub abi_error: Option<String>,
        pub executable: bool,
    }

    pub fn package_wasm(
        wasm: &[u8],
        name: &str,
        language: &str,
        functions: Option<Vec<String>>,
        compiler: Option<String>,
        version: Option<String>,
        description: Option<String>,
    ) -> Result<hivekit_core::CompileResult, String> {
        hivekit_core::package_wasm(hivekit_core::PackageWasmOptions {
            name: name.into(),
            language: language.into(),
            wasm_bytes: wasm.to_vec(),
            functions,
            compiler,
            version,
            description,
        })
        .map_err(|x| format!("{x:#}"))
    }

    pub fn compile(
        source: &str,
        wasm: &[u8],
        name: &str,
        language: &str,
    ) -> Result<hivekit_core::CompileResult, String> {
        hivekit_core::compile(hivekit_core::CompileOptions {
            name: name.into(),
            language: language.into(),
            source: source.into(),
            wasm_bytes: wasm.to_vec(),
            version: None,
            description: None,
        })
        .map_err(|x| format!("{x:#}"))
    }

    pub fn inspect(hbc: &[u8]) -> Result<Inspection, String> {
        let a = hivekit_core::load_hbc(hbc).map_err(|x| format!("{x:#}"))?;
        Ok(Inspection {
            executable: a.is_executable(),
            manifest: a.manifest,
            manifest_address: a.manifest_address,
            imports: a.wasm_info.imports,
            exports: a.wasm_info.exports,
            declared_functions: a.wasm_info.declared_functions,
            abi_error: a.wasm_info.abi_error,
        })
    }

    pub fn compute_address(manifest_json: &str, wasm: &[u8]) -> Result<String, String> {
        let m = hivekit_core::Manifest::from_json(manifest_json.as_bytes())
            .map_err(|x| format!("{x:#}"))?;
        m.compute_address(wasm).map_err(|x| format!("{x:#}"))
    }
}

// Note: exported functions must not name a parameter `wasm`; the generated JS
// glue would shadow its own `wasm` instance variable with it.

fn js_err(msg: String) -> JsValue {
    js_sys::Error::new(&msg).into()
}

fn to_js<T: serde::Serialize>(v: &T) -> Result<JsValue, JsValue> {
    v.serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|x| js_err(x.to_string()))
}

/// Result of packaging a module.
#[wasm_bindgen]
pub struct CompileResult {
    inner: hivekit_core::CompileResult,
}

#[wasm_bindgen]
impl CompileResult {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.inner.manifest.name.clone()
    }

    #[wasm_bindgen(getter, js_name = manifestAddress)]
    pub fn manifest_address(&self) -> String {
        self.inner.manifest_address.clone()
    }

    /// Sorted function names; func_id = index into this list.
    #[wasm_bindgen(getter)]
    pub fn functions(&self) -> Vec<String> {
        self.inner.functions.clone()
    }

    /// The .hbc archive (Uint8Array).
    #[wasm_bindgen(getter, js_name = hbcBytes)]
    pub fn hbc_bytes(&self) -> Vec<u8> {
        self.inner.hbc_bytes.clone()
    }

    /// Canonical manifest.json text stored in the archive.
    #[wasm_bindgen(getter, js_name = manifestJson)]
    pub fn manifest_json(&self) -> String {
        String::from_utf8_lossy(&self.inner.manifest_bytes).into_owned()
    }
}

/// Package a compiled hive-wasm-v1 module into a .hbc. `functions` defaults to
/// the module's `hivekit.functions` section; if both are present they must agree.
#[wasm_bindgen(js_name = packageWasm)]
pub fn package_wasm(
    module_wasm: &[u8],
    name: &str,
    language: &str,
    functions: Option<Vec<String>>,
    compiler: Option<String>,
    version: Option<String>,
    description: Option<String>,
) -> Result<CompileResult, JsValue> {
    api::package_wasm(
        module_wasm,
        name,
        language,
        functions,
        compiler,
        version,
        description,
    )
    .map(|inner| CompileResult { inner })
    .map_err(js_err)
}

/// Package compiled wasm, discovering the function list from `source`.
/// Source text is never packaged.
#[wasm_bindgen]
pub fn compile(
    source: &str,
    module_wasm: &[u8],
    name: &str,
    language: &str,
) -> Result<CompileResult, JsValue> {
    api::compile(source, module_wasm, name, language)
        .map(|inner| CompileResult { inner })
        .map_err(js_err)
}

/// Load and verify a .hbc. Returns
/// `{manifest, manifestAddress, imports, exports, declaredFunctions, abiError, executable}`.
#[wasm_bindgen]
pub fn inspect(hbc: &[u8]) -> Result<JsValue, JsValue> {
    to_js(&api::inspect(hbc).map_err(js_err)?)
}

/// Inspect a bare module.wasm.
#[wasm_bindgen(js_name = inspectWasm)]
pub fn inspect_wasm(module_wasm: &[u8]) -> Result<JsValue, JsValue> {
    to_js(&hivekit_core::inspect_wasm(module_wasm).map_err(|x| js_err(format!("{x:#}")))?)
}

/// Content address of a manifest JSON text (manifest_address ignored) + wasm bytes.
#[wasm_bindgen(js_name = computeAddress)]
pub fn compute_address(manifest_json: &str, module_wasm: &[u8]) -> Result<String, JsValue> {
    api::compute_address(manifest_json, module_wasm).map_err(js_err)
}

/// Canonical JSON of a JSON text.
#[wasm_bindgen(js_name = canonicalJson)]
pub fn canonical_json(json_text: &str) -> Result<String, JsValue> {
    hivekit_core::canonicalize_str(json_text).map_err(|x| js_err(format!("{x:#}")))
}

/// Keccak-256 (Ethereum) as "0x" + 64 lowercase hex.
#[wasm_bindgen(js_name = keccak256Hex)]
pub fn keccak256_hex(data: &[u8]) -> String {
    hivekit_core::keccak_hex(data)
}

/// Canonical module address, or undefined if malformed.
#[wasm_bindgen(js_name = normalizeAddress)]
pub fn normalize_address(address: &str) -> Option<String> {
    hivekit_core::normalize_module_address(address)
}

/// Exported function names found in source (sorted, unique).
#[wasm_bindgen(js_name = detectFunctions)]
pub fn detect_functions(source: &str, language: &str) -> Vec<String> {
    hivekit_core::FunctionRegistry::collect(source, language)
}

/// The runtime id every manifest must declare ("hive-wasm-v1").
#[wasm_bindgen(js_name = runtimeId)]
pub fn runtime_id() -> String {
    hivekit_core::RUNTIME_ID.to_string()
}

#[cfg(test)]
mod tests {
    use super::api;

    const ECHO_HEX: &str = "0061736d0100000001140360047f7f7f7f0060017f017f60037f7f7f017e020d01046869766504656d69740000030302010205030100010607017f014180080b072303066d656d6f72790200075f5f616c6c6f6300010c5f5f686976655f656e74727900020a2a021101017f23002101230020006a240020010b1600411041062001200210002001ad4220862002ad840b0b0c010041100b066563686f6564002c046e616d650107010004656d6974021502010200016e01017002030002696401017002016c07050100026870";
    const ADDR: &str = "0x8a9321e60b20d30e14ebb65002b6cec307fdf8a93b9f6e23279c6a1b8ee1b454";

    #[test]
    fn package_inspect_address_agree() {
        let wasm = hex::decode(ECHO_HEX).unwrap();
        let r = api::package_wasm(
            &wasm,
            "vector_echo",
            "wat",
            Some(vec!["echo".into()]),
            Some("ndsr-test-vectors/1".into()),
            Some("1.0.0".into()),
            None,
        )
        .unwrap();
        assert_eq!(r.manifest_address, ADDR);
        let i = api::inspect(&r.hbc_bytes).unwrap();
        assert_eq!(i.manifest_address, ADDR);
        assert!(i.executable);
        assert_eq!(i.imports, ["hive.emit"]);
        let m = String::from_utf8(r.manifest_bytes).unwrap();
        assert_eq!(api::compute_address(&m, &wasm).unwrap(), ADDR);
    }

    #[test]
    fn errors_are_strings() {
        assert!(api::inspect(b"nope").is_err());
        assert!(api::compile("", b"", "x", "rust")
            .unwrap_err()
            .contains("module.wasm"));
        assert!(api::package_wasm(
            b"\0asm\x01\0\0\0",
            "x",
            "rust",
            Some(vec!["a".into()]),
            None,
            None,
            None
        )
        .unwrap_err()
        .contains("ABI"));
    }
}
