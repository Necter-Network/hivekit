//! The `manifest.json` schema of a `hive-wasm-v1` module (HBC_SPEC.md §3) and its
//! content address (§4).

use crate::canonical::{canonical_json, keccak256_concat, reject_duplicate_keys};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The only runtime / guest ABI identifier NDSR executes.
pub const RUNTIME_ID: &str = "hive-wasm-v1";
/// Maximum number of exported functions.
pub const MAX_FUNCTIONS: usize = 256;
/// Every key a manifest may contain. Anything else is rejected.
pub const ALLOWED_KEYS: &[&str] = &[
    "compiler",
    "description",
    "functions",
    "language",
    "manifest_address",
    "name",
    "runtime",
    "version",
];

/// Default `compiler` string written by this crate.
pub fn default_compiler() -> String {
    format!("hivekit-core/{}", env!("CARGO_PKG_VERSION"))
}

/// A `hive-wasm-v1` manifest.
///
/// `manifest_address` is carried alongside but is never part of the hashed
/// document: [`Manifest::canonical_bytes`] always omits it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub language: String,
    pub compiler: String,
    pub runtime: String,
    pub functions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_address: Option<String>,
}

impl Manifest {
    /// Build a validated manifest. `functions` is sorted here; duplicates are an error.
    pub fn new(
        name: &str,
        language: &str,
        compiler: &str,
        mut functions: Vec<String>,
    ) -> Result<Self> {
        functions.sort();
        let m = Self {
            name: name.into(),
            language: language.into(),
            compiler: compiler.into(),
            runtime: RUNTIME_ID.into(),
            functions,
            version: None,
            description: None,
            manifest_address: None,
        };
        m.validate()?;
        Ok(m)
    }

    /// Enforce every schema rule of HBC_SPEC.md §3 (except the address check,
    /// which needs the wasm bytes: see [`Manifest::verify_address`]).
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() || self.name.len() > 128 {
            bail!("manifest.name must be 1..=128 bytes");
        }
        if !is_language(&self.language) {
            bail!(
                "manifest.language must match [a-z0-9_+-]{{1,32}}, got {:?}",
                self.language
            );
        }
        if self.compiler.is_empty() || self.compiler.len() > 128 {
            bail!("manifest.compiler must be 1..=128 bytes");
        }
        if self.runtime != RUNTIME_ID {
            bail!(
                "manifest.runtime is {:?}; only {RUNTIME_ID:?} modules are executable",
                self.runtime
            );
        }
        if self.functions.is_empty() || self.functions.len() > MAX_FUNCTIONS {
            bail!("manifest.functions must list 1..={MAX_FUNCTIONS} functions");
        }
        for f in &self.functions {
            if !is_function_name(f) {
                bail!("invalid function name {f:?} (must match [A-Za-z_][A-Za-z0-9_]{{0,63}})");
            }
        }
        if self
            .functions
            .windows(2)
            .any(|w| w[0].as_bytes() >= w[1].as_bytes())
        {
            bail!("manifest.functions must be sorted ascending (bytewise) with no duplicates");
        }
        for (k, v) in [
            ("version", &self.version),
            ("description", &self.description),
        ] {
            if v.as_ref().is_some_and(|s| s.len() > 1024) {
                bail!("manifest.{k} must be at most 1024 bytes");
            }
        }
        Ok(())
    }

    /// Strictly parse a `manifest.json` document: no duplicate keys, no floats,
    /// no unknown keys, every schema rule enforced. Does not check the address.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        reject_duplicate_keys(bytes).context("manifest.json")?;
        let value: Value =
            serde_json::from_slice(bytes).context("manifest.json is not valid JSON")?;
        canonical_json(&value)
            .context("manifest.json must not contain floats or unsafe integers")?;
        let Value::Object(obj) = value else {
            bail!("manifest.json must be a JSON object")
        };
        Self::from_map(obj)
    }

    fn from_map(obj: Map<String, Value>) -> Result<Self> {
        for k in obj.keys() {
            if !ALLOWED_KEYS.contains(&k.as_str()) {
                bail!("manifest.json has unknown field {k:?} (timestamps and extra metadata are not allowed)");
            }
        }
        let m: Manifest =
            serde_json::from_value(Value::Object(obj)).context("manifest.json schema")?;
        m.validate()?;
        Ok(m)
    }

    /// Canonical JSON of the manifest **without** `manifest_address` (the hashed form).
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        let mut v = serde_json::to_value(self)?;
        if let Value::Object(m) = &mut v {
            m.remove("manifest_address");
        }
        canonical_json(&v)
    }

    /// Canonical JSON including `manifest_address` if set (what goes into `manifest.json`).
    pub fn to_canonical_json(&self) -> Result<Vec<u8>> {
        canonical_json(&serde_json::to_value(self)?)
    }

    /// `keccak256(canonical_json(manifest − manifest_address) || wasm)`.
    pub fn compute_address(&self, wasm: &[u8]) -> Result<String> {
        let m = self.canonical_bytes()?;
        Ok(format!("0x{}", hex::encode(keccak256_concat(&[&m, wasm]))))
    }

    /// Compute the address and store it in `manifest_address`.
    pub fn seal(&mut self, wasm: &[u8]) -> Result<String> {
        let a = self.compute_address(wasm)?;
        self.manifest_address = Some(a.clone());
        Ok(a)
    }

    /// Compute the address; if `manifest_address` is declared it must match exactly.
    pub fn verify_address(&self, wasm: &[u8]) -> Result<String> {
        let computed = self.compute_address(wasm)?;
        if let Some(d) = &self.manifest_address {
            if *d != computed {
                bail!("manifest_address mismatch: declared={d} computed={computed}");
            }
        }
        Ok(computed)
    }

    /// `func_id` = index of `name` in the sorted function list.
    pub fn func_id(&self, name: &str) -> Option<u32> {
        self.functions
            .binary_search_by(|f| f.as_bytes().cmp(name.as_bytes()))
            .ok()
            .map(|i| i as u32)
    }
}

