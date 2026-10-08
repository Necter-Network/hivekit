//! hive-wasm-v1 guest ABI (docs/HBC_SPEC.md §6) for interpreter runtimes.
//!
//! Provides the `memory`/`__alloc` exports, safe wrappers over the host imports,
//! packing helpers, and access to the embedded script blob.
//!
//! The runtime crate must export `__hive_entry` itself (it calls [`entry`]).
//!
//! # Embedded script
//! The runtime `.wasm` is built once and committed. The SDK packager appends the
//! user's script as a new active data segment placed at the end of the initial
//! linear memory (raising the memory minimum accordingly) and writes its address
//! and length into [`HIVEKIT_SCRIPT_REF`], which it locates by its 16-byte magic.
//! The allocator only obtains memory through `memory.grow`, so the appended
//! pages are never handed out.

#![allow(clippy::missing_safety_doc)]

use std::alloc::{alloc, Layout};

/// Location of the embedded script, patched in place by the packager.
/// `ptr == len == u32::MAX` means "no script embedded" (an unpackaged runtime).
#[repr(C)]
pub struct ScriptRef {
    pub magic: [u8; 16],
    pub ptr: u32,
    pub len: u32,
}

#[no_mangle]
#[used]
pub static mut HIVEKIT_SCRIPT_REF: ScriptRef =
    ScriptRef { magic: *b"HIVEKIT-SCRIPT-1", ptr: u32::MAX, len: u32::MAX };

/// The embedded script blob, or `None` for an unpackaged runtime.
pub fn script() -> Option<&'static [u8]> {
    unsafe {
        let r = std::ptr::addr_of!(HIVEKIT_SCRIPT_REF);
        let ptr = std::ptr::read_volatile(std::ptr::addr_of!((*r).ptr));
        let len = std::ptr::read_volatile(std::ptr::addr_of!((*r).len));
        if ptr == u32::MAX && len == u32::MAX {
            return None;
        }
        Some(std::slice::from_raw_parts(ptr as usize as *const u8, len as usize))
    }
}

/// Script blob layout: `name\nname\n...\0source`. Names are the sorted manifest
/// functions (func_id = index).
pub struct Script {
    pub functions: Vec<String>,
    pub source: String,
}

pub fn load_script() -> Result<Script, String> {
    let blob = script().ok_or_else(|| "no script embedded in this runtime".to_string())?;
    let nul = blob.iter().position(|b| *b == 0).ok_or("malformed script blob")?;
    let names = std::str::from_utf8(&blob[..nul]).map_err(|_| "malformed script blob")?;
    let source = std::str::from_utf8(&blob[nul + 1..]).map_err(|_| "script is not UTF-8")?;
    let functions = if names.is_empty() { vec![] } else { names.split('\n').map(str::to_string).collect() };
    Ok(Script { functions, source: source.to_string() })
}

mod imp {
    #[link(wasm_import_module = "hive")]
    extern "C" {
        pub fn call(ap: *const u8, al: usize, fp: *const u8, fl: usize, ip: *const u8, il: usize) -> i64;
        pub fn emit(np: *const u8, nl: usize, dp: *const u8, dl: usize);
        pub fn abort(p: *const u8, l: usize);
    }
    #[link(wasm_import_module = "storage")]
    extern "C" {
        pub fn get(kp: *const u8, kl: usize) -> i64;
        pub fn set(kp: *const u8, kl: usize, vp: *const u8, vl: usize);
        pub fn del(kp: *const u8, kl: usize);
    }
    #[link(wasm_import_module = "console")]
    extern "C" {
        pub fn log(p: *const u8, l: usize);
    }
    #[link(wasm_import_module = "crypto")]
    extern "C" {
        pub fn hash(p: *const u8, l: usize) -> i64;
    }
}

/// `__alloc(len) -> ptr`: a real linear-memory pointer to `len` writable bytes.
#[no_mangle]
pub extern "C" fn __alloc(len: i32) -> i32 {
    let n = (len as u32 as usize).max(1);
    unsafe { alloc(Layout::from_size_align_unchecked(n, 1)) as usize as i32 }
}

/// Take ownership of bytes the host placed in memory via `__alloc`.
unsafe fn take(ptr: u32, len: u32) -> Vec<u8> {
    if len == 0 {
        return Vec::new();
    }
    Vec::from_raw_parts(ptr as usize as *mut u8, len as usize, len as usize)
}

/// Unpack a host-returned packed value; `None` for 0 (absent / empty).
fn unpack(v: i64) -> Option<Vec<u8>> {
    if v <= 0 {
        return None;
    }
    let ptr = (v as u64 >> 32) as u32;
    let len = (v as u64 & 0xffff_ffff) as u32;
    Some(unsafe { take(ptr, len) })
}

/// Pack an owned byte buffer as a `(ptr << 32) | len` return value.
pub fn pack(bytes: Vec<u8>) -> i64 {
    if bytes.is_empty() {
        return 0;
    }
    let b = bytes.into_boxed_slice();
    let len = b.len() as u64;
    let ptr = Box::into_raw(b) as *mut u8 as usize as u64;
    ((ptr << 32) | len) as i64
}

pub fn storage_get(key: &str) -> Option<String> {
    let v = unsafe { imp::get(key.as_ptr(), key.len()) };
    unpack(v).map(|b| String::from_utf8_lossy(&b).into_owned())
}

pub fn storage_set(key: &str, value: &str) {
    unsafe { imp::set(key.as_ptr(), key.len(), value.as_ptr(), value.len()) }
}

pub fn storage_del(key: &str) {
    unsafe { imp::del(key.as_ptr(), key.len()) }
}

pub fn emit(name: &str, data_json: &str) {
    unsafe { imp::emit(name.as_ptr(), name.len(), data_json.as_ptr(), data_json.len()) }
}

pub fn log(msg: &str) {
    let b = msg.as_bytes();
    let b = &b[..b.len().min(4096)];
    unsafe { imp::log(b.as_ptr(), b.len()) }
}

/// keccak256 of `data` as `0x` + 64 hex.
pub fn hash(data: &[u8]) -> String {
    let v = unsafe { imp::hash(data.as_ptr(), data.len()) };
    unpack(v).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// Cross-module call: `Ok(output)` or `Err(code)` (HBC_SPEC §6.5).
pub fn call(address: &str, function: &str, input: &str) -> Result<String, i64> {
    let r = unsafe {
        imp::call(address.as_ptr(), address.len(), function.as_ptr(), function.len(), input.as_ptr(), input.len())
    };
    if r < 0 {
        return Err(r);
    }
    Ok(unpack(r).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default())
}

/// Fail the whole call with `msg`. Never returns.
pub fn abort(msg: &str) -> ! {
    let b = msg.as_bytes();
    let b = &b[..b.len().min(1024)];
    unsafe { imp::abort(b.as_ptr(), b.len()) };
    // The host traps on abort; this is never reached.
    core::arch::wasm32::unreachable()
}

/// Shared `__hive_entry` body: decode input, run `f`, pack the output.
/// `f` returns `Err(msg)` to fail the call with a message.
pub fn entry(ptr: i32, len: i32, f: impl FnOnce(String) -> Result<String, String>) -> i64 {
    let bytes = unsafe { take(ptr as u32, len as u32) };
    let input = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => abort("input is not valid UTF-8"),
    };
    match f(input) {
        Ok(out) => pack(out.into_bytes()),
        Err(msg) => abort(&msg),
    }
}

/// Route panics to `hive.abort` so the receipt carries a readable error.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "runtime panic".to_string()
        };
        abort(&format!("runtime panic: {msg}"));
    }));
}
