//! End-to-end: build the example modules with `hivec build` (cargo, wasm32-unknown-unknown),
//! then verify and execute them on the real NDSR runtime (`ndsr inspect` / `ndsr run`).
//!
//! The ndsr binary is taken from `$NDSR_BIN` or a `tools/ndsr` in a parent
//! directory of this crate. If it is absent the tests print a notice and pass;
//! every other failure (including a missing wasm32 target) fails the test.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn ndsr() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NDSR_BIN") {
        return Some(PathBuf::from(p));
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|d| d.join("tools").join("ndsr"))
        .find(|p| p.is_file())
}

macro_rules! require_ndsr {
    () => {
        match ndsr() {
            Some(p) => p,
            None => {
                eprintln!("SKIPPED: ndsr binary not found (set NDSR_BIN or provide tools/ndsr)");
                return;
            }
        }
    };
}

struct Built {
    hbc: PathBuf,
    address: String,
    functions: Vec<String>,
}

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Build all examples once (hivec build -> cargo -> .hbc).
fn examples() -> &'static [(String, Built)] {
    static B: OnceLock<Vec<(String, Built)>> = OnceLock::new();
    B.get_or_init(|| {
        let out = scratch("hbc");
        let target_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("wasm-target");
        ["math_module", "counter", "price_oracle"]
            .iter()
            .map(|ex| {
                let o = Command::new(env!("CARGO_BIN_EXE_hivec"))
                    .args([
                        "build",
                        env!("CARGO_MANIFEST_DIR"),
                        "--example",
                        ex,
                        "--out",
                    ])
                    .arg(&out)
                    .arg("--target-dir")
                    .arg(&target_dir)
                    .output()
                    .unwrap();
                assert!(
                    o.status.success(),
                    "hivec build --example {ex} failed:\n{}",
                    String::from_utf8_lossy(&o.stderr)
                );
                let v: Value = serde_json::from_slice(&o.stdout).unwrap();
                let built = Built {
                    hbc: PathBuf::from(v["hbc"].as_str().unwrap()),
                    address: v["manifest_address"].as_str().unwrap().to_string(),
                    functions: serde_json::from_value(v["functions"].clone()).unwrap(),
                };
                (ex.to_string(), built)
            })
            .collect()
    })
}

fn get(name: &str) -> &'static Built {
    &examples().iter().find(|(n, _)| n == name).unwrap().1
}

struct Run {
    code: i32,
    v: Value,
}

