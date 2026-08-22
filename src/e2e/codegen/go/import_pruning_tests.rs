//! Go rejects an unused import outright, so every `import (...)` entry the e2e generator
//! emits has to be justified by the body it actually rendered — not by a fixture-level
//! guess about what the body might contain.

use super::test_file::{GoTestFileContext, render_test_file};
use crate::core::config::e2e::ArgMapping;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn e2e_config_for_describe() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "describe".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            result_is_simple: true,
            args: vec![ArgMapping {
                name: "content".to_string(),
                field: "input.data".to_string(),
                arg_type: "string".to_string(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    }
}

fn render_fixture(e2e_config: &E2eConfig, assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        id: "describe_plain".to_string(),
        description: "Describe a value".to_string(),
        input: serde_json::json!({"data": "hello"}),
        assertions,
        ..Fixture::default()
    };

    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();

    render_test_file(
        "describe",
        &[&fixture],
        GoTestFileContext {
            go_module_path: "github.com/example/mylib",
            import_alias: "sample_crate",
            e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &errors,
        },
    )
}

fn render_equals_fixture(value: serde_json::Value) -> String {
    render_fixture(
        &e2e_config_for_describe(),
        vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("result".to_string()),
            value: Some(value),
            ..Default::default()
        }],
    )
}

/// A string-valued `equals` satisfies the fixture-level heuristic that a body *might* want
/// `strings`, but Go renders it as a plain equality with no `strings.` call anywhere. The
/// heuristic used to be OR-ed with the rendered body rather than narrowed by it, so the
/// import was emitted regardless and every such file failed to build with
/// `"strings" imported and not used`.
#[test]
fn string_equals_does_not_emit_an_unused_strings_import() {
    let out = render_equals_fixture(serde_json::Value::String("hello".to_string()));

    // Precondition, not incidental: if Go ever starts rendering a string `equals` through
    // `strings.` this fixture stops exercising the unused-import path and the assertion below
    // would pass for the wrong reason. Fail loudly here instead of going quietly vacuous.
    assert!(
        !out.contains("strings."),
        "fixture no longer exercises the unused-import path — body now uses `strings.`; got:\n{out}"
    );
    assert!(
        !out.contains("\t\"strings\""),
        "emitted a `strings` import the body never uses; got:\n{out}"
    );
}

// The positive direction — an import IS emitted when the body references the package — is
// already covered by `tests::test_result_is_simple_contains_binds_result_and_emits_imports`,
// which asserts the `strings` import for a `contains` assertion that renders `strings.Contains`.

/// A declared `error` assertion value renders through `strings.Contains(err.Error(), ...)`
/// (`test_function.rs::emit_declared_error_value_assertion`) regardless of assertion kind, but
/// the fixture-level `needs_strings` heuristic only ever inspected `equals`/`contains`-family
/// assertion types — never `error`. Combining that heuristic with `body.contains("strings.")`
/// via `&&` meant a real `strings.` reference in the body was silently dropped from the import list,
/// producing a Go file that fails to compile (`undefined: strings`), exactly as observed in
/// a consumer's `error_test.go:210:7: undefined: strings`.
#[test]
fn declared_error_value_assertion_emits_strings_import() {
    let out = render_fixture(
        &e2e_config_for_describe(),
        vec![Assertion {
            assertion_type: "error".to_string(),
            value: Some(serde_json::Value::String("SomeExpectedError".to_string())),
            ..Default::default()
        }],
    );

    assert!(
        out.contains("strings.Contains(err.Error()"),
        "expected declared error value to render a strings.Contains check; got:\n{out}"
    );
    assert!(
        out.contains("\t\"strings\""),
        "body references `strings.` but the import was not emitted; got:\n{out}"
    );
}
