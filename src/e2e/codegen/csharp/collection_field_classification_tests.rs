//! Regression coverage for the C# e2e generator's collection-field classification.
//!
//! `render_test_method` decides whether an `is_empty`/`not_empty` assertion needs to serialize
//! a field before comparing it (`field_needs_json_serialize`) from `FieldResolver::is_array`/
//! `is_collection_root` — both purely `fields_array`/`fields_optional` config-derived. A
//! collection field with NO per-element path declared anywhere in the fixture suite (nothing
//! ever indexes into it — e.g. a recursive `List<DataNode> Children` field) has no config
//! signal at all, so `field_needs_json_serialize` stayed `false` and `is_empty` fell through to
//! `Assert.True(string.IsNullOrEmpty(Children.ToString()))`: `List<T>.ToString()` returns the
//! type name, a non-empty string, so the assertion could never pass.
//!
//! `csharp.rs` now wires the same IR-derived collection classification the C generator's
//! `fields_c_types` enum inference already pioneered (`FieldResolver::ir_collection_fields` +
//! `with_ir_collection_map`, anchored at the call's declared Rust return type) so a field
//! renders as a collection whenever the IR says so, config or not. These tests drive the real
//! entry point, `render_test_method`, with no `fields_array`/`fields_optional` config at
//! all — the classification must come from the IR alone, mirroring
//! `enum_field_classification_tests.rs` exactly. ~keep

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::HashMap;

fn children_field(ty: TypeRef) -> FieldDef {
    FieldDef {
        name: "children".to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn is_empty_children_assertion() -> Assertion {
    Assertion {
        assertion_type: "is_empty".to_string(),
        field: Some("children".to_string()),
        ..Assertion::default()
    }
}

fn fixture_calling(call: &str) -> Fixture {
    Fixture {
        id: "children_smoke".to_string(),
        description: "Children field smoke".to_string(),
        call: Some(call.to_string()),
        assertions: vec![is_empty_children_assertion()],
        ..Fixture::default()
    }
}

/// Render `fixture` through the real `render_test_method` entry point with `type_defs`/
/// `functions` as the only source of collection knowledge — no `fields_array`/
/// `fields_optional` config, matching a consumer `alef.toml` that never declared either.
#[allow(clippy::too_many_arguments)]
fn render(fixture: &Fixture, e2e_config: &E2eConfig, type_defs: &[TypeDef], functions: &[FunctionDef]) -> String {
    let field_resolver = FieldResolver::new(
        &HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    let mut visitor_class_decls: Vec<String> = Vec::new();
    super::render_test_method(
        &mut out,
        &mut visitor_class_decls,
        fixture,
        "Sample",
        "Process",
        "SampleException",
        "result",
        &[],
        &field_resolver,
        false,
        false,
        e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &config,
        type_defs,
        &[],
        functions,
        &[],
    );
    out
}

fn table_ir() -> (Vec<TypeDef>, Vec<FunctionDef>) {
    let type_defs = vec![TypeDef {
        name: "ProcessResult".to_string(),
        fields: vec![children_field(TypeRef::Vec(Box::new(TypeRef::Named(
            "DataNode".to_string(),
        ))))],
        ..TypeDef::default()
    }];
    let functions = vec![FunctionDef {
        name: "process".to_string(),
        return_type: TypeRef::Named("ProcessResult".to_string()),
        ..FunctionDef::default()
    }];
    (type_defs, functions)
}

fn e2e_config_for(call: &str, function: &str) -> E2eConfig {
    let call_config = CallConfig {
        function: function.to_string(),
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert(call.to_string(), call_config);
    e2e_config
}

/// Regression: `is_empty` on an undeclared `List<DataNode> Children` field must render
/// `Assert.Empty`, never the `ToString()`-based check that can never pass on a non-string
/// collection.
#[test]
fn is_empty_on_an_undeclared_collection_field_is_classified_via_the_ir() {
    let (type_defs, functions) = table_ir();
    let e2e_config = e2e_config_for("process", "process");
    let fixture = fixture_calling("process");
    let out = render(&fixture, &e2e_config, &type_defs, &functions);
    assert!(
        out.contains("Assert.Empty("),
        "expected Assert.Empty for an IR-proven collection field with zero config, got:\n{out}"
    );
    assert!(
        !out.contains("ToString()"),
        "an undeclared collection field must not fall back to the ToString()-based empty check, got:\n{out}"
    );
}

/// A plain `String` field with the same name on an unrelated type must not be misclassified as
/// a collection — the IR classification is anchored per-call, not matched on the leaf name
/// alone.
#[test]
fn a_same_named_string_field_on_an_unrelated_type_is_not_misclassified_as_a_collection() {
    let type_defs = vec![TypeDef {
        name: "OtherResult".to_string(),
        fields: vec![children_field(TypeRef::String)],
        ..TypeDef::default()
    }];
    let functions = vec![FunctionDef {
        name: "other".to_string(),
        return_type: TypeRef::Named("OtherResult".to_string()),
        ..FunctionDef::default()
    }];
    let e2e_config = e2e_config_for("other", "other");
    let fixture = fixture_calling("other");
    let out = render(&fixture, &e2e_config, &type_defs, &functions);
    assert!(
        out.contains("ToString()"),
        "a plain string field's is_empty must keep using the string-based check, got:\n{out}"
    );
}
