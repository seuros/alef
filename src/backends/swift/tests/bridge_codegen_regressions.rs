//! Regression tests for Swift trait bridge codegen bugs.
//!
//! B2 (`84eaa503d`) — the bridge adapter silently dropped methods, so a protocol
//! method had no `…Call` counterpart and every Swift call site failed to compile.
//! B3 (`428463e86`) — `String` / `Vec<String>` returns were marshalled straight from
//! `RustString`, so the adapter handed back `RustString` where the declared signature
//! promised a native Swift `String`.
//!
//! B4 (JSON-arg dispatch by parameter type) and B5 (throwing-closure `try` placement)
//! are pinned in `gen_bindings::overloads` and `gen_bindings::forwarders` instead:
//! both modules are private to `gen_bindings`, so they are unreachable from here and
//! their assertions live in those modules' own co-located test modules. ~keep

use crate::backends::swift::gen_bindings::trait_bridge::gen_trait_bridge_files;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};
use std::collections::HashSet;

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, error_type: Option<&str>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        error_type: error_type.map(|name| name.to_string()),
        ..Default::default()
    }
}

fn make_trait(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("testcrate::{name}"),
        is_trait: true,
        methods,
        ..Default::default()
    }
}

/// Render the per-trait bridge file (`Swift{Trait}Bridge.swift`), which carries both the
/// protocol declarations and the adapter class.
fn bridge_source(trait_def: &TypeDef) -> String {
    bridge_source_with_api(trait_def, &ApiSurface::default())
}

/// Like `bridge_source`, but with a caller-supplied `ApiSurface` -- needed for cases where the
/// generated content depends on IR lookups beyond the trait itself (e.g. `ApiSurface::enums`).
fn bridge_source_with_api(trait_def: &TypeDef, api: &ApiSurface) -> String {
    let bridge_cfg = TraitBridgeConfig {
        trait_name: trait_def.name.clone(),
        register_fn: Some(format!("register{}", trait_def.name)),
        ..Default::default()
    };
    let bridges = vec![(trait_def.name.clone(), &bridge_cfg, trait_def)];
    let files = gen_trait_bridge_files(&bridges, &HashSet::new(), &HashSet::new(), api);
    let wanted = format!("Swift{}Bridge.swift", trait_def.name);
    files
        .into_iter()
        .find(|(name, _)| *name == wanted)
        .unwrap_or_else(|| panic!("expected {wanted} among generated trait bridge files"))
        .1
}

/// B2: every protocol method must have a matching `…Call` adapter method.
///
/// The protocol loop and the adapter loop must walk `trait_def.methods` in lockstep;
/// when the adapter skipped a method the protocol still declared it, and Swift failed
/// with "does not conform to protocol" at every registration site.
#[test]
fn adapter_emits_a_call_method_for_every_protocol_method() {
    let source = bridge_source(&make_trait(
        "TextBackend",
        vec![
            method(
                "find_all",
                vec![param("text", TypeRef::String)],
                TypeRef::Vec(Box::new(TypeRef::String)),
                Some("BackendError"),
            ),
            method("scan", vec![], TypeRef::Primitive(PrimitiveType::Bool), None),
        ],
    ));

    assert!(
        source.contains("func findAll(text: String) throws -> [String]"),
        "protocol must declare findAll: {source}"
    );
    assert!(
        source.contains("func findAllCall(text: String) throws -> String {"),
        "adapter must register findAllCall: {source}"
    );
    assert!(
        source.contains("func scan() -> Bool"),
        "protocol must declare scan: {source}"
    );
    assert!(
        source.contains("func scanCall() -> Bool {"),
        "adapter must register scanCall: {source}"
    );
}

/// B3: a `String` return must be converted to a native Swift `String` before marshalling.
#[test]
fn adapter_converts_string_return_to_native_string() {
    let source = bridge_source(&make_trait(
        "TextBackend",
        vec![method("extract_text", vec![], TypeRef::String, Some("BackendError"))],
    ));

    assert!(
        source.contains("return marshal_ok_result(String(result))"),
        "String returns must be wrapped via String(result): {source}"
    );
}

/// B3: a `Vec<String>` return must be converted element-wise, not handed back as `[RustString]`.
#[test]
fn adapter_converts_vec_string_return_element_wise() {
    let source = bridge_source(&make_trait(
        "TextBackend",
        vec![method(
            "find_all",
            vec![],
            TypeRef::Vec(Box::new(TypeRef::String)),
            Some("BackendError"),
        )],
    ));

    assert!(
        source.contains("return marshal_ok_result(result.map { String($0) })"),
        "Vec<String> returns must be converted element-wise: {source}"
    );
}

/// Regression test for alef #258: a trait method that both has a Rust-side default
/// implementation and returns an enum produced a Swift default stub body of `return "{}"`,
/// a `String` literal that does not type-check against the enum-typed declared return
/// (`swift_return_type` declares a non-excluded `Named` return as the enum's own Swift type,
/// not `String`). The default body must ask the IR for a real case of that enum instead.
#[test]
fn default_method_for_enum_return_constructs_a_real_enum_case() {
    let enum_def = EnumDef {
        name: "ConfidenceLevel".to_string(),
        rust_path: "testcrate::ConfidenceLevel".to_string(),
        variants: vec![
            EnumVariant {
                name: "High".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Low".to_string(),
                ..Default::default()
            },
        ],
        has_serde: true,
        ..Default::default()
    };
    let api = ApiSurface {
        enums: vec![enum_def],
        ..Default::default()
    };

    let mut confidence_method = method(
        "confidence_level",
        vec![],
        TypeRef::Named("ConfidenceLevel".to_string()),
        None,
    );
    confidence_method.has_default_impl = true;

    let source = bridge_source_with_api(&make_trait("TextBackend", vec![confidence_method]), &api);

    assert!(
        source.contains("return .high"),
        "default stub must construct a real ConfidenceLevel case, got:\n{source}"
    );
    assert!(
        !source.contains("return \"{}\""),
        "default stub must not return a bare string literal for an enum-typed return, got:\n{source}"
    );
}
