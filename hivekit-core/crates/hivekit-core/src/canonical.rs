//! Canonical JSON, Keccak-256 and module-address helpers (docs/HBC_SPEC.md §4).
//!
//! Canonical JSON: UTF-8, object keys sorted by codepoint, no whitespace,
//! non-ASCII written as-is (never `\u` escaped), integers only with
//! `|n| <= 2^53 - 1`. Floats are rejected rather than formatted, because float
//! formatting differs between languages and would break hash agreement.
//! This matches Python's
//! `json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.

use anyhow::{bail, Result};
use serde_json::Value;
use tiny_keccak::{Hasher, Keccak};

/// Largest integer allowed in canonical JSON: 2^53 - 1.
pub const MAX_SAFE_INTEGER: u64 = (1u64 << 53) - 1;

const MAX_DEPTH: usize = 64;

/// Original Keccak-256 (Ethereum flavour, not FIPS SHA3-256).
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    keccak256_concat(&[data])
}

/// Keccak-256 over the concatenation of several byte slices.
pub fn keccak256_concat(parts: &[&[u8]]) -> [u8; 32] {
    let mut k = Keccak::v256();
    for p in parts {
        k.update(p);
    }
    let mut out = [0u8; 32];
    k.finalize(&mut out);
    out
}

/// `0x` + lowercase hex of keccak256(data). Same format as the `crypto.hash` host import.
pub fn keccak_hex(data: &[u8]) -> String {
    format!("0x{}", hex::encode(keccak256(data)))
}

/// Serialize `v` as canonical JSON bytes.
pub fn canonical_json(v: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(128);
    write_canonical(v, &mut out, 0)?;
    Ok(out)
}

/// Parse `text` as JSON (rejecting duplicate keys) and return its canonical form.
pub fn canonicalize_str(text: &str) -> Result<String> {
    reject_duplicate_keys(text.as_bytes())?;
    let v: Value = serde_json::from_str(text)?;
    Ok(String::from_utf8(canonical_json(&v)?).expect("canonical JSON is UTF-8"))
}

fn write_canonical(v: &Value, out: &mut Vec<u8>, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        bail!("JSON nesting deeper than {MAX_DEPTH}");
    }
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                if u > MAX_SAFE_INTEGER {
                    bail!("integer {u} exceeds 2^53-1; encode it as a string");
                }
                out.extend_from_slice(u.to_string().as_bytes());
            } else if let Some(i) = n.as_i64() {
                if i.unsigned_abs() > MAX_SAFE_INTEGER {
                    bail!("integer {i} exceeds 2^53-1; encode it as a string");
                }
                out.extend_from_slice(i.to_string().as_bytes());
            } else {
                bail!("non-integer number {n} is not allowed in canonical JSON");
            }
        }
        Value::String(s) => out.extend_from_slice(serde_json::to_string(s)?.as_bytes()),
        Value::Array(a) => {
            out.push(b'[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(x, out, depth + 1)?;
            }
            out.push(b']');
        }
        Value::Object(m) => {
            // Sort explicitly; never rely on serde_json's map ordering features.
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(serde_json::to_string(k)?.as_bytes());
                out.push(b':');
                write_canonical(&m[*k], out, depth + 1)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// Fail if any JSON object in `bytes` has a duplicate key. serde_json silently
/// keeps the last one, which would let two parties see different documents.
pub fn reject_duplicate_keys(bytes: &[u8]) -> Result<()> {
    use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
    struct Check;
    impl<'de> serde::Deserialize<'de> for Check {
        fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            d.deserialize_any(CheckVisitor)
        }
    }
    struct CheckVisitor;
    impl<'de> Visitor<'de> for CheckVisitor {
        type Value = Check;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("JSON")
        }
        fn visit_bool<E>(self, _: bool) -> std::result::Result<Check, E> {
            Ok(Check)
        }
        fn visit_i64<E>(self, _: i64) -> std::result::Result<Check, E> {
            Ok(Check)
        }
        fn visit_u64<E>(self, _: u64) -> std::result::Result<Check, E> {
            Ok(Check)
        }
        fn visit_f64<E>(self, _: f64) -> std::result::Result<Check, E> {
            Ok(Check)
        }
        fn visit_str<E>(self, _: &str) -> std::result::Result<Check, E> {
            Ok(Check)
        }
        fn visit_unit<E>(self) -> std::result::Result<Check, E> {
            Ok(Check)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<Check, A::Error> {
            while a.next_element::<Check>()?.is_some() {}
            Ok(Check)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<Check, A::Error> {
            let mut seen = std::collections::HashSet::new();
            while let Some(k) = a.next_key::<String>()? {
                if !seen.insert(k.clone()) {
                    return Err(de::Error::custom(format!("duplicate key {k:?}")));
                }
                a.next_value::<Check>()?;
            }
            Ok(Check)
        }
    }
    serde_json::from_slice::<Check>(bytes)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// True for a canonical module address: `0x` + 64 lowercase hex characters.
pub fn is_module_address(s: &str) -> bool {
    s.len() == 66
        && s.starts_with("0x")
        && s[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Accept `0x`/`0X` + 64 hex (any case), optionally prefixed by `hive:`, and
/// return the canonical lowercase form. Anything else is `None`.
pub fn normalize_module_address(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.strip_prefix("hive:").unwrap_or(s);
    if s.len() != 66 || !(s.starts_with("0x") || s.starts_with("0X")) {
        return None;
    }
    if !s[2..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", s[2..].to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keccak_is_ethereum_keccak() {
        assert_eq!(
            keccak_hex(b""),
            "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn sorts_compacts_and_keeps_non_ascii() {
        let v = json!({"b": 1, "a": {"z": [1, "é", null], "y": true}, "\u{1F600}": "x"});
        let s = String::from_utf8(canonical_json(&v).unwrap()).unwrap();
        assert_eq!(s, r#"{"a":{"y":true,"z":[1,"é",null]},"b":1,"😀":"x"}"#);
    }

    #[test]
    fn escapes_like_python() {
        let v = json!({"k": "a\"b\\c\n\t\u{0001}/"});
        let s = String::from_utf8(canonical_json(&v).unwrap()).unwrap();
        assert_eq!(s, r#"{"k":"a\"b\\c\n\t\u0001/"}"#);
    }

    #[test]
    fn rejects_floats_and_unsafe_ints() {
        assert!(canonical_json(&json!({"x": 1.5})).is_err());
        assert!(canonical_json(&json!(1u64 << 53)).is_err());
        assert!(canonical_json(&json!(-(1i64 << 53))).is_err());
        assert!(canonical_json(&json!(MAX_SAFE_INTEGER)).is_ok());
    }

    #[test]
    fn duplicate_keys_rejected() {
        assert!(reject_duplicate_keys(br#"{"a":1,"a":2}"#).is_err());
        assert!(reject_duplicate_keys(br#"{"a":{"b":1,"b":1}}"#).is_err());
        assert!(reject_duplicate_keys(br#"{"a":1,"b":[{"a":1}]}"#).is_ok());
    }

    #[test]
    fn address_normalization() {
        let a = format!("0x{}", "ab".repeat(32));
        assert!(is_module_address(&a));
        assert!(!is_module_address(&a.to_uppercase()));
        assert_eq!(
            normalize_module_address(&format!("hive:0X{}", "AB".repeat(32))),
            Some(a)
        );
        assert_eq!(normalize_module_address("0x1234"), None);
    }
}
