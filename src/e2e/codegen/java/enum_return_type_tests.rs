//! Regression coverage for enum-typed trait-bridge return values in the Java e2e stub.
//!
//! `build_args_and_setup`'s `test_backend` arm computed `excluded_named` via
//! `trait_bridge_excluded_type_names` (the enum-blind wrapper): `collect_hidden_named_types`
//! looks a return type's name up in `type_by_name`, which is built from `type_defs` alone.
//! Enums live in a separate `EnumDef` registry, so a method returning a real, exported enum
//! always missed that lookup and fell into the "unknown, therefore excluded" branch --
//! exactly like a genuinely opaque type. `java_stub_type_with_context` then substituted
//! `String` for the enum's real class name, leaving the stub's method signature mismatched
//! against the real interface's abstract `SampleClassification sample_classification()`: "is
//! not abstract and does not override abstract method". Passing the real enum names through
//! `_with_enums` (mirroring `csharp::setup::build_args_and_setup`) fixes this without
//! touching `java_stub_type_with_context` or `java_stub_default_with_context` at all -- both
//! already handle a non-excluded `Named` type correctly (qualified class name, `null`
//! default).

use super::args::{JavaArgsContext, build_args_and_setup};
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{EnumDef, EnumVariant, MethodDef, TypeDef, TypeRef};
use crate::e2e::codegen::call_ir::TargetParams;
use crate::e2e::config::ArgMapping;
use crate::e2e::fixture::Fixture;

fn fixture(id: &str) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "description": "test",
        "input": {},
    }))
    .expect("fixture")
}

fn test_backend_arg(trait_name: &str) -> ArgMapping {
    ArgMapping {
        name: "backend".to_string(),
        field: "input.backend".to_string(),
        arg_type: "test_backend".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some(trait_name.to_string()),
    }
}

fn trait_type_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("neutral_crate::plugins::{name}"),
        methods,
        is_trait: true,
        is_opaque: true,
        ..TypeDef::default()
    }
}

fn method(name: &str, ret: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type: ret,
        ..MethodDef::default()
    }
}

fn unit_enum(name: &str, variant: &str) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("neutral_crate::{name}"),
        variants: vec![EnumVariant {
            name: variant.to_string(),
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }
}

fn render_stub(config: &ResolvedCrateConfig, type_defs: &[TypeDef], enums: &[EnumDef]) -> String {
    let fixture = fixture("register_sample_backend_trait_bridge");
    let arg = test_backend_arg("SampleBackend");
    let mut teardown = String::new();
    let (setup, _args) = build_args_and_setup(
        &fixture.input,
        &[arg],
        JavaArgsContext {
            class_name: "Sample",
            options_type: None,
            fixture: &fixture,
            adapter_request_type: None,
            owner_handle_is_receiver: false,
            config,
            type_defs,
            enums,
            target_params: TargetParams::IrAbsent,
            teardown_block: &mut teardown,
        },
    );
    setup.join("\n")
}

/// The core regression: a method returning a real, exported enum must keep the enum's own
/// class name in the stub signature and default to `null` -- not fall through to `String`.
#[test]
fn enum_returning_method_keeps_its_real_type_not_string() {
    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "SampleBackend".to_string(),
            super_trait: None,
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = [trait_type_def(
        "SampleBackend",
        vec![method("sample_classification", TypeRef::Named("SampleClassification".to_string()))],
    )];
    let enums = [unit_enum("SampleClassification", "Baseline")];

    let stub = render_stub(&config, &type_defs, &enums);

    assert!(
        stub.contains("SampleClassification sample_classification()"),
        "stub must declare the enum's real return type, not String, got:\n{stub}"
    );
    assert!(
        !stub.contains("String sample_classification()"),
        "stub must not substitute String for a real, exported enum, got:\n{stub}"
    );
    assert!(
        stub.contains("return null;"),
        "a non-excluded Named return still safely defaults to null, got:\n{stub}"
    );
}

/// Negative control: a method returning a type with no matching `EnumDef` in scope (simulating
/// a type genuinely excluded for this language, e.g. by `exclude_types`) must still fall
/// through to the pre-existing `String` substitution -- this is not "every Named type passes
/// through unchanged", only ones the enum registry actually vouches for.
#[test]
fn a_type_absent_from_the_enum_registry_still_gets_the_string_substitution() {
    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "SampleBackend".to_string(),
            super_trait: None,
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = [trait_type_def(
        "SampleBackend",
        vec![method("backend_kind", TypeRef::Named("ExcludedBackendKind".to_string()))],
    )];
    // No `EnumDef` for "ExcludedBackendKind" -- as if this language's `exclude_types` config
    // already dropped it from the enum registry before this code ever sees it.
    let enums: [EnumDef; 0] = [];

    let stub = render_stub(&config, &type_defs, &enums);

    assert!(
        stub.contains("String backend_kind()"),
        "a type absent from the enum registry must still fall back to String, got:\n{stub}"
    );
}

/// Negative control: a plain scalar-returning method is unaffected by the excluded-types
/// computation either way.
#[test]
fn scalar_returning_method_is_unaffected() {
    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "SampleBackend".to_string(),
            super_trait: None,
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = [trait_type_def(
        "SampleBackend",
        vec![method(
            "priority",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
        )],
    )];
    let enums = [unit_enum("SampleClassification", "Baseline")];

    let stub = render_stub(&config, &type_defs, &enums);

    assert!(stub.contains("int priority()"), "got:\n{stub}");
}
