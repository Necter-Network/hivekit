//! The `.hbc` container (HBC_SPEC.md §2): a ZIP with exactly `manifest.json`
//! and `module.wasm`.

use crate::abi::{inspect_wasm, WasmInfo, MAX_WASM_BYTES};
use crate::manifest::Manifest;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::io::{Read, Write};

/// Maximum size of a whole `.hbc` file.
pub const MAX_HBC_BYTES: usize = 16 * 1024 * 1024;
/// Maximum size of `manifest.json`.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

const MANIFEST_ENTRY: &str = "manifest.json";
const WASM_ENTRY: &str = "module.wasm";

/// Write a deterministic `.hbc`: entries `manifest.json` then `module.wasm`,
/// stored (no compression), fixed 1980-01-01 timestamps.
pub fn write_hbc(manifest_json: &[u8], wasm: &[u8]) -> Result<Vec<u8>> {
    use zip::write::{SimpleFileOptions, ZipWriter};
    use zip::{CompressionMethod, DateTime};

    if manifest_json.len() > MAX_MANIFEST_BYTES {
        bail!("manifest.json exceeds {MAX_MANIFEST_BYTES} bytes");
    }
    if wasm.len() > MAX_WASM_BYTES {
        bail!(
            "module.wasm is {} bytes; the limit is {MAX_WASM_BYTES}",
            wasm.len()
        );
    }
    let mut buf = Vec::with_capacity(manifest_json.len() + wasm.len() + 256);
    {
        let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(DateTime::default())
            .unix_permissions(0o644);
        zip.start_file(MANIFEST_ENTRY, opts)?;
        zip.write_all(manifest_json)?;
        zip.start_file(WASM_ENTRY, opts)?;
        zip.write_all(wasm)?;
        zip.finish()?;
    }
    if buf.len() > MAX_HBC_BYTES {
        bail!(
            ".hbc would be {} bytes; the limit is {MAX_HBC_BYTES}",
            buf.len()
        );
    }
    Ok(buf)
}

/// A loaded and verified `.hbc` artifact.
#[derive(Debug, Clone, Serialize)]
pub struct Artifact {
    /// The manifest as declared (including `manifest_address` if present).
    pub manifest: Manifest,
    /// The computed content address (equal to the declared one when present).
    pub manifest_address: String,
    /// Raw `manifest.json` bytes as stored in the archive.
    #[serde(skip)]
    pub manifest_json: Vec<u8>,
    /// `module.wasm` bytes.
    #[serde(skip)]
    pub wasm: Vec<u8>,
    /// Static inspection of the wasm (imports, exports, ABI conformance).
    pub wasm_info: WasmInfo,
}

impl Artifact {
    /// True if the module would be accepted for execution by NDSR.
    pub fn is_executable(&self) -> bool {
        self.wasm_info.abi_error.is_none()
    }
}

fn read_entry<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    max: usize,
) -> Result<Vec<u8>> {
    let f = zip
        .by_name(name)
        .with_context(|| format!(".hbc is missing {name}"))?;
    let mut out = Vec::new();
    // Enforce the limit while reading, regardless of the declared size.
    f.take(max as u64 + 1).read_to_end(&mut out)?;
    if out.len() > max {
        bail!("{name} exceeds {max} bytes");
    }
    Ok(out)
}

/// Parse and verify a `.hbc` exactly as NDSR's loader does: container rules,
/// strict manifest schema, mandatory address verification. ABI conformance of
/// the wasm is reported in [`Artifact::wasm_info`]; use [`load_hbc_strict`] to
/// turn it into an error.
pub fn load_hbc(hbc: &[u8]) -> Result<Artifact> {
    if hbc.len() > MAX_HBC_BYTES {
        bail!(".hbc exceeds {MAX_HBC_BYTES} bytes");
    }
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(hbc)).context("failed to open .hbc as zip")?;
    let mut names: Vec<String> = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let name = zip.by_index_raw(i)?.name().to_string();
        if names.contains(&name) {
            bail!("duplicate entry {name:?} in .hbc");
        }
        names.push(name);
    }
    for n in &names {
        if n != MANIFEST_ENTRY && n != WASM_ENTRY {
            if n.starts_with("module.") {
                bail!(".hbc contains {n:?}: source/WASI artifacts are not executable; only hive-wasm-v1 module.wasm is");
            }
            bail!(
                "unexpected entry {n:?} in .hbc (only manifest.json and module.wasm are allowed)"
            );
        }
    }
    let manifest_json = read_entry(&mut zip, MANIFEST_ENTRY, MAX_MANIFEST_BYTES)?;
    let wasm = read_entry(&mut zip, WASM_ENTRY, MAX_WASM_BYTES)?;
    let manifest = Manifest::from_json(&manifest_json)?;
    let wasm_info = inspect_wasm(&wasm)?;
    let manifest_address = manifest.verify_address(&wasm)?;
    Ok(Artifact {
        manifest,
        manifest_address,
        manifest_json,
        wasm,
        wasm_info,
    })
}

/// [`load_hbc`] plus: the wasm must satisfy the hive-wasm-v1 ABI.
pub fn load_hbc_strict(hbc: &[u8]) -> Result<Artifact> {
    let a = load_hbc(hbc)?;
    if let Some(e) = &a.wasm_info.abi_error {
        bail!("module.wasm violates the hive-wasm-v1 ABI: {e}");
    }
    Ok(a)
}
