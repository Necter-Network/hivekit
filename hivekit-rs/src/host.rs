//! Host interface (HBC_SPEC.md §6.4). On wasm32 these call the real NDSR host
//! imports; natively they run against an in-process mock ([`testing`]).

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

/// Error codes returned by `hive.call` (HBC_SPEC.md §6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// `-1`: no module with that address.
    ModuleNotFound,
    /// `-2`: the callee's manifest does not list that function.
    FunctionNotFound,
    /// `-3`: the callee trapped, aborted or returned an error. Its writes and events were discarded.
    CalleeFailed,
    /// `-5`: the call would exceed the maximum call depth.
    DepthExceeded,
    /// `-6`: the address is not `0x` + 64 lowercase hex.
    BadAddress,
    /// The callee's output was not valid JSON (only from [`call_json`]).
    BadOutput(String),
    /// Any other negative code.
    Other(i64),
}

impl CallError {
    pub fn from_code(code: i64) -> Self {
        match code {
            -1 => CallError::ModuleNotFound,
            -2 => CallError::FunctionNotFound,
            -3 => CallError::CalleeFailed,
            -5 => CallError::DepthExceeded,
            -6 => CallError::BadAddress,
            c => CallError::Other(c),
        }
    }

    /// The ABI code (`BadOutput` has none and maps to 0).
    pub fn code(&self) -> i64 {
        match self {
            CallError::ModuleNotFound => -1,
            CallError::FunctionNotFound => -2,
            CallError::CalleeFailed => -3,
            CallError::DepthExceeded => -5,
            CallError::BadAddress => -6,
            CallError::BadOutput(_) => 0,
            CallError::Other(c) => *c,
        }
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::ModuleNotFound => f.write_str("hive.call: module not found"),
            CallError::FunctionNotFound => f.write_str("hive.call: function not found"),
            CallError::CalleeFailed => f.write_str("hive.call: callee failed"),
            CallError::DepthExceeded => f.write_str("hive.call: call depth exceeded"),
            CallError::BadAddress => f.write_str("hive.call: malformed module address"),
            CallError::BadOutput(e) => write!(f, "hive.call: callee output is not JSON: {e}"),
            CallError::Other(c) => write!(f, "hive.call: error code {c}"),
        }
    }
}

impl std::error::Error for CallError {}

/// Canonicalize `0x`/`0X`/`hive:` + 64 hex to lowercase `0x…`; anything else
/// is passed through unchanged (the host then answers [`CallError::BadAddress`]).
fn normalize_address(s: &str) -> std::borrow::Cow<'_, str> {
    let t = s.trim();
    let t = t.strip_prefix("hive:").unwrap_or(t);
    if t.len() == 66
        && (t.starts_with("0x") || t.starts_with("0X"))
        && t[2..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        std::borrow::Cow::Owned(format!("0x{}", t[2..].to_ascii_lowercase()))
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

// ── wasm32: the real host ──────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod sys {
    #[link(wasm_import_module = "hive")]
    extern "C" {
        #[link_name = "call"]
        pub fn hive_call(
            addr_ptr: i32,
            addr_len: i32,
            fn_ptr: i32,
            fn_len: i32,
            in_ptr: i32,
            in_len: i32,
        ) -> i64;
        #[link_name = "emit"]
        pub fn hive_emit(name_ptr: i32, name_len: i32, data_ptr: i32, data_len: i32);
        #[link_name = "abort"]
        pub fn hive_abort(msg_ptr: i32, msg_len: i32);
    }
    #[link(wasm_import_module = "storage")]
    extern "C" {
        #[link_name = "get"]
        pub fn storage_get(key_ptr: i32, key_len: i32) -> i64;
        #[link_name = "set"]
        pub fn storage_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32);
        #[link_name = "del"]
        pub fn storage_del(key_ptr: i32, key_len: i32);
    }
    #[link(wasm_import_module = "console")]
    extern "C" {
        #[link_name = "log"]
        pub fn console_log(ptr: i32, len: i32);
    }
    #[link(wasm_import_module = "crypto")]
    extern "C" {
        #[link_name = "hash"]
        pub fn crypto_hash(ptr: i32, len: i32) -> i64;
    }

    pub fn p(b: &[u8]) -> (i32, i32) {
        (b.as_ptr() as usize as i32, b.len() as i32)
    }

    /// Reclaim a packed host result (allocated through our `__alloc`).
    pub fn take(packed: i64) -> Vec<u8> {
        let (ptr, len) = crate::__rt::unpack(packed);
        // SAFETY: the host allocated exactly `len` bytes via __alloc and copied the result there.
        unsafe { crate::__rt::take_buffer(ptr, len) }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let msg = match info.payload().downcast_ref::<&str>() {
                Some(s) => s.to_string(),
                None => match info.payload().downcast_ref::<String>() {
                    Some(s) => s.clone(),
                    None => "panic".to_string(),
                },
            };
            let msg = match info.location() {
                Some(l) => format!("panicked at {}:{}: {msg}", l.file(), l.line()),
                None => format!("panicked: {msg}"),
            };
            abort(&msg);
        }));
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_panic_hook() {}

