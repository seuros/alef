use super::test_case::{DartTestCaseContext, render_test_case};
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

fn render_mock_url(input: serde_json::Value) -> String {
    let fixture = Fixture {
        id: "nested_mock_url".to_string(),
        description: "Nested mock URL".to_string(),
        input,
        preserve_input_urls: true,
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "fetch".to_string();
    e2e_config.call.args = serde_json::from_value(serde_json::json!([{
        "name": "endpoint",
        "field": "input.request.url",
        "type": "mock_url"
    }]))
    .expect("argument mapping");

    let config = ResolvedCrateConfig::default();
    let type_defs = Vec::new();
    let enums = Vec::new();
    let functions = Vec::new();
    let first_class_map = super::values::build_dart_first_class_map(&type_defs, &enums, &e2e_config);
    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        DartTestCaseContext {
            e2e_config: &e2e_config,
            lang: "dart",
            bridge_class: &config.dart_bridge_class_name(),
            dart_first_class_map: &first_class_map,
            adapters: &config.adapters,
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            functions: &functions,
            errors: &[],
            native_typed_dtos: true,
            is_snippet: false,
        },
    );
    out
}

#[test]
fn nested_mock_url_uses_the_configured_field_path() {
    let out = render_mock_url(serde_json::json!({
        "request": { "url": "https://example.test/nested" }
    }));

    assert!(
        out.contains("final endpoint = 'https://example.test/nested';"),
        "the nested fixture value must be preserved instead of falling back: {out}"
    );
    assert!(
        !out.contains("final endpoint = _fixtureUrl("),
        "a present nested value must not use the mock-server fallback: {out}"
    );
}

#[test]
fn missing_nested_mock_url_keeps_the_fixture_url_fallback() {
    let out = render_mock_url(serde_json::json!({ "request": {} }));

    assert!(
        out.contains("final endpoint = _fixtureUrl(\"nested_mock_url\");"),
        "a missing configured path must retain the existing fallback: {out}"
    );
}
