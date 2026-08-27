//! Regression coverage for alef task #485: `generate_bindings` named a visitor trait bridge's
//! PHP interface file `{trait_name}.php`, while the class the file's own content declares is
//! `{trait_name}Interface` (`trait_bridge::interfaces::gen_visitor_interface`). PSR-4
//! autoloading requires the class name to match the file basename, so no PHP autoloader could
//! ever resolve the class this file defined -- the interface was unusable no matter how a
//! consumer's `composer.json` was configured.
//!
//! Exercised through the real `generate_bindings` entry point rather than by calling
//! `visitor_interface_class_name` and the interface builder side by side: the bug was a
//! call-site mismatch between two independently-computed names, and a test that recomputes the
//! filename the same way the fixed call site does would pass even if the two drifted apart
//! again. Reading the class name back out of the actual rendered `.php` content is what keeps
//! this pinned to the real coupling. ~keep

use super::rust_bindings::generate_bindings;
use crate::core::config::resolved::ResolvedCrateConfig;
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, MethodDef, ParamDef, TypeDef, TypeRef};

/// A visitor-shaped trait: every method returns the configured result type and takes the
/// configured context type, and every method carries a default impl -- the exact shape
/// `generate_bindings` checks (`is_visitor_bridge`) to route through `gen_visitor_interface`
/// rather than `gen_registration_interface`.
fn visitor_trait_def() -> TypeDef {
    TypeDef {
        name: "SampleVisitor".to_string(),
        rust_path: "sample_core::SampleVisitor".to_string(),
        is_trait: true,
        methods: vec![MethodDef {
            name: "visit_node".to_string(),
            return_type: TypeRef::Named("VisitResult".to_string()),
            params: vec![ParamDef {
                name: "context".to_string(),
                ty: TypeRef::Named("NodeContext".to_string()),
                ..Default::default()
            }],
            has_default_impl: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn visitor_bridge_config() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "SampleVisitor".to_string(),
        type_alias: Some("SampleVisitorHandle".to_string()),
        context_type: Some("NodeContext".to_string()),
        result_type: Some("VisitResult".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    }
}

#[test]
fn visitor_interface_file_basename_matches_its_own_declared_class_name() {
    let api = ApiSurface {
        crate_name: "sample-core".to_string(),
        version: "1.0.0".to_string(),
        types: vec![visitor_trait_def()],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-core".to_string(),
        trait_bridges: vec![visitor_bridge_config()],
        ..ResolvedCrateConfig::default()
    };

    let files = generate_bindings(&api, &config).expect("generate_bindings ok");
    let interface_file = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|ext| ext == "php") && f.content.contains("interface "))
        .expect("a visitor bridge must emit one PHP interface file");

    let declared_class = interface_file
        .content
        .lines()
        .find_map(|line| line.strip_prefix("interface ").map(str::trim))
        .expect("interface content must declare a class via `interface <Name>`");

    let basename = interface_file
        .path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("interface file must have a UTF-8 basename");

    assert_eq!(
        basename,
        declared_class,
        "PSR-4 requires the file basename to match the declared class name -- file was {}, but \
         its content declares `interface {declared_class}`",
        interface_file.path.display()
    );
    assert_eq!(
        declared_class, "SampleVisitorInterface",
        "a visitor bridge's interface class must carry the `Interface` suffix, got: {declared_class}"
    );
}
