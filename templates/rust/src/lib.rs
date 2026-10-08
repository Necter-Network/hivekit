use hivekit::prelude::*;

/// Exported as "addNumbers" (camelCase of the identifier).
#[hive_export]
fn add_numbers(input: Value) -> Result<Value, String> {
    let a = input["a"].as_i64().ok_or("`a` must be an integer")?;
    let b = input["b"].as_i64().ok_or("`b` must be an integer")?;
    Ok(json!({ "total": a + b }))
}

/// Exported under an explicit name.
#[hive_export("increment")]
fn bump(input: Value) -> Result<Value, String> {
    let by = input["by"].as_i64().unwrap_or(1);
    let n = storage::get_json::<i64>("n").unwrap_or(0) + by;
    storage::set_json("n", &n);
    emit("incremented", &json!({ "value": n }));
    Ok(json!({ "value": n }))
}

// Every exported function, once per crate.
hive_module!(add_numbers, bump);
