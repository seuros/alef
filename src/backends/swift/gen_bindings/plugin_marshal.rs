//! Swift plugin marshaling helpers for Box class generation.
//!
//! This module provides type-conversion and marshaling utilities for emitting FFI shim methods
//! in the `Swift{Trait}Box` classes. Each Box class bridges between:
//! - The **FFI layer**: raw `RustString`, `RustVec<UInt8>`, primitive types (FFI types)
//! - The **user-facing bridge protocol**: typed Swift structs (Codable), String, Data, [String], enums, etc.
//!
//! The helpers cover all TypeRef variants that appear in plugin trait methods:
//! user DTOs, primitive values, byte buffers, string collections, and enums.

use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};

/// Returns the Swift FFI type for a Box `alef_*` shim parameter.
///
/// FFI types are what the Rust side passes via `extern "Swift"` shim declarations.
/// They correspond to swift-bridge's native bridging types.
///
/// Examples:
/// - `String` → `"RustString"`
/// - `[UInt8]` / `Bytes` → `"RustVec<UInt8>"`
/// - `Bool` → `"Bool"`
/// - `u32` → `"UInt32"`
/// - `Vec<String>` → `"RustVec<RustString>"`
/// - Named (Codable struct) → `"RustString"` (JSON-encoded)
/// - enum → `"RustString"` (JSON-encoded)
pub fn swift_shim_param_ffi_type(ty: &TypeRef, optional: bool) -> String {
    use crate::core::ir::PrimitiveType;
    let inner = match ty {
        TypeRef::String | TypeRef::Named(_) | TypeRef::Path | TypeRef::Json | TypeRef::Map(_, _) => {
            "RustString".to_string()
        }
        TypeRef::Optional(inner) => return format!("{}?", swift_shim_param_ffi_type(inner, false)),
        TypeRef::Vec(inner) => format!("RustVec<{}>", swift_shim_param_ffi_type(inner, false)),
        TypeRef::Primitive(PrimitiveType::Usize) | TypeRef::Primitive(PrimitiveType::Isize) => "UInt".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "Bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "UInt32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "UInt64".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "Int32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "Int64".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "Float".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "Double".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "UInt8".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "Int8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "UInt16".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "Int16".to_string(),
        TypeRef::Bytes => "RustVec<UInt8>".to_string(),
        TypeRef::Char => "Character".to_string(),
        TypeRef::Duration => "Double".to_string(),
        TypeRef::Unit => "Void".to_string(),
    };
    if optional { format!("{inner}?") } else { inner }
}

