//! Size limits from HBC_SPEC.md §2: `module.wasm` ≤ 12 MiB, `.hbc` ≤ 16 MiB.

use hivekit_core::abi::MAX_WASM_BYTES;
use hivekit_core::hbc::MAX_HBC_BYTES;
use hivekit_core::{load_hbc_strict, package_wasm, PackageWasmOptions};
use serde_json::Value;
use std::io::Write;

fn vector_wasm() -> Vec<u8> {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/test-vectors.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    hex::decode(v["manifest_address"]["wasm_hex"].as_str().unwrap()).unwrap()
}

/// `wasm` plus a trailing custom section so the result is exactly `total` bytes.
fn pad_to(mut wasm: Vec<u8>, total: usize) -> Vec<u8> {
    let size = (total - wasm.len() - 5) as u32; // section id + 4-byte LEB size
    assert!((1 << 21..1 << 28).contains(&size));
    wasm.push(0);
    wasm.extend_from_slice(&[
        (size & 0x7f) as u8 | 0x80,
        ((size >> 7) & 0x7f) as u8 | 0x80,
        ((size >> 14) & 0x7f) as u8 | 0x80,
        (size >> 21) as u8,
    ]);
    wasm.extend_from_slice(b"\x03pad");
    wasm.resize(total, 0);
    wasm
}

fn package(wasm: Vec<u8>) -> anyhow::Result<hivekit_core::CompileResult> {
    package_wasm(PackageWasmOptions {
        name: "big".into(),
        language: "wat".into(),
        wasm_bytes: wasm,
        functions: Some(vec!["echo".into()]),
        compiler: Some("limits-test/1".into()),
        version: None,
        description: None,
    })
}

#[test]
fn limits_match_the_spec() {
    assert_eq!(MAX_WASM_BYTES, 12 * 1024 * 1024);
    assert_eq!(MAX_HBC_BYTES, 16 * 1024 * 1024);
}

#[test]
fn module_wasm_at_the_limit_is_accepted() {
    let wasm = pad_to(vector_wasm(), MAX_WASM_BYTES);
    let r = package(wasm.clone()).unwrap();
    assert!(r.hbc_bytes.len() <= MAX_HBC_BYTES);
    let a = load_hbc_strict(&r.hbc_bytes).unwrap();
    assert_eq!(a.wasm, wasm);
    assert_eq!(a.manifest_address, r.manifest_address);
}

#[test]
fn module_wasm_over_the_limit_is_rejected() {
    let wasm = pad_to(vector_wasm(), MAX_WASM_BYTES + 1);
    assert!(package(wasm.clone()).is_err());
    assert!(hivekit_core::write_hbc(b"{}", &wasm).is_err());

    // A hand-built archive that skips the packager's check is rejected on load.
    use zip::write::{SimpleFileOptions, ZipWriter};
    let mut buf = Vec::new();
    {
        let mut z = ZipWriter::new(std::io::Cursor::new(&mut buf));
        let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("manifest.json", o).unwrap();
        z.write_all(br#"{"compiler":"c","functions":["echo"],"language":"wat","name":"big","runtime":"hive-wasm-v1"}"#)
            .unwrap();
        z.start_file("module.wasm", o).unwrap();
        z.write_all(&wasm).unwrap();
        z.finish().unwrap();
    }
    let err = load_hbc_strict(&buf).unwrap_err().to_string();
    assert!(err.contains("module.wasm exceeds"), "{err}");
}
