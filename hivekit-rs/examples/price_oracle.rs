//! Price oracle: reporters submit prices, the module keeps the latest per pair.
//!
//! Prices are integers in micro-units (1.25 USD = 1250000) because event data
//! must be canonical JSON, which has no floats.
//!
//!   hivec build --example price_oracle
//!   hivec run dist/price_oracle.hbc submit '{"pair":"ETH/USD","price":3200000000}' --data-dir /tmp/node
//!   hivec run dist/price_oracle.hbc price '{"pair":"ETH/USD"}' --data-dir /tmp/node

use hivekit::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Submit {
    pair: String,
    price: u64,
}

#[derive(Serialize, Deserialize)]
struct Quote {
    price: u64,
    updates: u64,
}

fn key(pair: &str) -> String {
    format!("price:{pair}")
}

/// Record a price for a pair; emits `price_updated`.
#[hive_export]
fn submit(input: Submit) -> Result<Json<Quote>, String> {
    if input.pair.is_empty() || input.pair.len() > 32 {
        return Err("pair must be 1..=32 bytes".into());
    }
    if input.price == 0 || input.price > (1u64 << 53) - 1 {
        return Err("price must be in 1..=2^53-1".into());
    }
    let prev: Option<Quote> = storage::get_json(key(&input.pair));
    let q = Quote {
        price: input.price,
        updates: prev.map_or(0, |p| p.updates) + 1,
    };
    storage::set_json(key(&input.pair), &q);
    emit(
        "price_updated",
        &json!({ "pair": input.pair, "price": q.price, "updates": q.updates }),
    );
    Ok(Json(q))
}

/// Latest price for `{"pair": ...}`.
#[hive_export]
fn price(input: Value) -> Result<Value, String> {
    let pair = input["pair"].as_str().ok_or("`pair` is required")?;
    let q: Quote = storage::get_json(key(pair)).ok_or_else(|| format!("no price for {pair}"))?;
    Ok(json!({ "pair": pair, "price": q.price, "updates": q.updates }))
}

hive_module!(submit, price);
