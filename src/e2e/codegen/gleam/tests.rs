use super::args::build_args_and_setup;
use super::constructors::render_gleam_element_constructor;
use super::test_case::render_test_case;
use crate::core::config::{GleamElementConstructor, GleamElementField};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn file_job_recipe() -> GleamElementConstructor {
    GleamElementConstructor {
        element_type: "FileJob".to_string(),
        constructor: "sample_crate.FileJob".to_string(),
        fields: vec![
            GleamElementField {
                gleam_field: "path".to_string(),
                kind: "file_path".to_string(),
                json_field: Some("path".to_string()),
                default: None,
                value: None,
            },
            GleamElementField {
                gleam_field: "config".to_string(),
                kind: "literal".to_string(),
                json_field: None,
                default: None,
                value: Some("option.None".to_string()),
            },
        ],
    }
}

#[test]
fn render_element_constructor_file_path_relative_path_gets_test_documents_prefix() {
    let item = serde_json::json!({ "path": "docx/fake.docx" });
    let out = render_gleam_element_constructor(&item, &file_job_recipe(), "../../test_documents");
    assert_eq!(
        out,
        "sample_crate.FileJob(path: \"../../test_documents/docx/fake.docx\", config: option.None)"
    );
}

#[test]
fn render_element_constructor_file_path_absolute_path_passes_through() {
    let item = serde_json::json!({ "path": "/etc/some/absolute" });
    let out = render_gleam_element_constructor(&item, &file_job_recipe(), "../../test_documents");
    assert!(
        out.contains("\"/etc/some/absolute\""),
        "absolute paths must NOT receive the test_documents prefix; got:\n{out}"
    );
}

#[test]
fn render_element_constructor_byte_array_emits_bitarray() {
    let recipe = GleamElementConstructor {
        element_type: "BytesJob".to_string(),
        constructor: "sample_crate.BytesJob".to_string(),
        fields: vec![
            GleamElementField {
                gleam_field: "content".to_string(),
                kind: "byte_array".to_string(),
                json_field: Some("content".to_string()),
                default: None,
                value: None,
            },
            GleamElementField {
                gleam_field: "mime_type".to_string(),
                kind: "string".to_string(),
                json_field: Some("mime_type".to_string()),
                default: Some("text/plain".to_string()),
                value: None,
            },
            GleamElementField {
                gleam_field: "config".to_string(),
                kind: "literal".to_string(),
                json_field: None,
                default: None,
                value: Some("option.None".to_string()),
            },
        ],
    };
    let item = serde_json::json!({ "content": [72, 105], "mime_type": "text/html" });
    let out = render_gleam_element_constructor(&item, &recipe, "../../test_documents");
    assert_eq!(
        out,
        "sample_crate.BytesJob(content: <<72, 105>>, mime_type: \"text/html\", config: option.None)"
    );
}

#[test]
fn build_args_with_json_object_wrapper_substitutes_placeholder() {
    use crate::e2e::config::ArgMapping;
    let arg = ArgMapping {
        name: "config".to_string(),
        field: "config".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let input = serde_json::json!({
        "config": { "use_cache": true, "force_ocr": false }
    });
    let Some((_setup, args_str)) = build_args_and_setup(
        &input,
        &[arg],
        "test_fixture",
        "../../test_documents",
        &[],
        Some("k.config_from_json_string({json})"),
        "sample_crate",
        &[],
        None,
        "default",
        false,
    ) else {
        panic!("expected Some result from build_args_and_setup");
    };
    assert!(
        args_str.starts_with("k.config_from_json_string("),
        "wrapper must envelop the JSON literal; got:\n{args_str}"
    );
    assert!(
        args_str.contains("use_cache"),
        "JSON payload must reach the wrapper; got:\n{args_str}"
    );
}

#[test]
fn build_args_without_json_object_wrapper_returns_none_for_skip() {
    use crate::e2e::config::ArgMapping;
    let arg = ArgMapping {
        name: "config".to_string(),
        field: "config".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let input = serde_json::json!({ "config": { "x": 1 } });
    let result = build_args_and_setup(
        &input,
        &[arg],
        "test_fixture",
        "../../test_documents",
        &[],
        None,
        "sample_crate",
        &[],
        None,
        "default",
        false,
    );
    assert!(
        result.is_none(),
        "json_object without recipe/wrapper/from_json must return None for skip; got: {result:?}"
    );
}

#[test]
fn render_element_constructor_string_falls_back_to_default() {
    let recipe = GleamElementConstructor {
        element_type: "BytesJob".to_string(),
        constructor: "k.BytesJob".to_string(),
        fields: vec![GleamElementField {
            gleam_field: "mime_type".to_string(),
            kind: "string".to_string(),
            json_field: Some("mime_type".to_string()),
            default: Some("text/plain".to_string()),
            value: None,
        }],
    };
    let item = serde_json::json!({});
    let out = render_gleam_element_constructor(&item, &recipe, "../../test_documents");
    assert!(
        out.contains("mime_type: \"text/plain\""),
        "missing string field must fall back to default; got:\n{out}"
    );
}

fn field_gated_e2e_config() -> E2eConfig {
    E2eConfig {
        result_fields: std::collections::HashSet::from(["content".to_string()]),
        call: CallConfig {
            function: "process".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    }
}

/// `render_test_case` writes into a `String` buffer shared across every fixture in
/// the generated file (see the loop in `test_file.rs`), so this proves the
/// offset-scoped scan wired into `render_test_case` attributes a skip marker to the
/// fixture that actually produced it, not to a fixture that merely shares the buffer.
#[test]
fn dropped_field_assertion_carries_the_marker_and_is_correctly_attributed_per_fixture() {
    let e2e_config = field_gated_e2e_config();
    let mut out = String::new();

    let clean_fixture = Fixture {
        id: "clean_smoke".to_string(),
        description: "clean smoke".to_string(),
        ..Fixture::default()
    };
    render_test_case(
        &mut out,
        &clean_fixture,
        &e2e_config,
        "sample_crate",
        "process",
        "result",
        &[],
        &[],
        None,
    );
    let clean_len = out.len();

    let mut dirty_fixture = Fixture {
        id: "dirty_smoke".to_string(),
        description: "dirty smoke".to_string(),
        ..Fixture::default()
    };
    dirty_fixture.assertions = vec![Assertion {
        assertion_type: "equals".to_string(),
        field: Some("nonexistent_field".to_string()),
        value: Some(serde_json::json!("x")),
        ..Default::default()
    }];
    render_test_case(
        &mut out,
        &dirty_fixture,
        &e2e_config,
        "sample_crate",
        "process",
        "result",
        &[],
        &[],
        None,
    );

    assert!(
        !out[..clean_len].contains("not available"),
        "the first fixture's own render must carry no skip marker, got:\n{}",
        &out[..clean_len]
    );
    assert!(
        out[clean_len..].contains("field 'nonexistent_field' not available on result type"),
        "the second fixture's own render must carry the skip marker, got:\n{}",
        &out[clean_len..]
    );
}
