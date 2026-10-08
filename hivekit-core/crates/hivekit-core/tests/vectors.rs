//! Known-answer tests against NDSR's `docs/test-vectors.json`.
//!
//! `tests/data/test-vectors.json` is a copy of that file. When the NDSR source
//! tree is checked out next to this repository the copy is also compared with
//! the original, so drift is caught.

use base64::Engine;
use hivekit_core::canonical::{canonical_json, keccak_hex};
use hivekit_core::{load_hbc_strict, package_wasm, Manifest, PackageWasmOptions};
use serde_json::Value;

fn vectors() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/test-vectors.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn copy_matches_upstream_if_present() {
    let up = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|a| a.join("docs/test-vectors.json"))
        .find(|p| p.exists());
    if let Some(up) = up {
        let upstream: Value = serde_json::from_str(&std::fs::read_to_string(&up).unwrap()).unwrap();
        assert_eq!(
            upstream,
            vectors(),
            "{} changed; refresh tests/data",
            up.display()
        );
    }
}

#[test]
fn canonical_json_vectors() {
    let v = vectors();
    let cases = v["canonical_json"].as_array().unwrap();
    assert!(!cases.is_empty());
    for c in cases {
        let got = String::from_utf8(canonical_json(&c["input"]).unwrap()).unwrap();
        assert_eq!(got, c["canonical"].as_str().unwrap());
    }
    // The receipt vectors are canonical JSON too.
    let r = &v["execution_receipt"];
    let ev = String::from_utf8(canonical_json(&r["events"]).unwrap()).unwrap();
    assert_eq!(ev, r["events_canonical_json"].as_str().unwrap());
}

#[test]
fn keccak_vectors() {
    for c in vectors()["keccak256"].as_array().unwrap() {
        assert_eq!(
            keccak_hex(c["input_utf8"].as_str().unwrap().as_bytes()),
            c["hash"].as_str().unwrap()
        );
    }
}

#[test]
fn manifest_address_vector() {
    let v = vectors();
    let ma = &v["manifest_address"];
    let wasm = hex::decode(ma["wasm_hex"].as_str().unwrap()).unwrap();
    let manifest: Manifest = serde_json::from_value(ma["manifest"].clone()).unwrap();
    manifest.validate().unwrap();
    assert_eq!(
        String::from_utf8(manifest.canonical_bytes().unwrap()).unwrap(),
        ma["canonical_manifest"].as_str().unwrap()
    );
    let want = ma["manifest_address"].as_str().unwrap();
    assert_eq!(manifest.compute_address(&wasm).unwrap(), want);
    assert_eq!(v["execution"]["module_address"].as_str().unwrap(), want);

    // The reference .hbc loads and verifies.
    let hbc = base64::engine::general_purpose::STANDARD
        .decode(ma["hbc_base64"].as_str().unwrap())
        .unwrap();
    let art = load_hbc_strict(&hbc).unwrap();
    assert_eq!(art.manifest_address, want);
    assert_eq!(art.wasm, wasm);

    // Packaging the same wasm + manifest fields reproduces the address and,
    // because the archive is deterministic, the exact reference bytes.
    let r = package_wasm(PackageWasmOptions {
        name: manifest.name.clone(),
        language: manifest.language.clone(),
        wasm_bytes: wasm,
        functions: Some(manifest.functions.clone()),
        compiler: Some(manifest.compiler.clone()),
        version: manifest.version.clone(),
        description: None,
    })
    .unwrap();
    assert_eq!(r.manifest_address, want);
    assert_eq!(
        r.hbc_bytes, hbc,
        "deterministic archive differs from the reference"
    );
}

#[test]
fn tampered_artifacts_rejected() {
    let v = vectors();
    let ma = &v["manifest_address"];
    let wasm = hex::decode(ma["wasm_hex"].as_str().unwrap()).unwrap();
    let base = |m: &str| hivekit_core::write_hbc(m.as_bytes(), &wasm).unwrap();
    let canon = ma["canonical_manifest"].as_str().unwrap();
    // Wrong declared address.
    let bad_addr = canon.replace(
        "\"name\"",
        &format!("\"manifest_address\":\"0x{}\",\"name\"", "00".repeat(32)),
    );
    assert!(load_hbc_strict(&base(&bad_addr)).is_err());
    // Unknown key.
    let extra = canon.replace("\"name\"", "\"created_at\":\"2026\",\"name\"");
    assert!(load_hbc_strict(&base(&extra)).is_err());
    // Without manifest_address it still loads and computes the address.
    assert_eq!(
        load_hbc_strict(&base(canon)).unwrap().manifest_address,
        ma["manifest_address"].as_str().unwrap()
    );
}
