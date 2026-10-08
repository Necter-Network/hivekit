//! HiveKit Python runtime for `hive-wasm-v1`.
//!
//! RustPython compiled to `wasm32-unknown-unknown`: no WASI, only the
//! HBC_SPEC §6.4 host imports. The user's module source is embedded by the SDK
//! packager (see `hive-guest`). Each call builds an interpreter, installs the
//! `hivekit` and `json` modules, executes the module source (running its
//! `@hive.define` decorators) and dispatches `functions[func_id]`.
//!
//! Determinism: there is no clock, randomness, filesystem or network. Only an
//! allow-list of pure built-in modules can be imported (`time`, `os`, `random`,
//! threads, ... cannot). String hashing uses a fixed seed.

use rustpython_vm::builtins::{PyBaseExceptionRef, PyStrRef};
use rustpython_vm::{pymodule, AsObject, Interpreter, PyObjectRef, PyResult, VirtualMachine};

const HIVEKIT_SRC: &str = include_str!("hivekit_rt.py");
const JSON_SRC: &str = include_str!("json_rt.py");

/// Built-in (Rust) modules that are deterministic and may be imported.
const ALLOWED_BUILTINS: &[&str] = &[
    "_hive", "sys", "builtins", "itertools", "_functools", "_collections", "_operator", "_abc", "_string",
    "_sre", "_weakref", "errno", "_typing", "_types", "marshal", "_codecs",
];

#[pymodule]
mod _hive {
    use rustpython_vm::builtins::PyUtf8StrRef;
    use rustpython_vm::function::OptionalArg;
    use rustpython_vm::{PyObjectRef, PyResult, VirtualMachine};

    #[pyfunction]
    fn storage_get(key: PyUtf8StrRef) -> Option<String> {
        hive_guest::storage_get(key.as_str())
    }

    #[pyfunction]
    fn storage_set(key: PyUtf8StrRef, value: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
        if key.as_str().is_empty() {
            return Err(vm.new_value_error("storage key must not be empty".to_owned()));
        }
        hive_guest::storage_set(key.as_str(), value.as_str());
        Ok(())
    }

    #[pyfunction]
    fn storage_del(key: PyUtf8StrRef) {
        hive_guest::storage_del(key.as_str())
    }

    #[pyfunction]
    fn emit(name: PyUtf8StrRef, data: PyUtf8StrRef) {
        hive_guest::emit(name.as_str(), data.as_str())
    }

    /// Output string on success, negative error code (int) on failure.
    #[pyfunction]
    fn call(address: PyUtf8StrRef, function: PyUtf8StrRef, input: PyUtf8StrRef, vm: &VirtualMachine) -> PyObjectRef {
        match hive_guest::call(address.as_str(), function.as_str(), input.as_str()) {
            Ok(out) => vm.ctx.new_str(out).into(),
            Err(code) => vm.ctx.new_int(code).into(),
        }
    }

    #[pyfunction]
    fn hash(data: PyUtf8StrRef) -> String {
        hive_guest::hash(data.as_str().as_bytes())
    }

    #[pyfunction]
    fn log(msg: PyUtf8StrRef) {
        hive_guest::log(msg.as_str())
    }

    #[pyfunction]
    fn abort(msg: PyUtf8StrRef) {
        hive_guest::abort(msg.as_str())
    }

