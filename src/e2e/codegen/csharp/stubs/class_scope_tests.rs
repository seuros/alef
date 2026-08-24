//! A trait-bridge test stub has to be legal in both places it is emitted.
//!
//! The e2e test file nests it inside the generated test class, where `private` is
//! redundant but harmless. A docs snippet is top-level statements followed by
//! file-scope declarations, where an explicit `private` is CS1527. Omitting the
//! modifier is legal in both: nested types default to `private`, file-scope types
//! default to `internal`. `build_csharp_visitor` already settled on that spelling
//! (`visitor_class_declaration_has_no_explicit_accessibility_modifier`); the stub
//! emitter is the other half of the same rule.

use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, ReceiverKind, TypeRef};
use crate::e2e::fixture::Fixture;

fn initialize_method() -> MethodDef {
    MethodDef {
        name: "initialize".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: Some("Plugin".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// Fails against the pre-fix emitter, which wrote `private class TestStub_… : I…`.
/// Every trait-bridge registration snippet carried it, so every one of them failed to
/// compile with CS1527 the moment it reached file scope.
#[test]
fn stub_class_declaration_has_no_explicit_accessibility_modifier() {
    let method = initialize_method();
    let bridge = TraitBridgeConfig {
        trait_name: "SampleBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };
    let fixture = Fixture {
        id: "register_sample_backend".to_string(),
        description: "Register a sample backend".to_string(),
        input: serde_json::json!({ "name": "sample" }),
        ..Fixture::default()
    };

    let emission = super::emit_test_backend(&bridge, &[&method], &fixture);

    assert!(
        !emission.setup_block.contains("private"),
        "stub class declaration must not carry an explicit accessibility modifier \
         (CS1527 at file scope): {}",
        emission.setup_block
    );
    assert!(
        emission
            .setup_block
            .contains("class TestStub_RegisterSampleBackend : ISampleBackend"),
        "{}",
        emission.setup_block
    );
}
