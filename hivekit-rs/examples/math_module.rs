//! Math module: pure functions plus one that records its result in storage.
//!
//! Build and package (writes dist/math_module.hbc):
//!   hivec build --example math_module
//! Run on NDSR:
//!   hivec run dist/math_module.hbc addNumbers '{"a": 10, "b": 32}'

use hivekit::prelude::*;

fn int(input: &Value, key: &str) -> Result<i64, String> {
    input[key]
        .as_i64()
        .ok_or_else(|| format!("`{key}` must be an integer"))
}

/// `{"a": i64, "b": i64}` -> `{"total": a + b}`
#[hive_export]
fn add_numbers(input: Value) -> Result<Value, String> {
    let (a, b) = (int(&input, "a")?, int(&input, "b")?);
    let total = a.checked_add(b).ok_or("overflow")?;
    Ok(json!({ "total": total }))
}

/// `{"a": i64, "b": i64}` -> `{"result": a * b}`
#[hive_export]
fn multiply(input: Value) -> Result<Value, String> {
    let (a, b) = (int(&input, "a")?, int(&input, "b")?);
    Ok(json!({ "result": a.checked_mul(b).ok_or("overflow")? }))
}

/// `{"numbers": [i64]}` -> count/sum/min/max/mean. Fails on an empty list.
#[hive_export]
fn stats(input: Value) -> Result<Value, String> {
    let numbers: Vec<i64> = input["numbers"]
        .as_array()
        .ok_or("`numbers` must be an array")?
        .iter()
        .map(|v| v.as_i64().ok_or("numbers must be integers"))
        .collect::<Result<_, _>>()?;
    if numbers.is_empty() {
        return Err("no numbers provided".into());
    }
    let sum: i64 = numbers.iter().sum();
    Ok(json!({
        "count": numbers.len(),
        "sum": sum,
        "min": numbers.iter().min(),
        "max": numbers.iter().max(),
        "mean": sum as f64 / numbers.len() as f64,
    }))
}

/// Keccak-256 of the `text` field, computed by the host (`crypto.hash`).
#[hive_export("hashText")]
fn hash_text(input: Value) -> Result<Value, String> {
    let text = input["text"].as_str().ok_or("`text` must be a string")?;
    Ok(json!({ "hash": hash(text.as_bytes()) }))
}

/// Add and remember the result in this module's storage; emits `recorded`.
/// Used by the counter example to show that a callee's writes and events are
/// kept when it is reached through `hive.call`.
#[hive_export]
fn record(input: Value) -> Result<Value, String> {
    let (a, b) = (int(&input, "a")?, int(&input, "b")?);
    let total = a.checked_add(b).ok_or("overflow")?;
    storage::set_json("last_total", &total);
    emit("recorded", &json!({ "total": total }));
    Ok(json!({ "total": total }))
}

/// The last total stored by `record` (`null` if none).
#[hive_export]
fn recorded() -> Value {
    json!({ "total": storage::get_json::<i64>("last_total") })
}

hive_module!(add_numbers, multiply, stats, hash_text, record, recorded);