/// Returns the Swift expression that converts an FFI parameter to the typed value
/// the bridge protocol method expects.
///
/// For simple types (String, Bool, primitives), this is a direct conversion or passthrough.
/// For complex types (Codable structs, enums, `Vec<String>`), this involves JSON decoding.
///
/// Returns `ParamDecode` with:
/// - `setup`: Vec of setup lines to emit before the bridge call (e.g., `let cfg = try JSONDecoder...`)
/// - `expr`: The expression to pass as the bridge argument
/// - `is_throwing`: Whether the decode itself can throw (wrapped in try/catch at call site)
pub fn swift_shim_param_decode(
    param_name: &str,
    ty: &TypeRef,
    _optional: bool,
    excluded_types: &std::collections::HashSet<String>,
) -> ParamDecode {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => ParamDecode {
            setup: vec![],
            expr: param_name.to_string(),
            is_throwing: false,
        },
        TypeRef::Primitive(PrimitiveType::U32)
        | TypeRef::Primitive(PrimitiveType::U64)
        | TypeRef::Primitive(PrimitiveType::I32)
        | TypeRef::Primitive(PrimitiveType::I64)
        | TypeRef::Primitive(PrimitiveType::U8)
        | TypeRef::Primitive(PrimitiveType::I8)
        | TypeRef::Primitive(PrimitiveType::U16)
        | TypeRef::Primitive(PrimitiveType::I16)
        | TypeRef::Primitive(PrimitiveType::Usize)
        | TypeRef::Primitive(PrimitiveType::Isize)
        | TypeRef::Primitive(PrimitiveType::F32)
        | TypeRef::Primitive(PrimitiveType::F64) => ParamDecode {
            setup: vec![],
            expr: param_name.to_string(),
            is_throwing: false,
        },
        TypeRef::String => ParamDecode {
            setup: vec![],
            expr: format!("{}.toString()", param_name),
            is_throwing: false,
        },
        TypeRef::Bytes => ParamDecode {
            setup: vec![],
            expr: format!("Data({})", param_name),
            is_throwing: false,
        },
        TypeRef::Vec(inner_ty) => {
            if vec_element_crosses_as_string(inner_ty, excluded_types) {
                ParamDecode {
                    setup: vec![format!(
                        "var {}_list: [String] = []\n\
                         let {}_count = {}.len()\n\
                         var {}_idx: UInt = 0\n\
                         while {}_idx < {}_count {{\n\
                         \x20   {}_list.append({}.get(index: {}_idx)!.as_str().toString())\n\
                         \x20   {}_idx += 1\n\
                         }}",
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name,
                        param_name
                    )],
                    expr: format!("{}_list", param_name),
                    is_throwing: false,
                }
            } else {
                ParamDecode {
                    setup: vec![],
                    expr: format!("{}.toString()", param_name),
                    is_throwing: false,
                }
            }
        }
        TypeRef::Named(type_name) => {
            if excluded_types.contains(type_name) {
                ParamDecode {
                    setup: vec![],
                    expr: format!("{}.toString()", param_name),
                    is_throwing: false,
                }
            } else {
                let setup = format!(
                    "let {}_decoded = try JSONDecoder().decode({}.self, from: Data({}.toString().utf8))",
                    param_name, type_name, param_name
                );
                ParamDecode {
                    setup: vec![setup],
                    expr: format!("{}_decoded", param_name),
                    is_throwing: true,
                }
            }
        }
        TypeRef::Char => ParamDecode {
            setup: vec![],
            expr: format!("Character({}.toString().first ?? \" \")", param_name),
            is_throwing: false,
        },
        TypeRef::Duration => ParamDecode {
            setup: vec![],
            expr: param_name.to_string(),
            is_throwing: false,
        },
        TypeRef::Unit => ParamDecode {
            setup: vec![],
            expr: "()".to_string(),
            is_throwing: false,
        },
        TypeRef::Optional(inner) => {
            let inner_decode = swift_shim_param_decode(param_name, inner, false, excluded_types);
            if inner_decode.is_throwing {
                let try_expr = format!("try? {}", inner_decode.expr);
                ParamDecode {
                    setup: inner_decode.setup,
                    expr: try_expr,
                    is_throwing: false,
                }
            } else if inner_decode.expr.ends_with("()") {
                let expr = format!(
                    "{}?.{}",
                    param_name,
                    &inner_decode.expr[format!("{}.", param_name).len()..]
                );
                ParamDecode {
                    setup: inner_decode.setup,
                    expr,
                    is_throwing: false,
                }
            } else {
                ParamDecode {
                    setup: inner_decode.setup,
                    expr: inner_decode.expr,
                    is_throwing: false,
                }
            }
        }
        TypeRef::Path => ParamDecode {
            setup: vec![],
            expr: format!("URL(fileURLWithPath: {}.toString())", param_name),
            is_throwing: false,
        },
        TypeRef::Json | TypeRef::Map(_, _) => ParamDecode {
            setup: vec![],
            expr: format!("{}.toString()", param_name),
            is_throwing: false,
        },
    }
}

/// Returns true when a `Vec` element reaches the bridge protocol as a Swift `String`.
///
/// `trait_bridge::swift_type_name` recurses through `Vec`, so a `Vec<Named>` whose element is in
/// the bridge policy set is declared `[String]` on the protocol and travels as `RustVec<RustString>`
/// across the shim -- `gen_rust_crate::plugin_inbound::inbound_bridge_type` maps it to `Vec<String>`
/// on the Rust side for the same reason. A `Vec<String>` and a `Vec<bridged Named>` are therefore
/// the same marshalling problem, and both need the element-wise conversion a `RustVec` requires. ~keep
fn vec_element_crosses_as_string(inner: &TypeRef, excluded_types: &std::collections::HashSet<String>) -> bool {
    match inner {
        TypeRef::String => true,
        TypeRef::Named(name) => excluded_types.contains(name),
        _ => false,
    }
}

