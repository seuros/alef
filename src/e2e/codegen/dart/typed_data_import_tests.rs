//! A trait-bridge snippet must import `dart:typed_data` whenever the stub it emits
//! spells a class from that library.
//!
//! `DartMapper` maps `Vec<u8>` to `Uint8List`, every other `Vec<integer>` to `Int64List`
//! and `Vec<float>` to `Float64List`. Both the stub emitter and the snippet preamble used
//! to look for `Uint8List` alone, so a stub whose methods take or return float or integer
//! vectors compiled nowhere: the class name was emitted, its library never imported.

use super::snippet::render_snippet_body;
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use crate::e2e::config::{ArgMapping, E2eConfig};
use crate::e2e::fixture::Fixture;

fn trait_bridge(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    }
}

fn method_returning(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: true,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
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

fn render_register_snippet(method: MethodDef) -> String {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "register_backend", "description": "register: trait bridge", "input": null
    }))
    .expect("fixture");
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "registerBackend".into();
    e2e_config.call.returns_void = true;
    e2e_config.call.args.push(ArgMapping {
        name: "backend".into(),
        field: "input.backend".into(),
        arg_type: "test_backend".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some("SampleBackend".into()),
    });
    let mut config = ResolvedCrateConfig::default();
    config.trait_bridges.push(trait_bridge("SampleBackend"));
    let type_defs = [TypeDef {
        name: "SampleBackend".into(),
        methods: vec![method],
        ..Default::default()
    }];

    render_snippet_body(&fixture, &e2e_config, &config, &type_defs, &[]).expect("snippet")
}

/// `Vec<Vec<f64>>` renders as `List<Float64List>`. Fails against the pre-fix preamble,
/// which asked only whether the stub text contained `Uint8List`.
#[test]
fn a_stub_returning_a_float_vector_imports_dart_typed_data() {
    let body = render_register_snippet(method_returning(
        "embed",
        TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F64))))),
    ));

    assert!(body.contains("Float64List"), "expected the mapped class name:\n{body}");
    assert!(
        body.contains("import 'dart:typed_data';"),
        "a stub spelling Float64List must import its library:\n{body}"
    );
}

/// `Vec<i64>` renders as `Int64List`, the third class the mapper can produce. Pinned
/// separately so a fix that hardcodes only `Float64List` alongside `Uint8List` still fails.
#[test]
fn a_stub_returning_an_integer_vector_imports_dart_typed_data() {
    let body = render_register_snippet(method_returning(
        "identifiers",
        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
    ));

    assert!(body.contains("Int64List"), "expected the mapped class name:\n{body}");
    assert!(
        body.contains("import 'dart:typed_data';"),
        "a stub spelling Int64List must import its library:\n{body}"
    );
}

/// The import stays out when nothing needs it — `unused_import` is a Dart analyzer
/// diagnostic, so an unconditional import trades one broken snippet for another.
#[test]
fn a_stub_with_no_typed_data_class_does_not_import_the_library() {
    let body = render_register_snippet(method_returning("supported", TypeRef::Primitive(PrimitiveType::Bool)));

    assert!(
        !body.contains("import 'dart:typed_data';"),
        "no typed-data class is spelled, so the import must be absent:\n{body}"
    );
}
