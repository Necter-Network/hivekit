//! HiveKit core: the shared, language-neutral half of every HiveKit SDK.
//!
//! * [`manifest`]: the `hive-wasm-v1` manifest schema and its content address
//!   (`keccak256(canonical_json(manifest − manifest_address) ‖ module.wasm)`).
//! * [`canonical`]: canonical JSON and Keccak-256 (docs/HBC_SPEC.md §4).
//! * [`abi`]: static checks of a `module.wasm` against the guest ABI (imports/exports).
//! * [`hbc`]: reading and writing `.hbc` containers.
//! * [`compiler`]: packaging compiled wasm into a `.hbc`.
//! * [`registry`]: discovering exported function names from source.
//!
//! The normative definition of all of the above is NDSR's `docs/HBC_SPEC.md`.

pub mod abi;
pub mod canonical;
pub mod compiler;
pub mod hbc;
pub mod manifest;
pub mod registry;

pub use abi::{inspect_wasm, validate_wasm, WasmInfo, FUNCTIONS_SECTION};
pub use canonical::{canonical_json, canonicalize_str, keccak_hex, normalize_module_address};
pub use compiler::{compile, package_wasm, CompileOptions, CompileResult, PackageWasmOptions};
pub use hbc::{load_hbc, load_hbc_strict, write_hbc, Artifact};
pub use manifest::{compute_address, Manifest, RUNTIME_ID};
pub use registry::FunctionRegistry;