/// Fail the current call with `msg` (`hive.abort`). Nothing the call wrote is
/// committed and its events are dropped. Never returns.
pub fn abort(msg: &str) -> ! {
    #[cfg(target_arch = "wasm32")]
    {
        let (p, l) = sys::p(msg.as_bytes());
        // SAFETY: valid (ptr, len) into our memory.
        unsafe { sys::hive_abort(p, l) };
        core::arch::wasm32::unreachable()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::panic::panic_any(testing::Abort(msg.to_string()))
    }
}

/// Debug log on the node (`console.log`); no consensus effect. At most 4 KiB is kept.
pub fn log(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        let (p, l) = sys::p(msg.as_bytes());
        // SAFETY: valid (ptr, len) into our memory.
        unsafe { sys::console_log(p, l) }
    }
    #[cfg(not(target_arch = "wasm32"))]
    testing::with(|h| h.logs.push(msg.to_string()))
}

/// Keccak-256 (Ethereum) of `data` as `0x` + 64 lowercase hex (`crypto.hash`).
pub fn hash(data: &[u8]) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let (p, l) = sys::p(data);
        // SAFETY: valid (ptr, len) into our memory.
        let r = unsafe { sys::crypto_hash(p, l) };
        String::from_utf8(sys::take(r)).unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use tiny_keccak::{Hasher, Keccak};
        let mut k = Keccak::v256();
        k.update(data);
        let mut out = [0u8; 32];
        k.finalize(&mut out);
        let mut s = String::with_capacity(66);
        s.push_str("0x");
        for b in out {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }
}

/// Append an event `{name, data}` to this call's event list (`hive.emit`).
///
/// `name` must match `[A-Za-z0-9_.:-]{1,64}`; `data` is serialized to JSON and
/// must be canonical-JSON compatible: **no floats**, integers within ±(2^53−1).
/// Otherwise the host traps the call.
pub fn emit<T: Serialize + ?Sized>(name: &str, data: &T) {
    let json = match serde_json::to_vec(data) {
        Ok(j) => j,
        Err(e) => abort(&format!("emit {name}: {e}")),
    };
    emit_raw(name, &json)
}

/// [`emit`] with pre-serialized JSON bytes.
pub fn emit_raw(name: &str, json: &[u8]) {
    #[cfg(target_arch = "wasm32")]
    {
        let (np, nl) = sys::p(name.as_bytes());
        let (dp, dl) = sys::p(json);
        // SAFETY: valid (ptr, len) pairs into our memory.
        unsafe { sys::hive_emit(np, nl, dp, dl) }
    }
    #[cfg(not(target_arch = "wasm32"))]
    testing::emit(name, json)
}

/// Synchronous call of `function` on the module at `address` (`hive.call`).
///
/// The callee runs with all remaining gas; on success its storage writes and
/// events are merged into this call. `address` may be upper case or carry a
/// `hive:` prefix; it is normalized before the call.
pub fn call(address: &str, function: &str, input: &[u8]) -> Result<Vec<u8>, CallError> {
    let address = normalize_address(address);
    #[cfg(target_arch = "wasm32")]
    {
        let (ap, al) = sys::p(address.as_bytes());
        let (fp, fl) = sys::p(function.as_bytes());
        let (ip, il) = sys::p(input);
        // SAFETY: valid (ptr, len) pairs into our memory.
        let r = unsafe { sys::hive_call(ap, al, fp, fl, ip, il) };
        if r < 0 {
            return Err(CallError::from_code(r));
        }
        Ok(sys::take(r))
    }
    #[cfg(not(target_arch = "wasm32"))]
    testing::call(&address, function, input)
}

/// [`call`] with JSON in and out.
pub fn call_json(address: &str, function: &str, input: &Value) -> Result<Value, CallError> {
    let bytes = serde_json::to_vec(input).map_err(|e| CallError::BadOutput(e.to_string()))?;
    let out = call(address, function, &bytes)?;
    if out.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&out).map_err(|e| CallError::BadOutput(e.to_string()))
}

/// This module's persistent key/value state (`storage.*`). Keys are private to
/// the module (namespaced by its address). Writes become durable only if the
/// top-level call succeeds. Limits: key 1..=256 bytes, value ≤ 64 KiB.
pub mod storage {
    use super::*;

