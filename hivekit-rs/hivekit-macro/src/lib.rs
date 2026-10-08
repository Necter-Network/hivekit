//! Procedural macros for the HiveKit Rust SDK. Use them through the `hivekit`
//! crate (`use hivekit::prelude::*;`), not directly.
//!
//! * `#[hive_export]` / `#[hive_export("name")]` turns a function into an
//!   exported module function. The default export name is the camelCase form
//!   of the Rust identifier (`add_numbers` -> `addNumbers`).
//! * `hive_module!(f, g, ...)` builds the module: a dispatch table sorted
//!   bytewise by export name (so `func_id` = index into the manifest's sorted
//!   `functions` list), the `hivekit.functions` custom section that `hivec`
//!   reads to write the manifest, and on wasm32 the `__alloc` / `__hive_entry`
//!   exports of the `hive-wasm-v1` ABI.
//!
//! ```ignore
//! use hivekit::prelude::*;
//!
//! #[hive_export]
//! fn add_numbers(input: Value) -> Value {
//!     json!({ "total": input["a"].as_i64().unwrap_or(0) + input["b"].as_i64().unwrap_or(0) })
//! }
//!
//! #[hive_export("greet")]
//! fn say_hello(input: Value) -> Result<String, String> {
//!     Ok(format!("hello {}", input["name"].as_str().ok_or("name is required")?))
//! }
//!
//! hive_module!(add_numbers, say_hello);
//! ```

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    FnArg, ItemFn, LitStr, Path, Token,
};

/// `snake_case` -> `camelCase`. Leading underscores are kept verbatim.
/// Must stay identical to `hivekit_core::registry::snake_to_camel`.
fn snake_to_camel(s: &str) -> String {
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

/// `[A-Za-z_][A-Za-z0-9_]{0,63}`
fn is_function_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

fn export_const_ident(fn_ident: &syn::Ident) -> syn::Ident {
    let raw = fn_ident.to_string();
    let raw = raw.strip_prefix("r#").unwrap_or(&raw);
    format_ident!("__hive_export_{}", raw)
}

/// Export a function from a HiveKit module.
///
/// Supported signatures (`T: serde::de::DeserializeOwned`, `R: hivekit::HiveOutput`):
/// `fn(T) -> R` and `fn() -> R`. The input bytes are parsed as JSON into `T`
/// (empty input is `null`). `R` is typically `serde_json::Value`, `String`
/// (returned as raw text), `hivekit::Json<impl Serialize>`, `()` or a
/// `Result<_, impl Display>` of those, where `Err` aborts the call.
///
/// `#[hive_export]` exports under the camelCase form of the identifier,
/// `#[hive_export("name")]` under an explicit name.
#[proc_macro_attribute]
pub fn hive_export(args: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    let fn_ident = &func.sig.ident;

    let (export_name, name_span) = if args.is_empty() {
        let raw = fn_ident.to_string();
        let raw = raw.strip_prefix("r#").unwrap_or(&raw).to_string();
        (snake_to_camel(&raw), fn_ident.span())
    } else {
        match syn::parse::<LitStr>(args) {
            Ok(s) => (s.value(), s.span()),
            Err(_) => {
                return syn::Error::new(
                    Span::call_site(),
                    "expected #[hive_export] or #[hive_export(\"exportName\")]",
                )
                .to_compile_error()
                .into()
            }
        }
    };
    if !is_function_name(&export_name) {
        return syn::Error::new(
            name_span,
            format!(
                "invalid export name {export_name:?}: must match [A-Za-z_][A-Za-z0-9_]{{0,63}}"
            ),
        )
        .to_compile_error()
        .into();
    }
    if func.sig.asyncness.is_some() || !func.sig.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &func.sig,
            "#[hive_export] functions must be plain, non-async, non-generic fns",
        )
        .to_compile_error()
        .into();
    }

    let call = match func.sig.inputs.len() {
        0 => quote! { #fn_ident() },
        1 => match &func.sig.inputs[0] {
            FnArg::Typed(pt) => {
                let ty = &pt.ty;
                quote! { #fn_ident(::hivekit::__rt::decode_input::<#ty>(__input)?) }
            }
            FnArg::Receiver(r) => {
                return syn::Error::new_spanned(r, "#[hive_export] cannot be used on methods")
                    .to_compile_error()
                    .into()
            }
        },
        _ => {
            return syn::Error::new_spanned(
                &func.sig.inputs,
                "#[hive_export] functions take zero or one argument (the decoded JSON input)",
            )
            .to_compile_error()
            .into()
        }
    };

    let const_ident = export_const_ident(fn_ident);
    let expanded = quote! {
        #func

        #[doc(hidden)]
        #[allow(non_upper_case_globals, dead_code)]
        pub(crate) const #const_ident: ::hivekit::Export = ::hivekit::Export {
            name: #export_name,
            func: {
                fn __hive_wrapper(__input: &[u8]) -> ::core::result::Result<::std::vec::Vec<u8>, ::std::string::String> {
                    ::hivekit::HiveOutput::into_output(#call)
                }
                __hive_wrapper
            },
        };
    };
    expanded.into()
}

struct FnList(Vec<Path>);

impl Parse for FnList {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let names = Punctuated::<Path, Token![,]>::parse_terminated(input)?;
        Ok(FnList(names.into_iter().collect()))
    }
}

