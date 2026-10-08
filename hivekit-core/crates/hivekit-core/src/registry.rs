//! Discover exported function names from module source code.
//!
//! The result is always sorted bytewise and de-duplicated, i.e. it is exactly the
//! manifest `functions` list, so `func_id` = index into it.
//!
//! ```rust
//! use hivekit_core::registry::FunctionRegistry;
//!
//! let source = r#"
//!     #[hive_export]
//!     fn add_numbers(input: Value) -> Value { input }
//!
//!     #[hive_export("greet")]
//!     pub fn say_hello(input: Value) -> Value { input }
//! "#;
//! assert_eq!(FunctionRegistry::collect(source, "rust"), vec!["addNumbers", "greet"]);
//! ```

use crate::manifest::is_function_name;
use regex::Regex;
use std::sync::OnceLock;

pub struct FunctionRegistry;

impl FunctionRegistry {
    /// Extract exported function names (sorted, unique, valid names only).
    /// Supports Rust, Python, JS/TS/AssemblyScript and Go.
    pub fn collect(source: &str, language: &str) -> Vec<String> {
        let names = match language {
            "rust" => collect_rust(source),
            "python" => collect_python(source),
            "go" => collect_go(source),
            _ => collect_js(source),
        };
        let mut v: Vec<String> = names.into_iter().filter(|n| is_function_name(n)).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// `snake_case` -> `camelCase`: the default export name of `#[hive_export]`.
/// Must stay identical to the conversion in `hivekit-macro`.
pub fn snake_to_camel(s: &str) -> String {
    // Leading underscores are kept verbatim; after that `_x` becomes `X`.
    let body = s.trim_start_matches('_');
    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..s.len() - body.len()]);
    let mut cap = false;
    for ch in body.chars() {
        if ch == '_' {
            cap = true;
        } else if cap {
            out.push(ch.to_ascii_uppercase());
            cap = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Remove `//` line comments and `/* */` block comments, keeping string literals intact.
fn strip_rust_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
        } else if c == b'"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i += 2;
            out.push(b' ');
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Rust: `#[hive_export]` (default name = camelCase of the fn ident) and
/// `#[hive_export("name")]`, optionally path-qualified (`#[hivekit::hive_export]`),
/// with other attributes / visibility / qualifiers before `fn`.
fn collect_rust(source: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(concat!(
            r#"#\[\s*(?:::\s*)?(?:hivekit\s*::\s*)?hive_export\s*(?:\(\s*"([^"]*)"\s*\))?\s*\]"#,
            r#"(?:\s*#\[[^\]]*\])*"#,
            r#"\s*(?:pub\s*(?:\([^)]*\))?\s*)?(?:(?:const|async|unsafe|extern\s*(?:"[^"]*")?)\s+)*"#,
            r#"fn\s+(?:r#)?([A-Za-z_][A-Za-z0-9_]*)"#,
        ))
        .unwrap()
    });
    let src = strip_rust_comments(source);
    re.captures_iter(&src)
        .filter_map(|c| match c.get(1) {
            Some(named) => Some(named.as_str().to_string()),
            None => c.get(2).map(|f| snake_to_camel(f.as_str())),
        })
        .collect()
}

/// Python: `@hive.define("name")` or `hive.define("name", fn)`.
fn collect_python(source: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"@?hive\.define\s*\(\s*["']([^"']+)["']"#).unwrap());
    re.captures_iter(source)
        .map(|c| c[1].trim().to_string())
        .collect()
}

/// JS/TS/AssemblyScript: `hive.define("name", ...)`.
fn collect_js(source: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re =
        RE.get_or_init(|| Regex::new(r#"hive\s*\.\s*define\s*\(\s*["'`]([^"'`]+)["'`]"#).unwrap());
    re.captures_iter(source)
        .map(|c| c[1].trim().to_string())
        .collect()
}

/// Go: `hive.Define("name", fn)`.
fn collect_go(source: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"hive\.Define\s*\(\s*"([^"]+)""#).unwrap());
    re.captures_iter(source)
        .map(|c| c[1].trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_both_forms() {
        let src = r#"
            use hivekit::prelude::*;

            #[hive_export]
            fn add_numbers(input: Value) -> Value { input }

            #[hive_export("Greet")]
            pub fn greet_user(input: Value) -> Value { input }

            #[hivekit::hive_export]
            #[allow(dead_code)]
            pub(crate) fn stats(input: Value) -> Value { input }

            #[ hive_export ( "zeta" ) ]
            fn z(input: Value) -> Value { input }

            // #[hive_export]
            // fn commented_out(input: Value) -> Value { input }
            /* #[hive_export("alsoCommented")] fn x() {} */

            fn not_exported() {}
            const S: &str = "// not a comment";
        "#;
        assert_eq!(
            FunctionRegistry::collect(src, "rust"),
            vec!["Greet", "addNumbers", "stats", "zeta"]
        );
    }

    #[test]
    fn snake_to_camel_matches_macro() {
        assert_eq!(snake_to_camel("add_numbers"), "addNumbers");
        assert_eq!(snake_to_camel("get"), "get");
        assert_eq!(snake_to_camel("_private_fn"), "_privateFn");
        assert_eq!(snake_to_camel("a_b_c"), "aBC");
        assert_eq!(snake_to_camel("__consensus"), "__consensus");
    }

    #[test]
    fn other_languages_sorted_unique() {
        let py = "@hive.define(\"ping\")\ndef p(i): ...\n@hive.define('greet')\ndef g(i): ...\nhive.define(\"ping\", p)";
        assert_eq!(
            FunctionRegistry::collect(py, "python"),
            vec!["greet", "ping"]
        );
        let js = "hive.define(`b`, f); hive.define('a', g); hive.define(\"bad-name\", h)";
        assert_eq!(FunctionRegistry::collect(js, "javascript"), vec!["a", "b"]);
        let go = "hive.Define(\"Echo\", echo)";
        assert_eq!(FunctionRegistry::collect(go, "go"), vec!["Echo"]);
    }
}