    /// Value for `key`, or `None` if absent.
    pub fn get(key: impl AsRef<[u8]>) -> Option<Vec<u8>> {
        let key = key.as_ref();
        #[cfg(target_arch = "wasm32")]
        {
            let (kp, kl) = sys::p(key);
            // SAFETY: valid (ptr, len) into our memory.
            let r = unsafe { sys::storage_get(kp, kl) };
            if r == 0 {
                None
            } else {
                Some(sys::take(r))
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        testing::storage_get(key)
    }

    /// Value for `key` as UTF-8 text (`None` if absent or not UTF-8).
    pub fn get_string(key: impl AsRef<[u8]>) -> Option<String> {
        get(key).and_then(|v| String::from_utf8(v).ok())
    }

    /// Value for `key` decoded as JSON (`None` if absent or not decodable).
    pub fn get_json<T: DeserializeOwned>(key: impl AsRef<[u8]>) -> Option<T> {
        get(key).and_then(|v| serde_json::from_slice(&v).ok())
    }

    /// Set `key` to `value`. An empty value deletes the key; an empty key traps.
    pub fn set(key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        let (key, value) = (key.as_ref(), value.as_ref());
        #[cfg(target_arch = "wasm32")]
        {
            let (kp, kl) = sys::p(key);
            let (vp, vl) = sys::p(value);
            // SAFETY: valid (ptr, len) pairs into our memory.
            unsafe { sys::storage_set(kp, kl, vp, vl) }
        }
        #[cfg(not(target_arch = "wasm32"))]
        testing::storage_set(key, value)
    }

    /// Set `key` to the JSON encoding of `value`.
    pub fn set_json<T: Serialize + ?Sized>(key: impl AsRef<[u8]>, value: &T) {
        match serde_json::to_vec(value) {
            Ok(v) => set(key, v),
            Err(e) => abort(&format!("storage::set_json: {e}")),
        }
    }

    /// Delete `key` (no-op if absent).
    pub fn del(key: impl AsRef<[u8]>) {
        let key = key.as_ref();
        #[cfg(target_arch = "wasm32")]
        {
            let (kp, kl) = sys::p(key);
            // SAFETY: valid (ptr, len) into our memory.
            unsafe { sys::storage_del(kp, kl) }
        }
        #[cfg(not(target_arch = "wasm32"))]
        testing::storage_del(key)
    }
}

// ── native: in-process mock host ──────────────────────────────────────────────

/// Native mock of the NDSR host, used by [`crate::Module::invoke`] in `cargo test`.
///
/// State is per thread (each `#[test]` gets its own). It mirrors NDSR
/// semantics: storage namespaced per module address, failed calls roll back
/// their writes and events, `hive.call` to modules registered with
/// [`testing::register_module`], canonical-JSON checks on event data.
#[cfg(not(target_arch = "wasm32"))]
pub mod testing {
    use crate::{CallError, Module};
    use serde_json::Value;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, HashMap};

    /// Address the top-level module runs under in [`crate::Module::invoke`].
    pub const LOCAL_ADDRESS: &str =
        "0x0000000000000000000000000000000000000000000000000000000000000000";
    const MAX_DEPTH: usize = 8;

    /// Panic payload used by [`crate::abort`] natively.
    #[derive(Debug)]
    pub struct Abort(pub String);

    /// One emitted event.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Event {
        pub name: String,
        pub data: Value,
    }

    #[derive(Default, Clone)]
    pub(crate) struct State {
        storage: BTreeMap<String, BTreeMap<Vec<u8>, Vec<u8>>>,
        events: Vec<Event>,
    }

    #[derive(Default)]
    pub(crate) struct Host {
        state: State,
        pub(crate) logs: Vec<String>,
        modules: HashMap<String, &'static Module>,
        stack: Vec<String>,
    }

    thread_local! {
        static HOST: RefCell<Host> = RefCell::new(Host::default());
    }

    pub(crate) fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R {
        HOST.with(|h| f(&mut h.borrow_mut()))
    }

    fn current() -> String {
        with(|h| {
            h.stack
                .last()
                .cloned()
                .unwrap_or_else(|| LOCAL_ADDRESS.to_string())
        })
    }

    /// Clear storage, events, logs and registered modules for this thread.
    pub fn reset() {
        with(|h| *h = Host::default());
    }

    /// Make `module` callable through `hive.call` at `address` (canonical form).
    pub fn register_module(address: &str, module: &'static Module) {
        with(|h| h.modules.insert(address.to_string(), module));
    }

    /// Events emitted by successful calls so far.
    pub fn events() -> Vec<Event> {
        with(|h| h.state.events.clone())
    }

    /// Messages passed to [`crate::log`].
    pub fn logs() -> Vec<String> {
        with(|h| h.logs.clone())
    }

