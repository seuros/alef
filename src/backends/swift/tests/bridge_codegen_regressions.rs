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
use crate::core::ir::{MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};
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
    let bridge_cfg = TraitBridgeConfig {
        trait_name: trait_def.name.clone(),
        register_fn: Some(format!("register{}", trait_def.name)),
        ..Default::default()
    };
    let bridges = vec![(trait_def.name.clone(), &bridge_cfg, trait_def)];
    let files = gen_trait_bridge_files(&bridges, &HashSet::new(), &HashSet::new());
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

/// A trait with a defaulted method touching a DTO type in both return and parameter position --
/// the shape alef #258 marshalled incorrectly twice.
fn trait_with_defaulted_dto_methods() -> TypeDef {
    let mut page_layout = method("page_layout", vec![], TypeRef::Named("PageLayout".to_string()), None);
    page_layout.has_default_impl = true;

    let mut describe = method(
        "describe",
        vec![param("layout", TypeRef::Named("PageLayout".to_string()))],
        TypeRef::String,
        None,
    );
    describe.has_default_impl = true;

    let mut is_ready = method("is_ready", vec![], TypeRef::Primitive(PrimitiveType::Bool), None);
    is_ready.has_default_impl = true;

    make_trait("DocumentSink", vec![page_layout, describe, is_ready])
}

/// alef #258, root cause: `excluded_named_type_bridge_policy` exempted `has_default_impl`
/// methods, so a defaulted method's `Named` types kept their real Swift names. Those names do not
/// resolve in `Sources/RustBridge/`, where this file is emitted. Every `Named` type a bridged
/// trait mentions crosses as a JSON `String`, defaulted method or not.
#[test]
fn defaulted_method_named_types_cross_the_boundary_as_json_strings() {
    let source = bridge_source(&trait_with_defaulted_dto_methods());

    assert!(
        source.contains("func pageLayout() -> String"),
        "a defaulted method's enum return must be declared as a JSON String, got:\n{source}"
    );
    assert!(
        source.contains("func describe(layout: String) -> String"),
        "a defaulted method's enum parameter must be declared as a JSON String, got:\n{source}"
    );
    assert!(
        !source.contains("PageLayout"),
        "no DTO type name may appear in a file emitted into RustBridge, got:\n{source}"
    );
}

/// alef #258, second failure: rather than fix the policy, 0.67.5 synthesized a default body --
/// `return .<firstFieldlessCase>` -- which is not the Rust `Default` for any type alef can see.
/// The IR carries `has_default_impl` but never the default *body*, so alef cannot know the value,
/// and the inbound Rust wrapper calls the Swift shim for defaulted methods too, so a guess would
/// replace the Rust default at runtime rather than sit unused. It must not emit one.
#[test]
fn no_default_method_bodies_are_synthesized() {
    let source = bridge_source(&trait_with_defaulted_dto_methods());

    assert!(
        !source.contains("public extension SwiftDocumentSinkBridge"),
        "alef must not ship a default-implementation extension it cannot populate correctly, got:\n{source}"
    );
    for invented in ["return .", "return true", "return \"{}\"", "return \"\""] {
        assert!(
            !source.contains(invented),
            "invented default body {invented:?} must not be emitted, got:\n{source}"
        );
    }
}

/// A conformer has to know that alef dropped the stub deliberately and that the Rust trait has a
/// real default it must reproduce -- otherwise the required conformance reads as an alef bug.
#[test]
fn defaulted_methods_are_annotated_with_why_no_stub_is_provided() {
    let source = bridge_source(&trait_with_defaulted_dto_methods());

    assert!(
        source.contains("supply the same value the Rust default would have produced"),
        "defaulted protocol methods must explain the missing stub, got:\n{source}"
    );
}

/// The note is specific to defaulted methods; a method Rust requires must not claim to have a
/// default. Without this, the previous assertion would also pass on a note emitted unconditionally.
#[test]
fn methods_without_a_rust_default_carry_no_default_note() {
    let source = bridge_source(&make_trait(
        "DocumentSink",
        vec![method(
            "accept",
            vec![param("chunk", TypeRef::String)],
            TypeRef::Unit,
            None,
        )],
    ));

    assert!(
        !source.contains("supply the same value the Rust default would have produced"),
        "a method with no Rust default must not be annotated as defaulted, got:\n{source}"
    );
}
