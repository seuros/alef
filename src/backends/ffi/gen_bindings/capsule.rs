//! C-ABI capsule-type codegen: host-native `Language` passthrough.
//!
//! When `[crates.ffi.capsule_types]` is configured, the listed Rust types are NOT
//! boxed into opaque `*mut {Type}` handles. Instead, the exported C function returns
//! the host ecosystem's native grammar pointer directly:
//!
//! ```ignore
//! #[no_mangle]
//! pub extern "C" fn TSLP_get_language(name: *const c_char) -> *const tree_sitter::ffi::TSLanguage {
//!     // ... param conversion ...
//!     let result = language_registry::get_language(&name_rs);
//!     result.into_raw() as *const tree_sitter::ffi::TSLanguage
//! }
//! ```
//!
//! This is the load-bearing layer: every C-ABI binding (Go, Java, C#, Swift, Dart,
//! Zig, Kotlin Android) links this C symbol and wraps the returned raw pointer in its
//! own host-native `Language` type.
//!
//! The corresponding opaque `_new`/`_free`/`_to_json`/`_from_json` symbols are
//! suppressed for capsule types (mirroring how pyo3/napi exclude their opaque wrappers),
//! because the returned pointer is owned by the host tree-sitter runtime, not by an
//! alef opaque box.

use crate::core::config::FfiCapsuleTypeConfig;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use std::collections::{HashMap, HashSet};

/// Returns the capsule return-type name if this function returns a configured capsule type.
///
/// Both bare `Named` and `Optional(Named)` returns are matched, so a fallible
/// `get_language` (whose return is unwrapped to `Named` by the IR) and an
/// `Option`-returning lookup both resolve here.
pub(in crate::backends::ffi::gen_bindings) fn capsule_return_name<'a>(
    func: &'a FunctionDef,
    capsule_types: &'a HashMap<String, FfiCapsuleTypeConfig>,
) -> Option<&'a str> {
    fn named_from_ref(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => named_from_ref(inner),
            _ => None,
        }
    }
    let name = named_from_ref(&func.return_type)?;
    if capsule_types.contains_key(name) {
        Some(name)
    } else {
        None
    }
}

/// The C return type (`*const {into_raw_type}`) emitted for a capsule-returning function.
pub(in crate::backends::ffi::gen_bindings) fn capsule_c_return_type(cfg: &FfiCapsuleTypeConfig) -> String {
    format!("*const {}", cfg.into_raw_type)
}

/// The owned-value conversion expression for a capsule return: `{expr}.into_raw() as *const {into_raw_type}`.
///
/// `value.into_raw()` transfers the grammar pointer to the host runtime; the cast
/// normalizes the pointee to the configured FFI type so cbindgen emits a stable
/// `const {c_return_type} *` signature.
pub(in crate::backends::ffi::gen_bindings) fn capsule_into_raw_expr(expr: &str, cfg: &FfiCapsuleTypeConfig) -> String {
    format!("{expr}.into_raw() as *const {}", cfg.into_raw_type)
}

/// The `c_return_type` names `gen_cbindgen_toml` must forward-declare: every configured capsule
/// type actually reachable from the current API surface, via either an instance method
/// (`capsule_used_as_opaque`, computed by the caller against `api.types.*.methods` -- a
/// different question from this function, see below) or a free function, checked here through
/// the SAME `capsule_return_name` decision the real function codegen in `lib_rs.rs` uses rather
/// than a second, independently-reasoned match on `TypeRef`.
///
/// `capsule_used_as_opaque` is intentionally NOT folded into this check: it answers whether the
/// capsule type's own handle-based typedef needs declaring *alongside* the raw pointee (a type
/// used by both a method and a capsule function), which is a different, narrower condition than
/// "is the raw pointee typedef needed at all" -- merging them would wrongly add the handle
/// typedef for a capsule type that is used only by a free function.
///
/// Before this existed, a `[crates.ffi.capsule_types]` entry left behind after the function
/// that used to return it was removed, renamed, or excluded still forward-declared its
/// `c_return_type` unconditionally, producing a header typedef no generated function ever used.
/// ~keep
pub(in crate::backends::ffi::gen_bindings) fn capsule_forward_declared_c_types<'a>(
    api: &'a ApiSurface,
    capsule_types: &'a HashMap<String, FfiCapsuleTypeConfig>,
    capsule_used_as_opaque: &HashSet<&str>,
) -> Vec<&'a str> {
    let used_by_function: HashSet<&str> = api
        .functions
        .iter()
        .filter_map(|f| capsule_return_name(f, capsule_types))
        .collect();
    let mut c_names: Vec<&str> = capsule_types
        .iter()
        .filter(|(name, _)| capsule_used_as_opaque.contains(name.as_str()) || used_by_function.contains(name.as_str()))
        .map(|(_, cfg)| cfg.c_return_type.as_str())
        .collect();
    c_names.sort_unstable();
    c_names.dedup();
    c_names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FunctionDef, TypeRef};

    fn capsule_map(entries: &[(&str, FfiCapsuleTypeConfig)]) -> HashMap<String, FfiCapsuleTypeConfig> {
        entries.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn default_cfg() -> FfiCapsuleTypeConfig {
        FfiCapsuleTypeConfig {
            into_raw_type: "tree_sitter::ffi::TSLanguage".to_string(),
            c_return_type: "TSLanguage".to_string(),
            package: None,
            package_version: None,
        }
    }

    fn make_fn(name: &str, ret: TypeRef) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            rust_path: format!("pack::{name}"),
            original_rust_path: String::new(),
            params: vec![],
            return_type: ret,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn capsule_return_name_detects_named_capsule() {
        let func = make_fn("get_language", TypeRef::Named("Language".to_string()));
        let caps = capsule_map(&[("Language", default_cfg())]);
        assert_eq!(capsule_return_name(&func, &caps), Some("Language"));
    }

    #[test]
    fn capsule_return_name_detects_optional_capsule() {
        let func = make_fn(
            "find_language",
            TypeRef::Optional(Box::new(TypeRef::Named("Language".to_string()))),
        );
        let caps = capsule_map(&[("Language", default_cfg())]);
        assert_eq!(capsule_return_name(&func, &caps), Some("Language"));
    }

    #[test]
    fn capsule_return_name_returns_none_for_non_capsule() {
        let func = make_fn("get_name", TypeRef::String);
        let caps = capsule_map(&[("Language", default_cfg())]);
        assert_eq!(capsule_return_name(&func, &caps), None);
    }

    #[test]
    fn capsule_c_return_type_emits_const_ptr() {
        assert_eq!(
            capsule_c_return_type(&default_cfg()),
            "*const tree_sitter::ffi::TSLanguage"
        );
    }

    #[test]
    fn capsule_into_raw_expr_casts_to_configured_type() {
        assert_eq!(
            capsule_into_raw_expr("result", &default_cfg()),
            "result.into_raw() as *const tree_sitter::ffi::TSLanguage"
        );
    }
}
