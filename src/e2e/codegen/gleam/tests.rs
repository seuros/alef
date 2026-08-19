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
    let Ok((_setup, args_str)) = build_args_and_setup(
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
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        &[],
        &[],
    ) else {
        panic!("expected Ok result from build_args_and_setup");
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
fn build_args_without_json_object_wrapper_returns_a_skip_reason() {
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
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        &[],
        &[],
    );
    let reason = result
        .expect_err("json_object without recipe/wrapper/from_json must skip")
        .to_string();
    assert!(
        reason.contains("json_object arg `config`"),
        "the skip reason must name the offending arg; got: {reason}"
    );
}

/// The seam's payoff for Gleam. `arg_type` defaults to `"string"`, so a fixture value for a
/// record-typed parameter used to emit a quoted JSON literal that `gleam build` rejects. With
/// a resolved signature the generator refuses instead, and the refusal names the declared
/// type so the operator can see why. ~keep
#[test]
fn a_string_arg_filling_an_ir_record_parameter_is_skipped_with_the_declared_type_named() {
    use crate::core::ir::{ParamDef, TypeDef, TypeRef};
    use crate::e2e::config::ArgMapping;
    let arg = ArgMapping {
        name: "request".to_string(),
        field: "request".to_string(),
        arg_type: "string".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let params = [ParamDef {
        name: "request".to_string(),
        ty: TypeRef::Named("CompletionRequest".to_string()),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "CompletionRequest".to_string(),
        ..TypeDef::default()
    }];
    let input = serde_json::json!({ "request": { "prompt": "hi" } });
    let reason = build_args_and_setup(
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
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
        &type_defs,
        &[],
    )
    .expect_err("a record-typed parameter cannot be filled from a bare Gleam literal");
    assert!(
        reason.contains("CompletionRequest") && reason.contains("`request`"),
        "the refusal must name the declared type and the arg; got: {reason}"
    );
}

/// The other half of the three-state trade: an IR-less caller must lower exactly as it did
/// before the seam. Same arg, same fixture value, `IrAbsent` instead of `Known` -- the
/// generator still emits the quoted literal rather than skipping. Without this a `Known`-only
/// test would let every IR-less consumer regress in silence. ~keep
#[test]
fn the_same_string_arg_still_lowers_to_a_literal_when_the_ir_is_absent() {
    use crate::e2e::config::ArgMapping;
    let arg = ArgMapping {
        name: "request".to_string(),
        field: "request".to_string(),
        arg_type: "string".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let input = serde_json::json!({ "request": "hi" });
    let (_setup, args_str) = build_args_and_setup(
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
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        &[],
        &[],
    )
    .expect("an IR-less caller must keep rendering exactly as before");
    assert_eq!(args_str, "\"hi\"");
}

/// An optional parameter the fixture leaves unset lowers to `option.None`, which is well-typed
/// against `Option<Record>` whatever the record is. Refusing there would skip tests that
/// compile today -- the refusal is about a *value being lowered*, not about the declared type
/// existing. ~keep
#[test]
fn an_unset_optional_record_parameter_is_not_a_refusal() {
    use crate::core::ir::{ParamDef, TypeDef, TypeRef};
    use crate::e2e::config::ArgMapping;
    let arg = ArgMapping {
        name: "request".to_string(),
        field: "request".to_string(),
        arg_type: "string".to_string(),
        optional: true,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let params = [ParamDef {
        name: "request".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Named("CompletionRequest".to_string()))),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "CompletionRequest".to_string(),
        ..TypeDef::default()
    }];
    let input = serde_json::json!({});
    let (_setup, args_str) = build_args_and_setup(
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
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
        &type_defs,
        &[],
    )
    .expect("an unset optional parameter renders option.None and must not skip");
    assert_eq!(args_str, "option.None");
}

/// A named type the IR does not know is not a refusal: it may be a newtype the binding
/// flattens to a plain string, and skipping on it would drop tests that compile today. ~keep
#[test]
fn a_declared_type_absent_from_both_ir_registries_does_not_skip() {
    use crate::core::ir::{ParamDef, TypeRef};
    use crate::e2e::config::ArgMapping;
    let arg = ArgMapping {
        name: "request".to_string(),
        field: "request".to_string(),
        arg_type: "string".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let params = [ParamDef {
        name: "request".to_string(),
        ty: TypeRef::Named("PromptText".to_string()),
        ..ParamDef::default()
    }];
    let input = serde_json::json!({ "request": "hi" });
    let (_setup, args_str) = build_args_and_setup(
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
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
        &[],
        &[],
    )
    .expect("an IR-unknown named type must not be treated as unrepresentable");
    assert_eq!(args_str, "\"hi\"");
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
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
        &[],
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
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
        &[],
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

/// Gleam's error path emits `should.be_error()` and returns, so every other assertion on the
/// fixture used to leave no trace in the generated module at all. The marker is emitted by
/// `test_file.rs`, so this drives the file-level renderer rather than `render_test_case`.
fn render_gleam_error_file_with_declared_value(
    extra: Vec<Assertion>,
    declared_value: Option<&str>,
    errors: &[crate::core::ir::ErrorDef],
) -> String {
    let mut assertions = vec![Assertion {
        assertion_type: "error".to_string(),
        value: declared_value.map(|v| serde_json::Value::String(v.to_string())),
        ..Default::default()
    }];
    assertions.extend(extra);
    let fixture = Fixture {
        id: "rate_limited".to_string(),
        description: "Rejects the request".to_string(),
        assertions,
        ..Fixture::default()
    };
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "process".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let _ = crate::e2e::codegen::take_skip_records();
    super::test_file::render_test_file(
        "error",
        &[&fixture],
        &e2e_config,
        "sample_crate",
        "process",
        "result",
        &[],
        &[],
        None,
        // This fixture exercises the error path, not argument lowering, so it deliberately
        // supplies no IR: `CallIr::default()` is the IrAbsent state, under which the arg
        // builder must behave exactly as it did before the seam existed. ~keep
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
        errors,
    )
}

fn render_gleam_error_file(extra: Vec<Assertion>) -> String {
    render_gleam_error_file_with_declared_value(extra, None, &[])
}

#[test]
fn gleam_equals_on_an_error_field_is_named_instead_of_dropped() {
    let out = render_gleam_error_file(vec![Assertion {
        assertion_type: "equals".to_string(),
        field: Some("error.status_code".to_string()),
        ..Default::default()
    }]);

    // Positive first: the error block really rendered.
    assert!(
        out.contains("|> should.be_error()"),
        "the error block must render:\n{out}"
    );
    assert!(
        out.contains(
            "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
        ),
        "got:\n{out}"
    );

    let records = crate::e2e::codegen::take_skip_records();
    assert_eq!(records.len(), 1, "got: {records:?}");
    assert_eq!(records[0].language, "gleam");
    assert_eq!(records[0].field, "equals");
}

/// Negative control: a lone `error` assertion must leave the generated module marker-free.
#[test]
fn gleam_a_lone_error_assertion_renders_no_marker() {
    let out = render_gleam_error_file(Vec::new());

    assert!(
        out.contains("|> should.be_error()"),
        "the error block must render:\n{out}"
    );
    assert!(!out.contains("has no accessor for error field"), "got:\n{out}");
}

fn coded_authentication_variant() -> Vec<crate::core::ir::ErrorDef> {
    vec![crate::core::ir::ErrorDef {
        name: "ApiError".to_string(),
        rust_path: "lib::ApiError".to_string(),
        original_rust_path: String::new(),
        variants: vec![crate::core::ir::ErrorVariant {
            name: "Authentication".to_string(),
            error_code: Some(100),
            is_unit: true,
            ..crate::core::ir::ErrorVariant::default()
        }],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }]
}

/// A message-style declared value (not a known variant name) keeps rendering the existing
/// `string.inspect`/`string.contains` comparison unchanged — proves the fix does not regress
/// config-validation fixtures.
#[test]
fn gleam_message_style_declared_value_still_asserts() {
    let errors = coded_authentication_variant();
    let out = render_gleam_error_file_with_declared_value(Vec::new(), Some("size"), &errors);

    assert!(
        out.contains("should.be_true(string.contains(string.inspect(__reason), \"size\"))"),
        "got:\n{out}"
    );
    assert!(out.contains("import gleam/string"), "got:\n{out}");
}

/// The defect this fix closes: a declared value that names a real `ErrorVariant` — every Gleam
/// binding rides on the same Rustler NIF glue that stringifies the reason, so no constructor
/// name ever survives to `string.inspect` — must render the registered skip, not a comparison
/// that can never pass. The `import gleam/string` line must also disappear: nothing in the
/// `Unsubstantiable` rendering path needs it, and importing an unused module fails `gleam build`.
#[test]
fn gleam_skips_a_known_variant_it_cannot_substantiate() {
    let errors = coded_authentication_variant();
    let out = render_gleam_error_file_with_declared_value(Vec::new(), Some("Authentication"), &errors);

    assert!(
        out.contains("let assert Error(_) = __result"),
        "the call must still be proven to fail, got:\n{out}"
    );
    assert!(
        out.contains(
            "// skipped: declared error variant 'Authentication' not yet preserved as a distinct identity by \
             this backend's generator"
        ),
        "got:\n{out}"
    );
    assert!(
        !out.contains("string.contains(string.inspect"),
        "must not render a comparison that can never pass, got:\n{out}"
    );
    assert!(
        !out.contains("import gleam/string"),
        "the Unsubstantiable path needs no gleam/string import, got:\n{out}"
    );

    let records = crate::e2e::codegen::take_skip_records();
    assert_eq!(records.len(), 1, "got: {records:?}");
    assert_eq!(records[0].language, "gleam");
    assert_eq!(records[0].fixture_id, "rate_limited");
}
