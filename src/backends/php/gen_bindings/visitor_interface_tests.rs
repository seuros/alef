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
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, TypeDef, TypeRef};

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

/// The result enum a visitor callback returns: one `#[default]` unit variant, a second unit
/// variant, and a string-payload variant -- the three shapes
/// `codegen::visitor_result::visitor_result_metadata` distinguishes.
fn visit_result_enum() -> EnumDef {
    EnumDef {
        name: "VisitResult".to_string(),
        rust_path: "sample_core::VisitResult".to_string(),
        variants: vec![
            EnumVariant {
                name: "Proceed".to_string(),
                is_default: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Halt".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Replace".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn context_type_def(doc: &str) -> TypeDef {
    TypeDef {
        name: "NodeContext".to_string(),
        rust_path: "sample_core::NodeContext".to_string(),
        doc: doc.to_string(),
        ..Default::default()
    }
}

/// Render the visitor interface through the real `generate_bindings` entry point, so the
/// docblock under assertion is the one a consumer's `alef build` actually writes.
fn rendered_visitor_interface(context_type: Option<TypeDef>, enums: Vec<EnumDef>) -> String {
    let mut types = vec![visitor_trait_def()];
    types.extend(context_type);
    let api = ApiSurface {
        crate_name: "sample-core".to_string(),
        version: "1.0.0".to_string(),
        types,
        enums,
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-core".to_string(),
        trait_bridges: vec![visitor_bridge_config()],
        ..ResolvedCrateConfig::default()
    };

    generate_bindings(&api, &config)
        .expect("generate_bindings ok")
        .into_iter()
        .find(|f| {
            f.path
                .file_name()
                .is_some_and(|name| name == "SampleVisitorInterface.php")
        })
        .expect("a visitor bridge must emit its PHP interface file")
        .content
}

/// The `@param` line must say what the context carries, sourced from the context type's own
/// rustdoc. This detail was lost when the docblock's hardcoded type names became template
/// variables: the prose that mentioned them was replaced with a restatement of the parameter's
/// own name rather than re-derived from the IR, so the generated interface stopped documenting
/// the argument at all. ~keep
#[test]
fn visitor_docblock_describes_the_context_parameter_from_its_own_type_doc() {
    let content = rendered_visitor_interface(
        Some(context_type_def("Position, depth and path of the item being visited.")),
        vec![visit_result_enum()],
    );

    assert!(
        content.contains("@param NodeContext $context Position, depth and path of the item being visited."),
        "the context @param must carry the context type's own documented summary, got:\n{content}"
    );
}

/// The `@return` line must name the values the callback may actually return, derived from the
/// configured result enum rather than from prose naming one crate's variants.
#[test]
fn visitor_docblock_lists_the_result_values_the_callback_may_return() {
    let content = rendered_visitor_interface(Some(context_type_def("Context.")), vec![visit_result_enum()]);

    assert!(
        content.contains("@return VisitResult How to proceed with traversal (Proceed, Halt, or Replace)"),
        "the return @return must enumerate the result enum's variants, got:\n{content}"
    );
}

/// The interface docblock must state what the default implementations return, spelled from the
/// configured result type and its `#[default]` variant.
#[test]
fn visitor_interface_docblock_states_what_the_default_implementations_return() {
    let content = rendered_visitor_interface(Some(context_type_def("Context.")), vec![visit_result_enum()]);

    assert!(
        content.contains("All methods have default no-op implementations that return `VisitResult::Proceed`."),
        "the interface docblock must state the default implementations' result, got:\n{content}"
    );
}

/// Control for both derivations above: with no documented context type and no result enum in the
/// API surface, the docblock must fall back to neutral wording and invent nothing. A generator
/// that describes a parameter it has no information about is worse than one that says little. ~keep
#[test]
fn undocumented_context_and_unresolvable_result_gain_no_invented_prose() {
    let content = rendered_visitor_interface(Some(context_type_def("")), Vec::new());

    assert!(
        content.contains("@param NodeContext $context Visitor context information\n"),
        "an undocumented context type must keep the neutral description, got:\n{content}"
    );
    assert!(
        content.contains("@return VisitResult How to proceed with traversal\n"),
        "an unresolvable result type must get no variant list, got:\n{content}"
    );
    assert!(
        !content.contains("All methods have default no-op implementations"),
        "the default-implementation sentence must be omitted when the result enum cannot be \
         resolved, got:\n{content}"
    );
}