    #[pyfunction]
    fn json_loads(s: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult {
        let mut de = serde_json::Deserializer::from_str(s.as_str());
        let v = rustpython_vm::py_serde::deserialize(vm, &mut de)
            .map_err(|e| vm.new_value_error(format!("invalid JSON: {e}")))?;
        de.end().map_err(|e| vm.new_value_error(format!("invalid JSON: {e}")))?;
        Ok(v)
    }

    #[pyfunction]
    fn json_quote(s: PyUtf8StrRef, ensure_ascii: OptionalArg<bool>) -> String {
        let ascii = ensure_ascii.unwrap_or(true);
        let mut out = String::with_capacity(s.as_str().len() + 2);
        out.push('"');
        for c in s.as_str().chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 || (ascii && (c as u32) > 0x7e) => {
                    let mut buf = [0u16; 2];
                    for unit in c.encode_utf16(&mut buf) {
                        out.push_str(&format!("\\u{:04x}", unit));
                    }
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

/// `builtins.__import__` replacement: modules preloaded into `sys.modules`
/// (hivekit, json) plus an allow-list of deterministic built-in modules.
fn hive_import(
    name: PyStrRef,
    _globals: rustpython_vm::function::OptionalArg<PyObjectRef>,
    _locals: rustpython_vm::function::OptionalArg<PyObjectRef>,
    fromlist: rustpython_vm::function::OptionalArg<PyObjectRef>,
    level: rustpython_vm::function::OptionalArg<i32>,
    vm: &VirtualMachine,
) -> PyResult {
    let _ = fromlist;
    if level.unwrap_or(0) != 0 {
        return Err(vm.new_import_error("relative imports are not available in a hive-wasm-v1 module".to_owned(), name));
    }
    let n = name.to_str().unwrap_or("").to_owned();
    let n = n.as_str();
    let modules = vm.sys_module.get_attr("modules", vm)?;
    if let Ok(m) = modules.get_item(n, vm) {
        return Ok(m);
    }
    let top = n.split('.').next().unwrap_or(n);
    if ALLOWED_BUILTINS.contains(&top) {
        return rustpython_vm::import::import_builtin(vm, n);
    }
    Err(vm.new_import_error(
        format!(
            "No module named '{n}': only hivekit, json and pure built-ins are available inside a hive-wasm-v1 module \
             (clocks, randomness, files and network are not deterministic)"
        ),
        name,
    ))
}

fn format_exception(vm: &VirtualMachine, e: &PyBaseExceptionRef) -> String {
    // hive.fail(msg) -> exactly msg.
    if e.class().name().to_string() == "HiveAbort" {
        if let Some(arg) = e.args().first() {
            if let Ok(s) = arg.str(vm) {
                return s.to_string();
            }
        }
    }
    let mut s = String::new();
    if vm.write_exception(&mut s, e).is_ok() && !s.is_empty() {
        // Keep the message within the 1 KiB that hive.abort records: last lines first.
        let lines: Vec<&str> = s.trim_end().lines().collect();
        let mut out = String::new();
        for l in lines.iter().rev() {
            if out.len() + l.len() + 1 > 1000 {
                break;
            }
            out = if out.is_empty() { l.to_string() } else { format!("{l}\n{out}") };
        }
        return out;
    }
    "Python exception".to_string()
}

fn install(vm: &VirtualMachine) -> PyResult<()> {
    let import = vm.new_function("__import__", hive_import);
    vm.builtins.set_attr("__import__", import, vm)?;
    rustpython_vm::import::import_source(vm, "json", JSON_SRC)?;
    rustpython_vm::import::import_source(vm, "hivekit", HIVEKIT_SRC)?;
    Ok(())
}

/// A fresh interpreter with `_hive` registered and `json`/`hivekit` installed.
fn new_interpreter() -> Result<Interpreter, String> {
    let mut settings = rustpython_vm::Settings::default();
    settings.hash_seed = Some(0);
    settings.import_site = false;
    settings.allow_external_library = false;
    let builder = Interpreter::builder(settings);
    let def = _hive::module_def(&builder.ctx);
    let interp = builder.add_native_module(def).build();
    interp
        .enter(|vm| install(vm).map_err(|e| format!("runtime setup failed: {}", format_exception(vm, &e))))?;
    Ok(interp)
}

thread_local! {
    /// Interpreter prepared by `__hive_preinit` (captured in the build-time snapshot).
    static PREPARED: std::cell::RefCell<Option<Interpreter>> = const { std::cell::RefCell::new(None) };
}

/// Build-time pre-initialization: create the interpreter and install the
/// runtime modules. `build.sh` runs this once and snapshots linear memory into
/// the shipped module, so calls skip interpreter start-up. Calls no host functions.
#[no_mangle]
pub extern "C" fn __hive_preinit() {
    let interp = new_interpreter().expect("pre-initialization failed");
    PREPARED.with(|p| *p.borrow_mut() = Some(interp));
}

fn run(func_id: i32, input: String) -> Result<String, String> {
    let script = hive_guest::load_script()?;
    let name = usize::try_from(func_id)
        .ok()
        .and_then(|i| script.functions.get(i))
        .ok_or_else(|| format!("unknown func_id {func_id}"))?
        .clone();

    let interp = match PREPARED.with(|p| p.borrow_mut().take()) {
        Some(i) => i,
        None => new_interpreter()?,
    };
    interp.enter(|vm| -> Result<String, String> {
        let scope = vm.new_scope_with_builtins();
        scope
            .globals
            .set_item("__name__", vm.ctx.new_str("__hive_module__").into(), vm)
            .map_err(|e| format_exception(vm, &e))?;
        let code = vm
            .compile(&script.source, rustpython_vm::compiler::Mode::Exec, "module.py")
            .map_err(|e| format!("{}", e))?;
        vm.run_code_obj(code, scope).map_err(|e| format_exception(vm, &e))?;
        let out: PyResult<String> = (|| {
            let hk = vm.sys_module.get_attr("modules", vm)?.get_item("hivekit", vm)?;
            let hive = hk.get_attr("hive", vm)?;
            let r = vm.call_method(&hive, "_dispatch", (name.clone(), input.clone()))?;
            let s: rustpython_vm::builtins::PyUtf8StrRef = r.try_into_value(vm)?;
            Ok(s.as_str().to_owned())
        })();
        out.map_err(|e| format_exception(vm, &e))
    })
}

#[no_mangle]
pub extern "C" fn __hive_entry(func_id: i32, ptr: i32, len: i32) -> i64 {
    hive_guest::install_panic_hook();
    hive_guest::entry(ptr, len, |input| run(func_id, input))
}

/// Deterministic getrandom backend: there is no entropy on hive-wasm-v1.
#[no_mangle]
unsafe extern "Rust" fn __getrandom_v03_custom(dest: *mut u8, len: usize) -> Result<(), getrandom::Error> {
    for i in 0..len {
        *dest.add(i) = 0;
    }
    Ok(())
}
