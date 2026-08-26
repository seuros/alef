//! Regression coverage for [`super::test_backend::resolve_test_backend_emission`].
//!
//! The Go trait-bridge stub generator merges a configured `super_trait`'s methods into
//! the stub by looking up a `TypeDef` whose `rust_path` equals the configured value. That
//! lookup finds nothing when the super-trait is declared in a private module and
//! re-exported via `pub use` -- its `rust_path` (e.g. `sample_crate::plugins::traits::Plugin`)
//! need not equal the configured, publicly re-exported value (e.g.
//! `sample_crate::plugins::Plugin`). These tests pin that `resolve_test_backend_emission`
//! still produces a stub implementing every method the real Go interface requires even
//! when that lookup fails, by never supplying a `type_defs` entry for the super-trait at
//! all.

use super::test_backend::resolve_test_backend_emission;
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;

fn fixture(id: &str) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "description": "test",
        "input": null,
    }))
    .expect("fixture")
}

fn method(name: &str, ret: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type: ret,
        ..MethodDef::default()
    }
}

fn own_trait_type_def(trait_name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: trait_name.to_string(),
        is_trait: true,
        methods,
        ..TypeDef::default()
    }
}

fn plugin_bridge(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("sample_crate::plugins::Plugin".to_string()),
        register_fn: Some(format!("register_{trait_name}")),
        ..TraitBridgeConfig::default()
    }
}

/// The core regression: `type_defs` contains the trait's own `TypeDef` but no entry
/// whose `rust_path` equals the configured `super_trait` (simulating the private-module
/// `pub use` re-export case) — the stub must still implement all four `Plugin` methods
/// with the exact signatures the real Go interface declares (`Name() string`,
/// `Version() string`, `Initialize() error`, `Shutdown() error`), not silently drop them.
#[test]
fn synthesizes_all_four_plugin_methods_when_the_super_trait_lookup_finds_nothing() {
    let trait_bridge = plugin_bridge("SampleBackend");
    let own_method = method("do_work", TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool));
    let type_defs = vec![own_trait_type_def("SampleBackend", vec![own_method])];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![trait_bridge.clone()],
        ..ResolvedCrateConfig::default()
    };
    let fixture = fixture("synth_all_four");

    let emission = resolve_test_backend_emission(&fixture, "SampleBackend", &trait_bridge, &config, &type_defs, &[], "pkg");

    assert!(
        emission.setup_block.contains("Name() string"),
        "missing Name() string:\n{}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("Version() string"),
        "missing Version() string:\n{}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("Initialize() error"),
        "missing Initialize() error:\n{}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("Shutdown() error"),
        "missing Shutdown() error:\n{}",
        emission.setup_block
    );
    // The trait's own method must still be present alongside the synthesized ones.
    assert!(
        emission.setup_block.contains("DoWork()"),
        "own trait method dropped:\n{}",
        emission.setup_block
    );
}

/// `Name()` must return the fixture id (matching what the super-trait-merge path emits
/// when the lookup *does* succeed), not the generic empty-string default a synthesized
/// method would otherwise fall through to.
#[test]
fn synthesized_name_method_returns_the_fixture_id() {
    let trait_bridge = plugin_bridge("SampleBackend");
    let type_defs = vec![own_trait_type_def("SampleBackend", vec![])];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![trait_bridge.clone()],
        ..ResolvedCrateConfig::default()
    };
    let fixture = fixture("name_returns_fixture_id");

    let emission = resolve_test_backend_emission(&fixture, "SampleBackend", &trait_bridge, &config, &type_defs, &[], "pkg");

    assert!(
        emission
            .setup_block
            .contains("Name() string { return \"name_returns_fixture_id\" }"),
        "Name() must return the fixture id, got:\n{}",
        emission.setup_block
    );
}

/// Negative control: a trait bridge with no `super_trait` configured at all must not
/// synthesize any `Plugin` methods — only the trait's own methods are emitted.
#[test]
fn no_super_trait_configured_synthesizes_nothing() {
    let mut trait_bridge = plugin_bridge("SampleBackend");
    trait_bridge.super_trait = None;
    let type_defs = vec![own_trait_type_def(
        "SampleBackend",
        vec![method("do_work", TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool))],
    )];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![trait_bridge.clone()],
        ..ResolvedCrateConfig::default()
    };
    let fixture = fixture("no_super_trait");

    let emission = resolve_test_backend_emission(&fixture, "SampleBackend", &trait_bridge, &config, &type_defs, &[], "pkg");

    assert!(!emission.setup_block.contains("Initialize"), "{}", emission.setup_block);
    assert!(!emission.setup_block.contains("Shutdown"), "{}", emission.setup_block);
    assert!(!emission.setup_block.contains("func (testStub_no_super_trait) Name"), "{}", emission.setup_block);
}

/// Negative control: when the trait already declares its own `version` method (an
/// unusual but legal shape), the synthesizer must not emit a second, conflicting
/// `Version()` — the real method wins and nothing is duplicated.
#[test]
fn does_not_duplicate_a_method_the_trait_already_declares() {
    let trait_bridge = plugin_bridge("SampleBackend");
    let type_defs = vec![own_trait_type_def(
        "SampleBackend",
        vec![method("version", TypeRef::Primitive(crate::core::ir::PrimitiveType::I32))],
    )];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![trait_bridge.clone()],
        ..ResolvedCrateConfig::default()
    };
    let fixture = fixture("no_duplicate_version");

    let emission = resolve_test_backend_emission(&fixture, "SampleBackend", &trait_bridge, &config, &type_defs, &[], "pkg");

    let version_count = emission.setup_block.matches("Version(").count();
    assert_eq!(version_count, 1, "Version() must appear exactly once:\n{}", emission.setup_block);
    // The real (int32) return type must win over the synthesized `string` fallback.
    assert!(
        emission.setup_block.contains("Version() int32"),
        "the trait's own Version() signature must win, got:\n{}",
        emission.setup_block
    );
}

/// Positive control proving the merge path (the lookup succeeding) is unaffected: when
/// `type_defs` DOES contain an entry whose `rust_path` matches the configured
/// `super_trait`, its methods are used and nothing is synthesized on top of them.
#[test]
fn uses_the_real_super_trait_methods_when_the_lookup_succeeds() {
    let trait_bridge = plugin_bridge("SampleBackend");
    let plugin_type = TypeDef {
        name: "Plugin".to_string(),
        rust_path: "sample_crate::plugins::Plugin".to_string(),
        is_trait: true,
        methods: vec![
            method("name", TypeRef::String),
            method("version", TypeRef::String),
            method("initialize", TypeRef::Unit),
            method("shutdown", TypeRef::Unit),
        ],
        ..TypeDef::default()
    };
    let type_defs = vec![own_trait_type_def("SampleBackend", vec![]), plugin_type];
    let config = ResolvedCrateConfig {
        trait_bridges: vec![trait_bridge.clone()],
        ..ResolvedCrateConfig::default()
    };
    let fixture = fixture("lookup_succeeds");

    let emission = resolve_test_backend_emission(&fixture, "SampleBackend", &trait_bridge, &config, &type_defs, &[], "pkg");

    // All four methods came from the real lookup; none of them must appear a second time
    // via the synthetic fallback.
    for needle in ["Name(", "Version(", "Initialize(", "Shutdown("] {
        let count = emission.setup_block.matches(needle).count();
        assert_eq!(count, 1, "{needle} must appear exactly once:\n{}", emission.setup_block);
    }
}
