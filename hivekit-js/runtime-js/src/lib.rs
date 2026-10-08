//! HiveKit JavaScript runtime for `hive-wasm-v1`.
//!
//! A Boa JavaScript engine compiled to `wasm32-unknown-unknown` with no WASI and
//! no imports other than the HBC_SPEC §6.4 host functions. The user's script is
//! embedded by the SDK packager (see `hive-guest`). Each call evaluates the
//! prelude and the script in a fresh context, then dispatches to the handler
//! registered under `functions[func_id]`.
//!
//! Determinism: the engine has no access to clocks, randomness, the filesystem
//! or the network. `Date.now()`, argument-less `new Date()` and `Math.random()`
//! throw; Boa's internal clock is a fixed clock at the Unix epoch.

use boa_engine::context::time::FixedClock;
use boa_engine::context::ContextBuilder;
use boa_engine::{js_string, Context, JsArgs, JsError, JsNativeError, JsResult, JsValue, NativeFunction, Source};
use std::cell::RefCell;
use std::rc::Rc;

const PRELUDE: &str = include_str!("prelude.js");

fn arg_string(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<String> {
    let v = args.get_or_undefined(i);
    Ok(v.to_string(ctx)?.to_std_string_escaped())
}

fn n_get(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let key = arg_string(args, 0, ctx)?;
    Ok(match hive_guest::storage_get(&key) {
        Some(v) => js_string!(v).into(),
        None => JsValue::null(),
    })
}

fn n_set(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let key = arg_string(args, 0, ctx)?;
    let val = arg_string(args, 1, ctx)?;
    if key.is_empty() {
        return Err(JsNativeError::typ().with_message("storage key must not be empty").into());
    }
    hive_guest::storage_set(&key, &val);
    Ok(JsValue::undefined())
}

fn n_del(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let key = arg_string(args, 0, ctx)?;
    hive_guest::storage_del(&key);
    Ok(JsValue::undefined())
}

fn n_emit(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = arg_string(args, 0, ctx)?;
    let data = arg_string(args, 1, ctx)?;
    hive_guest::emit(&name, &data);
    Ok(JsValue::undefined())
}

fn n_call(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let addr = arg_string(args, 0, ctx)?;
    let func = arg_string(args, 1, ctx)?;
    let input = arg_string(args, 2, ctx)?;
    Ok(match hive_guest::call(&addr, &func, &input) {
        Ok(out) => js_string!(out).into(),
        Err(code) => JsValue::from(code as f64),
    })
}

fn n_hash(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let data = arg_string(args, 0, ctx)?;
    Ok(js_string!(hive_guest::hash(data.as_bytes())).into())
}

fn n_log(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let msg = arg_string(args, 0, ctx)?;
    hive_guest::log(&msg);
    Ok(JsValue::undefined())
}

fn n_abort(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let msg = arg_string(args, 0, ctx)?;
    hive_guest::abort(&msg)
}

fn describe(e: JsError, ctx: &mut Context) -> String {
    match e.try_native(ctx) {
        Ok(n) => n.to_string(),
        Err(_) => e.to_string(),
    }
}

/// A fresh context with the host natives registered and the prelude evaluated.
fn new_context() -> Result<Context, String> {
    let mut ctx = ContextBuilder::default()
        .clock(Rc::new(FixedClock::from_millis(0)))
        .build()
        .map_err(|e| format!("cannot create JS context: {e}"))?;
    let natives: [(&str, usize, NativeFunction); 8] = [
        ("__hk_get", 1, NativeFunction::from_fn_ptr(n_get)),
        ("__hk_set", 2, NativeFunction::from_fn_ptr(n_set)),
        ("__hk_del", 1, NativeFunction::from_fn_ptr(n_del)),
        ("__hk_emit", 2, NativeFunction::from_fn_ptr(n_emit)),
        ("__hk_call", 3, NativeFunction::from_fn_ptr(n_call)),
        ("__hk_hash", 1, NativeFunction::from_fn_ptr(n_hash)),
        ("__hk_log", 1, NativeFunction::from_fn_ptr(n_log)),
        ("__hk_abort", 1, NativeFunction::from_fn_ptr(n_abort)),
    ];
    for (n, len, f) in natives {
        ctx.register_global_callable(js_string!(n), len, f).map_err(|e| e.to_string())?;
    }
    if let Err(e) = ctx.eval(Source::from_bytes(PRELUDE)) {
        return Err(format!("prelude failed: {}", describe(e, &mut ctx)));
    }
    Ok(ctx)
}

thread_local! {
    /// Context prepared by `__hive_preinit` (captured in the build-time snapshot).
    static PREPARED: RefCell<Option<Context>> = const { RefCell::new(None) };
}

/// Build-time pre-initialization: create the context and evaluate the prelude.
/// `runtime-js/build.sh` runs this once and snapshots linear memory into the
/// shipped module, so calls skip engine start-up. Calls no host functions.
#[no_mangle]
pub extern "C" fn __hive_preinit() {
    let ctx = new_context().expect("pre-initialization failed");
    PREPARED.with(|p| *p.borrow_mut() = Some(ctx));
}

fn run(func_id: i32, input: String) -> Result<String, String> {
    let script = hive_guest::load_script()?;
    let name = usize::try_from(func_id)
        .ok()
        .and_then(|i| script.functions.get(i))
        .ok_or_else(|| format!("unknown func_id {func_id}"))?
        .clone();

    let mut ctx = match PREPARED.with(|p| p.borrow_mut().take()) {
        Some(ctx) => ctx,
        None => new_context()?,
    };
    if let Err(e) = ctx.eval(Source::from_bytes(script.source.as_bytes())) {
        return Err(format!("module script failed: {}", describe(e, &mut ctx)));
    }
    let global = ctx.global_object();
    let dispatch = global.get(js_string!("__hive_dispatch"), &mut ctx).map_err(|e| e.to_string())?;
    let finish = global.get(js_string!("__hive_finish"), &mut ctx).map_err(|e| e.to_string())?;
    let (Some(dispatch), Some(finish)) = (dispatch.as_callable(), finish.as_callable()) else {
        return Err("runtime prelude is broken".into());
    };
    let args = [JsValue::from(js_string!(name.as_str())), JsValue::from(js_string!(input))];
    if let Err(e) = dispatch.call(&JsValue::undefined(), &args, &mut ctx) {
        return Err(describe(e, &mut ctx));
    }
    if let Err(e) = ctx.run_jobs() {
        return Err(describe(e, &mut ctx));
    }
    let out = finish.call(&JsValue::undefined(), &[], &mut ctx).map_err(|e| describe(e, &mut ctx))?;
    Ok(out.to_string(&mut ctx).map_err(|e| e.to_string())?.to_std_string_escaped())
}

#[no_mangle]
pub extern "C" fn __hive_entry(func_id: i32, ptr: i32, len: i32) -> i64 {
    hive_guest::install_panic_hook();
    hive_guest::entry(ptr, len, |input| run(func_id, input))
}

/// Deterministic getrandom backend: there is no entropy on hive-wasm-v1.
/// Boa only reaches this through `Math.random`, which the prelude disables.
#[no_mangle]
unsafe extern "Rust" fn __getrandom_v03_custom(dest: *mut u8, len: usize) -> Result<(), getrandom::Error> {
    for i in 0..len {
        *dest.add(i) = 0;
    }
    Ok(())
}