/// Build the module from its `#[hive_export]` functions. Call exactly once per crate.
///
/// Generates `pub static HIVE_MODULE: hivekit::Module` (dispatch table sorted
/// by export name; use it in native tests: `HIVE_MODULE.invoke("addNumbers", json!(..))`)
/// and, when compiling for wasm32, the `hivekit.functions` custom section and
/// the `__alloc` / `__hive_entry` exports.
#[proc_macro]
pub fn hive_module(input: TokenStream) -> TokenStream {
    let FnList(paths) = parse_macro_input!(input as FnList);
    if paths.is_empty() {
        return syn::Error::new(
            Span::call_site(),
            "hive_module!() needs at least one #[hive_export] function",
        )
        .to_compile_error()
        .into();
    }
    let consts: Vec<TokenStream2> = paths
        .iter()
        .map(|p| {
            let mut p = p.clone();
            let last = p.segments.last_mut().expect("non-empty path");
            last.ident = export_const_ident(&last.ident);
            quote! { #p }
        })
        .collect();
    let n = consts.len();

    let expanded = quote! {
        #[doc(hidden)]
        const __HIVE_EXPORTS_SORTED: [::hivekit::Export; #n] =
            ::hivekit::__rt::sort_exports([#(#consts),*]);
        #[doc(hidden)]
        static __HIVE_EXPORTS: [::hivekit::Export; #n] = __HIVE_EXPORTS_SORTED;

        /// This crate's HiveKit module: the exported functions sorted by name
        /// (index = `func_id`).
        pub static HIVE_MODULE: ::hivekit::Module = ::hivekit::Module::new(&__HIVE_EXPORTS);

        #[cfg(target_arch = "wasm32")]
        const _: () = {
            const __HIVE_SECTION_LEN: usize = ::hivekit::__rt::names_len(&__HIVE_EXPORTS_SORTED);

            /// Sorted export names, one per line; read by `hivec build`.
            #[unsafe(link_section = "hivekit.functions")]
            #[used]
            static __HIVE_FUNCTIONS_SECTION: [u8; __HIVE_SECTION_LEN] =
                ::hivekit::__rt::names_bytes::<__HIVE_SECTION_LEN>(&__HIVE_EXPORTS_SORTED);

            /// hive-wasm-v1: allocate `len` bytes of linear memory.
            #[unsafe(no_mangle)]
            pub extern "C" fn __alloc(len: i32) -> i32 {
                ::hivekit::__rt::alloc(len)
            }

            /// hive-wasm-v1: run function `func_id` on `memory[ptr..ptr+len]`,
            /// return the output packed as `(ptr << 32) | len`.
            #[unsafe(no_mangle)]
            pub extern "C" fn __hive_entry(func_id: i32, ptr: i32, len: i32) -> i64 {
                ::hivekit::__rt::entry(&HIVE_MODULE, func_id, ptr, len)
            }
        };
    };
    expanded.into()
}
