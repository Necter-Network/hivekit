//! Native tests of the SDK against the in-process mock host.

use hivekit::testing;
use serde_json::json;

mod math {
    use hivekit::prelude::*;

    #[hive_export]
    fn add_numbers(input: Value) -> Result<Value, String> {
        let a = input["a"].as_i64().ok_or("a")?;
        let b = input["b"].as_i64().ok_or("b")?;
        Ok(json!({ "total": a + b }))
    }

    #[hive_export("Greet")]
    fn greet_user(input: Value) -> String {
        format!("hello {}", input["name"].as_str().unwrap_or("stranger"))
    }

    #[hive_export]
    fn record(input: Value) -> Value {
        storage::set("last", input.to_string());
        emit("recorded", &input);
        json!({ "ok": true })
    }

    #[hive_export]
    fn boom() -> Value {
        storage::set("x", "y");
        panic!("kaboom")
    }

    #[hive_export]
    fn nothing() {}

    hive_module!(add_numbers, greet_user, record, boom, nothing);
}

mod counter {
    use hivekit::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize)]
    pub struct By {
        by: i64,
    }

    #[derive(Serialize)]
    pub struct Count {
        value: i64,
    }

    #[hive_export]
    fn increment(input: By) -> Json<Count> {
        let value = storage::get_json::<i64>("n").unwrap_or(0) + input.by;
        storage::set_json("n", &value);
        emit("incremented", &json!({ "value": value }));
        log("incremented");
        Json(Count { value })
    }

    #[hive_export]
    fn fail() -> Result<Value, String> {
        storage::set_json("n", &999);
        emit("never", &json!({}));
        Err("deliberate".into())
    }

    #[hive_export("emitFloat")]
    fn emit_float() -> Value {
        emit("bad", &json!({ "x": 1.5 }));
        json!({})
    }

    #[hive_export("callMath")]
    fn call_math(input: Value) -> Result<Value, String> {
        let addr = input["module"].as_str().ok_or("module")?;
        let f = input["function"].as_str().ok_or("function")?;
        Ok(match call_json(addr, f, &input["input"]) {
            Ok(v) => json!({ "ok": v }),
            Err(e) => json!({ "code": e.code() }),
        })
    }

    #[hive_export]
    fn digest(input: Value) -> Value {
        json!({ "hash": hash(input["text"].as_str().unwrap_or("").as_bytes()) })
    }

    hive_module!(increment, fail, emit_float, call_math, digest);
}

const MATH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

#[test]
fn table_is_sorted_bytewise() {
    // Uppercase sorts first; func_id = index.
    assert_eq!(
        math::HIVE_MODULE.functions(),
        ["Greet", "addNumbers", "boom", "nothing", "record"]
    );
    assert_eq!(math::HIVE_MODULE.func_id("addNumbers"), Some(1));
    assert_eq!(
        counter::HIVE_MODULE.functions(),
        ["callMath", "digest", "emitFloat", "fail", "increment"]
    );
}

#[test]
fn invoke_json_and_text_outputs() {
    testing::reset();
    let m = &math::HIVE_MODULE;
    assert_eq!(
        m.invoke("addNumbers", json!({"a": 40, "b": 2})).unwrap(),
        json!({"total": 42})
    );
    assert_eq!(
        m.invoke_raw("Greet", br#"{"name":"Ada"}"#).unwrap(),
        b"hello Ada"
    );
    assert_eq!(m.invoke_raw("Greet", b"").unwrap(), b"hello stranger");
    assert_eq!(m.invoke_raw("nothing", b"").unwrap(), b"");
    assert!(m.invoke("addNumbers", json!({})).is_err());
    assert!(m
        .invoke_raw("addNumbers", b"{")
        .unwrap_err()
        .contains("invalid input"));
    assert!(m.invoke("missing", json!({})).is_err());
}

#[test]
fn storage_events_logs_persist_on_success() {
    testing::reset();
    let c = &counter::HIVE_MODULE;
    assert_eq!(c.invoke("increment", json!({"by": 2})).unwrap()["value"], 2);
    assert_eq!(c.invoke("increment", json!({"by": 3})).unwrap()["value"], 5);
    assert_eq!(testing::storage_value(b"n").unwrap(), b"5");
    let ev = testing::events();
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[1].name, "incremented");
    assert_eq!(ev[1].data, json!({"value": 5}));
    assert_eq!(testing::logs(), ["incremented", "incremented"]);
}

#[test]
fn failures_roll_back() {
    testing::reset();
    let c = &counter::HIVE_MODULE;
    c.invoke("increment", json!({"by": 1})).unwrap();
    assert_eq!(c.invoke("fail", json!(null)).unwrap_err(), "deliberate");
    assert!(c
        .invoke("emitFloat", json!(null))
        .unwrap_err()
        .contains("floats"));
    assert!(math::HIVE_MODULE
        .invoke("boom", json!(null))
        .unwrap_err()
        .contains("kaboom"));
    assert_eq!(testing::storage_value(b"n").unwrap(), b"1");
    assert_eq!(testing::storage_value(b"x"), None);
    assert_eq!(testing::events().len(), 1);
}

#[test]
fn hive_call_between_modules() {
    testing::reset();
    testing::register_module(MATH, &math::HIVE_MODULE);
    let c = &counter::HIVE_MODULE;
    let call = |f: &str, input: serde_json::Value, module: &str| {
        c.invoke(
            "callMath",
            json!({"module": module, "function": f, "input": input}),
        )
        .unwrap()
    };
    assert_eq!(
        call("addNumbers", json!({"a": 1, "b": 2}), MATH),
        json!({"ok": {"total": 3}})
    );
    // Upper case / hive: prefix are normalized.
    assert_eq!(
        call(
            "addNumbers",
            json!({"a": 1, "b": 1}),
            &format!("hive:{}", MATH.replace("0x", "0X"))
        ),
        json!({"ok": {"total": 2}})
    );
    assert_eq!(call("addNumbers", json!({}), MATH), json!({"code": -3}));
    assert_eq!(call("missing", json!({}), MATH), json!({"code": -2}));
    assert_eq!(
        call("addNumbers", json!({}), &format!("0x{}", "2".repeat(64))),
        json!({"code": -1})
    );
    assert_eq!(call("addNumbers", json!({}), "0x12"), json!({"code": -6}));
    // A callee's writes land in its own namespace and its events are merged.
    call("record", json!({"k": 1}), MATH);
    assert_eq!(testing::storage_of(MATH, b"last").unwrap(), br#"{"k":1}"#);
    assert_eq!(testing::storage_value(b"last"), None);
    assert_eq!(testing::events().last().unwrap().name, "recorded");
    // A failing callee's writes are discarded.
    call("boom", json!(null), MATH);
    assert_eq!(testing::storage_of(MATH, b"x"), None);
}

#[test]
fn hash_is_keccak256() {
    let out = counter::HIVE_MODULE
        .invoke("digest", json!({"text": "abc"}))
        .unwrap();
    assert_eq!(
        out["hash"],
        "0x4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"
    );
}