    /// Value stored under `key` by the module at `address`.
    pub fn storage_of(address: &str, key: &[u8]) -> Option<Vec<u8>> {
        with(|h| {
            h.state
                .storage
                .get(address)
                .and_then(|m| m.get(key).cloned())
        })
    }

    /// Value stored under `key` by the top-level module ([`LOCAL_ADDRESS`]).
    pub fn storage_value(key: &[u8]) -> Option<Vec<u8>> {
        storage_of(LOCAL_ADDRESS, key)
    }

    fn trap(msg: String) -> ! {
        std::panic::panic_any(Abort(msg))
    }

    pub(crate) fn storage_get(key: &[u8]) -> Option<Vec<u8>> {
        storage_of(&current(), key)
    }

    pub(crate) fn storage_set(key: &[u8], value: &[u8]) {
        if key.is_empty() {
            trap("storage key must not be empty".into());
        }
        if key.len() > 256 || value.len() > 64 * 1024 {
            trap("storage key/value exceeds limit".into());
        }
        let ns = current();
        with(|h| {
            let m = h.state.storage.entry(ns).or_default();
            if value.is_empty() {
                m.remove(key);
            } else {
                m.insert(key.to_vec(), value.to_vec());
            }
        })
    }

    pub(crate) fn storage_del(key: &[u8]) {
        let ns = current();
        with(|h| {
            if let Some(m) = h.state.storage.get_mut(&ns) {
                m.remove(key);
            }
        })
    }

    fn check_canonical(v: &Value) -> Result<(), String> {
        const MAX: u64 = (1u64 << 53) - 1;
        match v {
            Value::Number(n) => match (n.as_u64(), n.as_i64()) {
                (Some(u), _) if u <= MAX => Ok(()),
                (None, Some(i)) if i.unsigned_abs() <= MAX => Ok(()),
                _ => Err(format!(
                    "event data contains {n}: floats and integers beyond 2^53-1 are not allowed"
                )),
            },
            Value::Array(a) => a.iter().try_for_each(check_canonical),
            Value::Object(m) => m.values().try_for_each(check_canonical),
            _ => Ok(()),
        }
    }

    pub(crate) fn emit(name: &str, json: &[u8]) {
        let valid_name = !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b));
        if !valid_name {
            trap(format!("invalid event name {name:?}"));
        }
        if json.len() > 16 * 1024 {
            trap("event data exceeds 16 KiB".into());
        }
        let data: Value = match serde_json::from_slice(json) {
            Ok(v) => v,
            Err(e) => trap(format!("event data is not JSON: {e}")),
        };
        if let Err(e) = check_canonical(&data) {
            trap(e);
        }
        with(|h| {
            if h.state.events.len() >= 64 {
                trap("too many events (limit 64)".into());
            }
            h.state.events.push(Event {
                name: name.to_string(),
                data,
            })
        });
    }

    /// Run `f` as one atomic frame: on failure, restore storage and events.
    fn frame(
        address: &str,
        module: &'static Module,
        name: &str,
        input: &[u8],
    ) -> Result<Vec<u8>, String> {
        let id = module.func_id(name).ok_or_else(|| {
            format!(
                "function {name:?} not found; module exports {:?}",
                module.functions()
            )
        })?;
        let func = module.export(id).expect("valid id").func;
        let snapshot = with(|h| {
            h.stack.push(address.to_string());
            h.state.clone()
        });
        let r = std::panic::catch_unwind(|| func(input));
        let r = match r {
            Ok(r) => r,
            Err(p) => Err(match p.downcast::<Abort>() {
                Ok(a) => a.0,
                Err(p) => p
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| p.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "panic".into()),
            }),
        };
        with(|h| {
            h.stack.pop();
            if r.is_err() {
                h.state = snapshot;
            }
        });
        if let Ok(out) = &r {
            if std::str::from_utf8(out).is_err() {
                return Err("output is not valid UTF-8".into());
            }
        }
        r
    }

    pub(crate) fn invoke_top(
        module: &'static Module,
        name: &str,
        input: &[u8],
    ) -> Result<Vec<u8>, String> {
        frame(LOCAL_ADDRESS, module, name, input)
    }

    pub(crate) fn call(address: &str, function: &str, input: &[u8]) -> Result<Vec<u8>, CallError> {
        let canonical = address.len() == 66
            && address.starts_with("0x")
            && address[2..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !canonical {
            return Err(CallError::BadAddress);
        }
        let (module, depth) = with(|h| (h.modules.get(address).copied(), h.stack.len()));
        let module = module.ok_or(CallError::ModuleNotFound)?;
        if module.func_id(function).is_none() {
            return Err(CallError::FunctionNotFound);
        }
        if depth > MAX_DEPTH {
            return Err(CallError::DepthExceeded);
        }
        frame(address, module, function, input).map_err(|_| CallError::CalleeFailed)
    }
}