impl Run {
    fn success(&self) -> bool {
        self.v["success"] == true
    }
    fn output(&self) -> Value {
        serde_json::from_str(self.v["output"].as_str().unwrap()).unwrap_or(Value::Null)
    }
    fn events(&self) -> Vec<Value> {
        self.v["receipt"]["receipt"]["events"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn event_names(&self) -> Vec<String> {
        self.events()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect()
    }
    fn error(&self) -> String {
        self.v["error"].as_str().unwrap_or("").to_string()
    }
}

fn parse(o: std::process::Output) -> Run {
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| {
        panic!(
            "unparseable ndsr output ({e}):\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    });
    Run {
        code: o.status.code().unwrap_or(-1),
        v,
    }
}

/// `ndsr run` directly.
fn ndsr_run(ndsr: &Path, hbc: &Path, f: &str, input: &str, data: Option<&Path>) -> Run {
    let mut c = Command::new(ndsr);
    c.arg("run").arg(hbc).arg(f).arg("--input").arg(input);
    if let Some(d) = data {
        c.arg("--data-dir").arg(d);
    }
    parse(c.output().unwrap())
}

/// `hivec run` (which delegates to ndsr), optionally making extra modules callable.
fn hivec_run(ndsr: &Path, hbc: &Path, f: &str, input: &str, data: &Path, modules: &[&Path]) -> Run {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hivec"));
    c.arg("run")
        .arg(hbc)
        .arg(f)
        .arg(input)
        .arg("--data-dir")
        .arg(data)
        .arg("--ndsr")
        .arg(ndsr);
    for m in modules {
        c.arg("--module").arg(m);
    }
    parse(c.output().unwrap())
}

#[test]
fn artifacts_pass_ndsr_inspect() {
    let ndsr = require_ndsr!();
    for (name, b) in examples() {
        let o = Command::new(&ndsr)
            .arg("inspect")
            .arg(&b.hbc)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "ndsr inspect {name}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["abi_valid"], true, "{name}: {}", v["abi_error"]);
        assert_eq!(v["manifest_address"], b.address.as_str(), "{name}");
        assert_eq!(v["manifest"]["runtime"], "hive-wasm-v1");
        assert_eq!(v["manifest"]["language"], "rust");
        assert_eq!(v["manifest"]["functions"], json!(b.functions));
        // NDSR's func_id assignment equals the guest's sorted dispatch order.
        for (i, f) in b.functions.iter().enumerate() {
            assert_eq!(v["functions"][i]["func_id"], i);
            assert_eq!(v["functions"][i]["name"], f.as_str());
        }

        // Our own loader agrees, and the wasm has only hive-wasm-v1 imports (no WASI).
        let art = hivekit_core::load_hbc_strict(&std::fs::read(&b.hbc).unwrap()).unwrap();
        assert_eq!(art.manifest_address, b.address);
        let allowed = hivekit_core::abi::allowed_imports();
        for imp in &art.wasm_info.imports {
            assert!(!imp.starts_with("wasi"), "{name} imports {imp}");
            assert!(allowed.contains(imp), "{name} imports {imp}");
        }
        let mut declared = art.wasm_info.declared_functions.clone().unwrap();
        declared.sort();
        assert_eq!(declared, b.functions);
    }
    assert_eq!(
        get("counter").functions,
        ["callAdd", "fail", "get", "increment", "reset", "tryCall"]
    );
}

#[test]
fn pure_functions_and_error_paths() {
    let ndsr = require_ndsr!();
    let math = &get("math_module").hbc;
    let r = ndsr_run(&ndsr, math, "addNumbers", r#"{"a":40,"b":2}"#, None);
    assert!(r.success() && r.code == 0, "{}", r.error());
    assert_eq!(r.output(), json!({"total": 42}));

    let r = ndsr_run(&ndsr, math, "stats", r#"{"numbers":[1,2,3,4]}"#, None);
    assert_eq!(
        r.output(),
        json!({"count": 4, "sum": 10, "min": 1, "max": 4, "mean": 2.5})
    );

    // crypto.hash = Keccak-256.
    let r = ndsr_run(&ndsr, math, "hashText", r#"{"text":"abc"}"#, None);
    assert_eq!(
        r.output()["hash"],
        "0x4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"
    );

    // Deterministic gas.
    let again = ndsr_run(&ndsr, math, "hashText", r#"{"text":"abc"}"#, None);
    assert_eq!(r.v["gas_used"], again.v["gas_used"]);

    // Guest errors become failed receipts with the message, exit code 2.
    let r = ndsr_run(&ndsr, math, "stats", r#"{"numbers":[]}"#, None);
    assert!(!r.success());
    assert_eq!(r.code, 2);
    assert!(r.error().contains("no numbers provided"), "{}", r.error());
    let r = ndsr_run(&ndsr, math, "addNumbers", "not json", None);
    assert!(
        !r.success() && r.error().contains("invalid input"),
        "{}",
        r.error()
    );

    // Empty input is passed as (0, 0) and decodes as null.
    let r = ndsr_run(&ndsr, math, "recorded", "", None);
    assert!(r.success(), "{}", r.error());
    assert_eq!(r.output(), json!({"total": null}));
}

#[test]
fn storage_persists_and_failures_roll_back() {
    let ndsr = require_ndsr!();
    let counter = &get("counter").hbc;
    let data = scratch("node-storage");

    let r = ndsr_run(&ndsr, counter, "increment", r#"{"by":2}"#, Some(&data));
    assert!(r.success(), "{}", r.error());
    assert_eq!(r.output(), json!({"value": 2}));
    assert_eq!(
        r.events(),
        [json!({"name": "incremented", "data": {"by": 2, "value": 2}})]
    );

    let r = ndsr_run(&ndsr, counter, "increment", "", Some(&data));
    assert_eq!(r.output(), json!({"value": 3}));

    // A new process sees the committed state.
    let r = ndsr_run(&ndsr, counter, "get", "", Some(&data));
    assert_eq!(r.output(), json!({"value": 3}));
    assert!(r.events().is_empty());

    // fail() writes 999 and emits, then aborts: nothing is committed or recorded.
    let r = ndsr_run(&ndsr, counter, "fail", "", Some(&data));
    assert!(!r.success());
    assert_eq!(r.code, 2);
    assert!(
        r.error().contains("guest abort: deliberate failure"),
        "{}",
        r.error()
    );
    assert!(r.events().is_empty());
    assert_eq!(
        ndsr_run(&ndsr, counter, "get", "", Some(&data)).output(),
        json!({"value": 3})
    );

    let r = ndsr_run(&ndsr, counter, "reset", "", Some(&data));
    assert_eq!(r.event_names(), ["reset"]);
    assert_eq!(
        ndsr_run(&ndsr, counter, "get", "", Some(&data)).output(),
        json!({"value": 0})
    );

    // Without a data dir every run starts from empty state.
    ndsr_run(&ndsr, counter, "increment", r#"{"by":5}"#, None);
    assert_eq!(
        ndsr_run(&ndsr, counter, "get", "", None).output(),
        json!({"value": 0})
    );

    // Typed input / Json<T> output module.
    let oracle = &get("price_oracle").hbc;
    let r = ndsr_run(
        &ndsr,
        oracle,
        "submit",
        r#"{"pair":"ETH/USD","price":3200000000}"#,
        Some(&data),
    );
    assert!(r.success(), "{}", r.error());
    assert_eq!(r.output(), json!({"price": 3200000000u64, "updates": 1}));
    ndsr_run(
        &ndsr,
        oracle,
        "submit",
        r#"{"pair":"ETH/USD","price":3300000000}"#,
        Some(&data),
    );
    let r = ndsr_run(&ndsr, oracle, "price", r#"{"pair":"ETH/USD"}"#, Some(&data));
    assert_eq!(
        r.output(),
        json!({"pair": "ETH/USD", "price": 3300000000u64, "updates": 2})
    );
    let r = ndsr_run(
        &ndsr,
        oracle,
        "submit",
        r#"{"pair":"ETH/USD"}"#,
        Some(&data),
    );
    assert!(
        !r.success() && r.error().contains("invalid input"),
        "{}",
        r.error()
    );
}

#[test]
fn hive_call_between_modules() {
    let ndsr = require_ndsr!();
    let counter = &get("counter").hbc;
    let math = get("math_module");
    let data = scratch("node-call");
    let m = math.address.as_str();

    // Callee available through <data-dir>/modules (placed there by hivec run --module).
    let r = hivec_run(
        &ndsr,
        counter,
        "callAdd",
        &json!({"module": m, "a": 2, "b": 40}).to_string(),
        &data,
        &[&math.hbc],
    );
    assert!(r.success(), "{}", r.error());
    assert_eq!(r.output(), json!({"total": 42, "via": m}));
    assert_eq!(r.event_names(), ["remote_sum"]);
    assert!(data.join("modules").join(format!("{m}.hbc")).is_file());

    let try_call = |function: &str, input: Value, module: &str| {
        let r = hivec_run(
            &ndsr,
            counter,
            "tryCall",
            &json!({"module": module, "function": function, "input": input}).to_string(),
            &data,
            &[],
        );
        assert!(r.success(), "{}", r.error());
        r
    };

    // Callee writes and events are merged into the caller's call...
    let r = try_call("record", json!({"a": 1, "b": 2}), m);
    assert_eq!(r.output(), json!({"ok": true, "output": {"total": 3}}));
    assert_eq!(
        r.events(),
        [json!({"name": "recorded", "data": {"total": 3}})]
    );
    // ...and committed under the callee's own address.
    let r = ndsr_run(&ndsr, &math.hbc, "recorded", "", Some(&data));
    assert_eq!(r.output(), json!({"total": 3}));

    // Error codes (HBC_SPEC §6.5).
    assert_eq!(
        try_call("stats", json!({"numbers": []}), m).output()["code"],
        -3
    );
    assert_eq!(try_call("nope", json!({}), m).output()["code"], -2);
    let unknown = format!("0x{}", "ab".repeat(32));
    assert_eq!(
        try_call("addNumbers", json!({}), &unknown).output()["code"],
        -1
    );
    assert_eq!(
        try_call("addNumbers", json!({}), "0x1234").output()["code"],
        -6
    );
    // Upper-case address is normalized by the SDK before the call.
    let upper = format!("0x{}", m[2..].to_uppercase());
    assert_eq!(
        try_call("addNumbers", json!({"a": 1, "b": 1}), &upper).output(),
        json!({"ok": true, "output": {"total": 2}})
    );

    // callAdd surfaces the callee failure as its own failure: nothing committed.
    let r = hivec_run(
        &ndsr,
        counter,
        "callAdd",
        &json!({"module": m, "a": 1}).to_string(),
        &data,
        &[],
    );
    assert!(!r.success());
    assert!(r.error().contains("callee failed"), "{}", r.error());
}