/// Content address of a manifest (any `manifest_address` key is ignored) + wasm bytes.
pub fn compute_address(manifest: &Manifest, wasm: &[u8]) -> Result<String> {
    manifest.compute_address(wasm)
}

/// `[A-Za-z_][A-Za-z0-9_]{0,63}`
pub fn is_function_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// `[a-z0-9_+-]{1,32}`
pub fn is_language(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_+-".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Manifest {
        Manifest::new("t", "rust", "hivec-rs/0", vec!["b".into(), "a".into()]).unwrap()
    }

    #[test]
    fn functions_are_sorted_and_unique() {
        assert_eq!(m().functions, ["a", "b"]);
        assert!(Manifest::new("t", "rust", "c", vec!["a".into(), "a".into()]).is_err());
        // Uppercase sorts before lowercase bytewise.
        let mm = Manifest::new("t", "rust", "c", vec!["b".into(), "Z".into()]).unwrap();
        assert_eq!(mm.functions, ["Z", "b"]);
        assert_eq!(mm.func_id("b"), Some(1));
        assert_eq!(mm.func_id("nope"), None);
    }

    #[test]
    fn rejects_bad_fields() {
        assert!(Manifest::new("t", "Rust", "c", vec!["a".into()]).is_err());
        assert!(Manifest::new("t", "rust", "c", vec!["1a".into()]).is_err());
        assert!(Manifest::new("t", "rust", "c", vec![]).is_err());
        assert!(Manifest::new("", "rust", "c", vec!["a".into()]).is_err());
    }

    #[test]
    fn strict_parse() {
        let ok = br#"{"compiler":"c","functions":["a"],"language":"rust","name":"n","runtime":"hive-wasm-v1"}"#;
        Manifest::from_json(ok).unwrap();
        for bad in [
            &br#"{"compiler":"c","functions":["a"],"language":"rust","name":"n","runtime":"hive-wasm-v1","created_at":"x"}"#[..],
            br#"{"compiler":"c","functions":["a"],"language":"rust","name":"n","runtime":"wasm32-wasi"}"#,
            br#"{"compiler":"c","functions":["b","a"],"language":"rust","name":"n","runtime":"hive-wasm-v1"}"#,
            br#"{"compiler":"c","functions":["a"],"language":"rust","name":"n","name":"n","runtime":"hive-wasm-v1"}"#,
            br#"{"compiler":"c","functions":["a"],"language":"rust","name":"n","runtime":"hive-wasm-v1","version":1}"#,
            br#"{"compiler":"c","functions":["a"],"language":"rust","name":"n","runtime":"hive-wasm-v1","nrc1":{}}"#,
        ] {
            assert!(Manifest::from_json(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn address_ignores_manifest_address_key() {
        let mut a = m();
        let addr = a.seal(b"\0asm\x01\0\0\0").unwrap();
        assert_eq!(a.compute_address(b"\0asm\x01\0\0\0").unwrap(), addr);
        assert!(a.verify_address(b"\0asm\x01\0\0\0").is_ok());
        assert!(a.verify_address(b"\0asm\x01\0\0\x01").is_err());
        assert!(!String::from_utf8(a.canonical_bytes().unwrap())
            .unwrap()
            .contains("manifest_address"));
    }
}