/// Result of parameter decode that can be passed to a bridge method.
pub struct ParamDecode {
    /// Lines to emit before the bridge call (declarations, JSON decode, etc.)
    pub setup: Vec<String>,
    /// The expression to pass as the bridge argument.
    pub expr: String,
    /// Whether this decode sequence can throw (requires try/catch wrapping).
    pub is_throwing: bool,
}

/// Returns the Swift FFI return type for the Box `alef_*` shim.
///
/// Rules:
/// - If method has an error_type (throws): always `"String"` (JSON envelope).
/// - If method returns Unit and no error: `"Void"`.
/// - If method returns Bool and no error: `"Bool"`.
/// - If method returns primitive int and no error: the mapped type (UInt32, Int64, etc.).
/// - If method returns `Vec<String>` or `Vec<Named>` and no error: `"RustVec<RustString>"`.
/// - If method returns [other complex] and no error: `"RustString"` (envelope).
///
/// `Vec<Named>` shares the `Vec<String>` mapping because a bridged `Named` crosses as a JSON
/// `String`: the protocol declares `[String]` and `inbound_bridge_type` declares `Vec<String>` on
/// the Rust side, so a single `RustString` envelope would match neither end. ~keep
pub fn swift_shim_return_ffi_type(method: &MethodDef) -> String {
    if method.error_type.is_some() {
        return "String".to_string();
    }

    match &method.return_type {
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "Bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "UInt32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "UInt64".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "Int32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "Int64".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "UInt8".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "Int8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "UInt16".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "Int16".to_string(),
        TypeRef::Primitive(PrimitiveType::Usize) => "UInt".to_string(),
        TypeRef::Primitive(PrimitiveType::Isize) => "UInt".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "Float".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "Double".to_string(),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Named(_)) => {
            "RustVec<RustString>".to_string()
        }
        _ => "RustString".to_string(),
    }
}

