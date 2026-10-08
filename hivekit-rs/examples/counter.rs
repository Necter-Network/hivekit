//! Counter module: persistent storage, events, error paths and `hive.call`.
//!
//!   hivec build --example counter
//!   hivec run dist/counter.hbc increment '{"by": 5}' --data-dir /tmp/node
//!   hivec run dist/counter.hbc get --data-dir /tmp/node        # -> {"value":5}
//!
//! Cross-module calls need the callee's .hbc available to the node:
//!   hivec run dist/counter.hbc callAdd '{"module":"0x…","a":1,"b":2}' --module dist/math_module.hbc

use hivekit::prelude::*;

const KEY: &str = "counter";

fn current() -> i64 {
    storage::get_json(KEY).unwrap_or(0)
}

/// `{"by": i64}` (default 1): add to the counter, emit `incremented`.
#[hive_export]
fn increment(input: Value) -> Result<Value, String> {
    let by = if input.is_null() || input.get("by").is_none() {
        1
    } else {
        input["by"].as_i64().ok_or("`by` must be an integer")?
    };
    let value = current().checked_add(by).ok_or("overflow")?;
    storage::set_json(KEY, &value);
    emit("incremented", &json!({ "by": by, "value": value }));
    log(&format!("counter is now {value}"));
    Ok(json!({ "value": value }))
}

/// Current value.
#[hive_export]
fn get() -> Value {
    json!({ "value": current() })
}

/// Delete the counter, emit `reset`.
#[hive_export]
fn reset() -> Value {
    storage::del(KEY);
    emit("reset", &json!({}));
    json!({ "value": 0 })
}

/// Writes, emits, then fails: nothing it did may be committed.
#[hive_export]
fn fail() -> Result<Value, String> {
    storage::set_json(KEY, &999);
    emit("never_seen", &json!({}));
    Err("deliberate failure".into())
}

fn module_arg(input: &Value) -> Result<&str, String> {
    input["module"]
        .as_str()
        .ok_or_else(|| "`module` (callee address) is required".into())
}

/// `{"module": addr, "a", "b"}`: `hive.call(addr, "addNumbers")`, store the
/// sum, emit `remote_sum`.
#[hive_export("callAdd")]
fn call_add(input: Value) -> Result<Value, String> {
    let module = module_arg(&input)?;
    let out = call_json(
        module,
        "addNumbers",
        &json!({ "a": input["a"], "b": input["b"] }),
    )
    .map_err(|e| e.to_string())?;
    let total = out["total"].as_i64().ok_or("callee returned no total")?;
    storage::set_json("last_sum", &total);
    emit("remote_sum", &json!({ "total": total }));
    Ok(json!({ "total": total, "via": module }))
}

/// `{"module": addr, "function": name, "input": any}`: call anything and
/// report the outcome instead of failing, so error codes are observable.
#[hive_export("tryCall")]
fn try_call(input: Value) -> Result<Value, String> {
    let module = module_arg(&input)?;
    let function = input["function"].as_str().ok_or("`function` is required")?;
    Ok(match call_json(module, function, &input["input"]) {
        Ok(output) => json!({ "ok": true, "output": output }),
        Err(e) => json!({ "ok": false, "code": e.code(), "error": e.to_string() }),
    })
}

hive_module!(increment, get, reset, fail, call_add, try_call);
