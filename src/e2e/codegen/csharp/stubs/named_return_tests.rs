//! Regression coverage for `emit_csharp_stub_default`'s `TypeRef::Named` branch: a
//! non-nullable reference-type return must default to a constructed instance, never
//! `default(T)` -- which is `null` for any C# class and fails CS8603 on a non-nullable
//! return. `CSharpDefaults::emit_default` already uses `new T()` for `TypeRef::Named`
//! everywhere else; this branch exists only to substitute the C#-cased `visible_type` name,
//! and must follow the same convention.

use crate::codegen::defaults::language_defaults;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};
use crate::e2e::fixture::Fixture;

use super::{emit_csharp_stub_default, emit_test_backend};

fn visible_names<'a>(names: &[&'a str]) -> std::collections::HashSet<&'a str> {
    names.iter().copied().collect()
}

/// A non-nullable reference-type return (a visible `Named` struct/record type) must default
/// to a parameterless-constructed instance, never `default(T)`.
#[test]
fn a_visible_named_return_defaults_to_a_constructed_instance() {
    let defaults = language_defaults("csharp");
    let ty = TypeRef::Named("SampleRecord".to_string());
    let names = visible_names(&["SampleRecord"]);

    assert_eq!(
        emit_csharp_stub_default(&ty, "SampleRecord", &*defaults, &names),
        "new SampleRecord()"
    );
}

/// A nullable reference-type return (`Optional<Named>`) must keep its existing `null`
/// default -- this fix must not touch a case that already compiles.
#[test]
fn a_nullable_named_return_keeps_its_null_default() {
    let defaults = language_defaults("csharp");
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("SampleRecord".to_string())));
    let names = visible_names(&["SampleRecord"]);

    assert_eq!(
        emit_csharp_stub_default(&ty, "SampleRecord?", &*defaults, &names),
        "null"
    );
}

/// A value-type (primitive) return must keep its existing scalar default -- this fix is
/// scoped to `TypeRef::Named` only.
#[test]
fn a_primitive_value_type_return_keeps_its_scalar_default() {
    let defaults = language_defaults("csharp");
    let ty = TypeRef::Primitive(PrimitiveType::I64);
    let names = visible_names(&[]);

    assert_eq!(emit_csharp_stub_default(&ty, "long", &*defaults, &names), "0");
}

fn record_returning_method() -> MethodDef {
    MethodDef {
        name: "get_record".to_string(),
        params: vec![ParamDef {
            name: "hint".to_string(),
            ty: TypeRef::String,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("SampleRecord".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: Some("SampleBackend".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// End-to-end companion through the real stub emitter: a trait method returning a visible
/// struct type must never emit `default(T)` in the generated stub body. Reverting the
/// `emit_csharp_stub_default` fix makes this fail: `emission.setup_block` would contain
/// `=> default(SampleRecord);` (CS8603 at compile time) instead of `=> new SampleRecord();`.
#[test]
fn a_struct_returning_trait_method_stub_never_emits_default_of_type() {
    let method = record_returning_method();
    let bridge = TraitBridgeConfig {
        trait_name: "SampleBackend".to_string(),
        ..Default::default()
    };
    let fixture = Fixture {
        id: "sample_backend".to_string(),
        description: "Register a sample backend".to_string(),
        input: serde_json::json!({ "name": "sample-backend" }),
        ..Fixture::default()
    };

    let emission = emit_test_backend(&bridge, &[&method], &fixture);

    assert!(
        !emission.setup_block.contains("default(SampleRecord)"),
        "a non-nullable reference return must never be `default(T)` (CS8603): {}",
        emission.setup_block
    );
    assert!(
        emission
            .setup_block
            .contains("public SampleRecord GetRecord(string hint)"),
        "{}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("=> new SampleRecord();"),
        "{}",
        emission.setup_block
    );
}