/// Returns the Swift body lines that wrap the bridge call result into the FFI return type.
///
/// Handles:
/// - Throwing methods returning Unit: encode `{"ok":null}` on success, `{"err": "..."}` on error
/// - Throwing methods returning T: encode `{"ok": <T>}` / `{"err": "..."}`
/// - Non-throwing methods: passthrough the result or build RustVec for `Vec<String>` / `Vec<Named>`
/// - String return types are wrapped in RustString for FFI boundary
///
/// A non-throwing `Unit` method still has to *call* the bridge: the shim returns nothing, but the
/// conformer's side effect is the entire point of the method, and dropping `bridge_call_expr` here
/// made every such method a silent no-op that compiled cleanly. ~keep
///
/// An `Optional(Named)` return arrives from the bridge as `String?` while the Rust side declares a
/// plain `String` and decodes it with `serde_json::from_str::<Option<T>>`, so `nil` is sent as the
/// JSON literal `null` rather than as a Swift optional. ~keep
///
/// A `Named` return is wrapped in `RustString` directly, with no JSON encoding step, because the
/// bridge protocol already declares it as a `String`: `excluded_named_type_bridge_policy` puts
/// every `Named` type a bridged trait mentions into the JSON-string policy, so `bridge_call_expr`
/// evaluates to a JSON `String` the conformer produced. Encoding it again here would double-encode
/// the payload, and — since these shims are emitted into `Sources/RustBridge/`, which cannot name
/// a downstream DTO — would not even compile. That was alef #258's second failure. ~keep
///
/// The `bridge_call_expr` is the expression that calls the inner bridge method
/// (e.g., `bridge.processImage(imageBytes: imageBytes, config: config)`).
///
/// Returns lines to emit as the method body (from opening brace to closing brace).
pub fn swift_shim_return_marshal(method: &MethodDef, bridge_call_expr: &str) -> Vec<String> {
    if method.error_type.is_some() {
        match &method.return_type {
            TypeRef::Unit => vec![
                "do {".to_string(),
                format!("  try {}", bridge_call_expr),
                "  return encodeOkVoidEnvelope()".to_string(),
                "} catch { return encodeErrEnvelope(\"\\(error)\") }".to_string(),
            ],
            _ => {
                vec![
                    "do {".to_string(),
                    format!("  let result = try {}", bridge_call_expr),
                    "  return encodeOkEnvelope(result)".to_string(),
                    "} catch { return encodeErrEnvelope(\"\\(error)\") }".to_string(),
                ]
            }
        }
    } else {
        match &method.return_type {
            TypeRef::Unit => vec![bridge_call_expr.to_string(), "return ()".to_string()],
            TypeRef::String => {
                vec![format!("return RustString({})", bridge_call_expr)]
            }
            TypeRef::Named(_) => vec![format!("return RustString({})", bridge_call_expr)],
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                vec![format!("return RustString({} ?? \"null\")", bridge_call_expr)]
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Named(_)) => {
                vec![
                    format!("let strings = {}", bridge_call_expr),
                    "let vec = RustVec<RustString>()".to_string(),
                    "for s in strings { vec.push(value: RustString(s)) }".to_string(),
                    "return vec".to_string(),
                ]
            }
            TypeRef::Primitive(PrimitiveType::Usize) | TypeRef::Primitive(PrimitiveType::Isize) => {
                vec![format!("return UInt({})", bridge_call_expr)]
            }
            // `Map` (alef-tasks #309) and every other shape that falls through to
            // `swift_shim_return_ffi_type`'s own catch-all declare the shim's return type as
            // `RustString`. A bare `return {bridge_call_expr}` type-checks only when the bridge
            // call's own Swift return type (`swift_type_name`) already happens to be
            // `RustString`, which it never is -- `swift_type_name` never produces the FFI
            // wrapper type, only plain `String`. Checking the FFI type here keeps this
            // catch-all honest for whichever shape lands in it, without hand-enumerating every
            // one of them the way the explicit arms above do. ~keep
            _ if swift_shim_return_ffi_type(method) == "RustString" => {
                vec![format!("return RustString({})", bridge_call_expr)]
            }
            _ => vec![format!("return {}", bridge_call_expr)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_method(
        name: &str,
        params: Vec<(String, TypeRef, bool)>,
        return_type: TypeRef,
        error_type: Option<String>,
    ) -> MethodDef {
        use crate::core::ir::ParamDef;
        MethodDef {
            name: name.to_string(),
            params: params
                .into_iter()
                .map(|(name, ty, optional)| ParamDef {
                    name,
                    ty,
                    optional,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                })
                .collect(),
            return_type,
            error_type,
            is_async: false,
            is_static: false,
            doc: String::new(),
            receiver: None,
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn test_shim_param_ffi_type_string() {
        assert_eq!(swift_shim_param_ffi_type(&TypeRef::String, false), "RustString");
    }

    #[test]
    fn test_shim_param_ffi_type_bytes() {
        assert_eq!(swift_shim_param_ffi_type(&TypeRef::Bytes, false), "RustVec<UInt8>");
    }

    #[test]
    fn test_shim_param_ffi_type_bool() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Primitive(PrimitiveType::Bool), false),
            "Bool"
        );
    }

    #[test]
    fn test_shim_param_ffi_type_u32() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Primitive(PrimitiveType::U32), false),
            "UInt32"
        );
    }

    #[test]
    fn test_shim_param_ffi_type_vec_string() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Vec(Box::new(TypeRef::String)), false),
            "RustVec<RustString>"
        );
    }

    #[test]
    fn test_shim_param_ffi_type_optional_string() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Optional(Box::new(TypeRef::String)), false),
            "RustString?"
        );
    }

    #[test]
    fn test_shim_param_ffi_type_named() {
        assert_eq!(
            swift_shim_param_ffi_type(&TypeRef::Named("ParseConfig".to_string()), false),
            "RustString"
        );
    }

    #[test]
    fn test_param_decode_string() {
        let decode = swift_shim_param_decode("config", &TypeRef::String, false, &std::collections::HashSet::new());
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "config.toString()");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_bytes() {
        let decode = swift_shim_param_decode("image_bytes", &TypeRef::Bytes, false, &std::collections::HashSet::new());
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "Data(image_bytes)");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_bool() {
        let decode = swift_shim_param_decode(
            "flag",
            &TypeRef::Primitive(PrimitiveType::Bool),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "flag");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_u32() {
        let decode = swift_shim_param_decode(
            "count",
            &TypeRef::Primitive(PrimitiveType::U32),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "count");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_vec_string() {
        let decode = swift_shim_param_decode(
            "langs",
            &TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(!decode.setup.is_empty());
        assert!(decode.setup[0].contains("langs_list"));
        assert_eq!(decode.expr, "langs_list");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_named_codable() {
        let decode = swift_shim_param_decode(
            "cfg",
            &TypeRef::Named("ParseConfig".to_string()),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(!decode.setup.is_empty());
        assert!(decode.setup[0].contains("JSONDecoder"));
        assert!(decode.setup[0].contains("ParseConfig"));
        assert_eq!(decode.expr, "cfg_decoded");
        assert!(decode.is_throwing);
    }

    #[test]
    fn test_param_decode_optional_string() {
        let decode = swift_shim_param_decode(
            "opt_str",
            &TypeRef::Optional(Box::new(TypeRef::String)),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "opt_str?.toString()");
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_param_decode_optional_named() {
        let decode = swift_shim_param_decode(
            "opt_cfg",
            &TypeRef::Optional(Box::new(TypeRef::Named("Config".to_string()))),
            false,
            &std::collections::HashSet::new(),
        );
        assert!(!decode.setup.is_empty());
        assert!(decode.expr.contains("try?"));
        assert!(!decode.is_throwing);
    }

    #[test]
    fn test_return_ffi_type_throwing_unit() {
        let method = make_method("initialize", vec![], TypeRef::Unit, Some("Error".to_string()));
        assert_eq!(swift_shim_return_ffi_type(&method), "String");
    }

    #[test]
    fn test_return_ffi_type_throwing_string() {
        let method = make_method("process", vec![], TypeRef::String, Some("Error".to_string()));
        assert_eq!(swift_shim_return_ffi_type(&method), "String");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_unit() {
        let method = make_method("get_value", vec![], TypeRef::Unit, None);
        assert_eq!(swift_shim_return_ffi_type(&method), "Void");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_bool() {
        let method = make_method("supports_lang", vec![], TypeRef::Primitive(PrimitiveType::Bool), None);
        assert_eq!(swift_shim_return_ffi_type(&method), "Bool");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_u64() {
        let method = make_method("get_size", vec![], TypeRef::Primitive(PrimitiveType::U64), None);
        assert_eq!(swift_shim_return_ffi_type(&method), "UInt64");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_vec_string() {
        let method = make_method("languages", vec![], TypeRef::Vec(Box::new(TypeRef::String)), None);
        assert_eq!(swift_shim_return_ffi_type(&method), "RustVec<RustString>");
    }

    #[test]
    fn test_return_ffi_type_non_throwing_named() {
        let method = make_method("process", vec![], TypeRef::Named("ParseResult".to_string()), None);
        assert_eq!(swift_shim_return_ffi_type(&method), "RustString");
    }

    #[test]
    fn test_return_marshal_throwing_unit() {
        let method = make_method("initialize", vec![], TypeRef::Unit, Some("Error".to_string()));
        let lines = swift_shim_return_marshal(&method, "try inner.initialize()");
        assert_eq!(lines[0], "do {");
        assert!(lines.join("\n").contains("encodeOkVoidEnvelope"));
        assert!(lines.join("\n").contains("encodeErrEnvelope"));
    }

    #[test]
    fn test_return_marshal_throwing_string() {
        let method = make_method("process", vec![], TypeRef::String, Some("Error".to_string()));
        let lines = swift_shim_return_marshal(&method, "try inner.process()");
        assert_eq!(lines[0], "do {");
        assert!(lines.join("\n").contains("encodeOkEnvelope"));
        assert!(lines.join("\n").contains("encodeErrEnvelope"));
    }

    /// A non-throwing `Unit` method exists purely for its side effect, so the shim body must
    /// contain the bridge call. Emitting only `return ()` type-checks and ships a method that
    /// never reaches the conformer -- a defect no compile gate can see.
    #[test]
    fn test_return_marshal_non_throwing_unit_still_calls_the_bridge() {
        let method = make_method("get_value", vec![], TypeRef::Unit, None);
        let lines = swift_shim_return_marshal(&method, "inner.getValue()");
        assert_eq!(lines, vec!["inner.getValue()".to_string(), "return ()".to_string()]);
    }

    #[test]
    fn test_return_marshal_non_throwing_bool() {
        let method = make_method("supports_lang", vec![], TypeRef::Primitive(PrimitiveType::Bool), None);
        let lines = swift_shim_return_marshal(&method, "inner.supportsLanguage(lang)");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("return"));
    }

    #[test]
    fn test_return_marshal_non_throwing_vec_string() {
        let method = make_method("languages", vec![], TypeRef::Vec(Box::new(TypeRef::String)), None);
        let lines = swift_shim_return_marshal(&method, "inner.languages()");
        assert!(lines.join("\n").contains("RustVec<RustString>"));
        assert!(lines.join("\n").contains("vec.push"));
    }

    #[test]
    fn test_param_decode_path_url() {
        let decode = swift_shim_param_decode("path", &TypeRef::Path, false, &std::collections::HashSet::new());
        assert!(decode.setup.is_empty());
        assert_eq!(decode.expr, "URL(fileURLWithPath: path.toString())");
        assert!(!decode.is_throwing);
    }

    /// Every `Named` return -- struct or enum -- reaches this function already declared as a JSON
    /// `String` by `excluded_named_type_bridge_policy`, so the shim wraps the bridge call and
    /// nothing more.
    #[test]
    fn test_return_marshal_non_throwing_named_struct_wraps_rust_string() {
        let method = make_method(
            "backend_type",
            vec![],
            TypeRef::Named("TextBackendType".to_string()),
            None,
        );
        let lines = swift_shim_return_marshal(&method, "bridge.backendType()");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "return RustString(bridge.backendType())");
    }

    /// alef #258: 0.67.5 routed an enum-typed return through `JSONEncoder().encode(...)` here.
    /// That was a fix for the wrong layer -- the bridge protocol should never have declared the
    /// return as the enum's own type in the first place -- and it could not compile, because
    /// these shims are emitted into `Sources/RustBridge/`, which cannot name (let alone prove
    /// `Encodable`) a DTO from the target that depends on it. The bridge call yields the JSON
    /// `String` the conformer produced; encoding it again would double-encode the payload.
    #[test]
    fn test_return_marshal_named_enum_is_not_re_encoded() {
        let method = make_method(
            "confidence_level",
            vec![],
            TypeRef::Named("ConfidenceLevel".to_string()),
            None,
        );
        let lines = swift_shim_return_marshal(&method, "bridge.confidenceLevel()");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "return RustString(bridge.confidenceLevel())");
        assert!(
            !lines[0].contains("JSONEncoder"),
            "a bridged Named return arrives as JSON already; re-encoding double-encodes it, got:\n{}",
            lines[0]
        );
    }

    /// A `Vec<Named>` element is a JSON `String` by the time it reaches the shim, so the shim
    /// declares the same `RustVec<RustString>` a `Vec<String>` does. The catch-all `RustString`
    /// contradicted both the `[String]` the protocol declares and the `Vec<String>` the Rust
    /// extern block declares.
    #[test]
    fn test_return_ffi_type_non_throwing_vec_named_matches_vec_string() {
        let named = make_method(
            "stats_history",
            vec![],
            TypeRef::Vec(Box::new(TypeRef::Named("SinkStats".to_string()))),
            None,
        );
        let strings = make_method("languages", vec![], TypeRef::Vec(Box::new(TypeRef::String)), None);
        assert_eq!(swift_shim_return_ffi_type(&named), "RustVec<RustString>");
        assert_eq!(
            swift_shim_return_ffi_type(&named),
            swift_shim_return_ffi_type(&strings),
            "a bridged Named element and a String element cross identically"
        );
    }

    /// The bridge hands back `[String]` of JSON payloads. Each element is wrapped, never re-encoded:
    /// a `JSONEncoder` pass here would double-encode every element and still compile.
    #[test]
    fn test_return_marshal_vec_named_builds_rust_vec_without_re_encoding() {
        let method = make_method(
            "stats_history",
            vec![],
            TypeRef::Vec(Box::new(TypeRef::Named("SinkStats".to_string()))),
            None,
        );
        let lines = swift_shim_return_marshal(&method, "bridge.statsHistory()");
        assert_eq!(
            lines,
            vec![
                "let strings = bridge.statsHistory()".to_string(),
                "let vec = RustVec<RustString>()".to_string(),
                "for s in strings { vec.push(value: RustString(s)) }".to_string(),
                "return vec".to_string(),
            ]
        );
        assert!(
            !lines.join("\n").contains("JSONEncoder"),
            "elements arrive as JSON already; re-encoding double-encodes them, got:\n{}",
            lines.join("\n")
        );
    }

    /// A `Vec<Named>` parameter arrives as `RustVec<RustString>` and the protocol expects
    /// `[String]`, so it needs the same element-wise walk `Vec<String>` gets. `.toString()` is not
    /// a member of `RustVec`.
    #[test]
    fn test_param_decode_vec_named_walks_the_rust_vec() {
        let excluded = std::collections::HashSet::from(["SinkStats".to_string()]);
        let decode = swift_shim_param_decode(
            "entries",
            &TypeRef::Vec(Box::new(TypeRef::Named("SinkStats".to_string()))),
            false,
            &excluded,
        );
        assert_eq!(decode.expr, "entries_list");
        assert!(
            decode.setup[0].contains("var entries_list: [String] = []"),
            "expected an element-wise RustVec walk, got:\n{}",
            decode.setup[0]
        );
        assert!(!decode.is_throwing);
    }

    /// The bridge declares `String?` while the Rust extern declares a plain `String` it feeds to
    /// `serde_json::from_str::<Option<T>>`. `nil` therefore has to become the JSON literal `null`.
    #[test]
    fn test_return_marshal_optional_named_sends_json_null_for_nil() {
        let method = make_method(
            "last_stats",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::Named("SinkStats".to_string()))),
            None,
        );
        let lines = swift_shim_return_marshal(&method, "bridge.lastStats()");
        assert_eq!(
            lines,
            vec![r#"return RustString(bridge.lastStats() ?? "null")"#.to_string()]
        );
    }

    #[test]
    fn test_return_marshal_non_throwing_usize() {
        let method = make_method("dimensions", vec![], TypeRef::Primitive(PrimitiveType::Usize), None);
        let lines = swift_shim_return_marshal(&method, "bridge.dimensions()");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "return UInt(bridge.dimensions())");
    }

    #[test]
    fn test_return_marshal_vec_vec_f32_with_error() {
        let method = make_method(
            "embed",
            vec![],
            TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F32))))),
            Some("Error".to_string()),
        );
        let lines = swift_shim_return_marshal(&method, "try inner.embed(texts)");
        assert!(lines.join("\n").contains("encodeOkEnvelope"));
    }

    /// alef-tasks #309: `swift_shim_return_ffi_type` declares `Map` returns as `RustString`, so
    /// the marshal must wrap the bridge call in `RustString(...)` -- a bare `return
    /// bridge.sinkTotals()` would return the bridge's own `String` where the shim's declared
    /// `RustString` return type is expected, and would not compile.
    #[test]
    fn test_return_marshal_map_named_value_wraps_rust_string() {
        let method = make_method(
            "sink_totals",
            vec![],
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("SinkStats".to_string())),
            ),
            None,
        );
        assert_eq!(swift_shim_return_ffi_type(&method), "RustString");
        let lines = swift_shim_return_marshal(&method, "bridge.sinkTotals()");
        assert_eq!(lines, vec!["return RustString(bridge.sinkTotals())".to_string()]);
    }

    /// The bool/u32/... primitive catch-all shapes must keep passing through unwrapped: their
    /// FFI return type is the primitive itself, not `RustString`, so wrapping them would break
    /// compilation the same way the missing wrap broke `Map`.
    #[test]
    fn test_return_marshal_bool_catch_all_stays_unwrapped() {
        let method = make_method("supports_lang", vec![], TypeRef::Primitive(PrimitiveType::Bool), None);
        assert_eq!(swift_shim_return_ffi_type(&method), "Bool");
        let lines = swift_shim_return_marshal(&method, "bridge.supportsLang()");
        assert_eq!(lines, vec!["return bridge.supportsLang()".to_string()]);
    }
}
