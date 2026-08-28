//! Docs-snippet rendering in the Kotlin/Android style (sync `main`, simple class names).

use super::test_support::fixture;
use super::*;
use crate::e2e::config::{CallConfig, CallOverride};

#[test]
fn android_snippet_uses_simple_class_name_and_sync_main() {
    let mut call = CallConfig {
        function: "convert".into(),
        result_var: "result".into(),
        ..CallConfig::default()
    };
    call.overrides.insert(
        "kotlin_android".into(),
        CallOverride {
            class: Some("dev.sample.SampleApi".into()),
            ..CallOverride::default()
        },
    );
    let mut config = ResolvedCrateConfig::default();
    config.kotlin_android = Some(crate::core::config::KotlinAndroidConfig {
        package: Some("dev.sample".into()),
        ..Default::default()
    });

    let body = render_snippet_body(
        &fixture(),
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
        &[],
        true,
    )
    .expect("snippet renders");

    assert!(body.contains("import dev.sample.*"), "{body}");
    assert!(body.contains("SampleApi.convert()"), "{body}");
    assert!(body.contains("fun main() {"), "{body}");
    assert!(!body.contains("runBlocking"), "{body}");
    assert!(!body.contains("DevSampleSampleApi"), "{body}");
}

#[test]
fn android_snippet_declares_typed_config_without_coroutine_wrapper() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "process_source",
        "description": "Process source",
        "input": {
            "source_code": "fn main() {}",
            "config": {"language": "rust"}
        }
    }))
    .expect("fixture parses");
    let mut call = CallConfig {
        function: "process".into(),
        result_var: "result".into(),
        args: vec![
            crate::e2e::config::ArgMapping {
                name: "source".into(),
                field: "source_code".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
            crate::e2e::config::ArgMapping {
                name: "config".into(),
                field: "config".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
        ],
        ..CallConfig::default()
    };
    call.overrides.insert(
        "java".into(),
        CallOverride {
            options_type: Some("ProcessConfig".into()),
            ..Default::default()
        },
    );

    let config = ResolvedCrateConfig {
        name: "sample_api".into(),
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
        &[],
        true,
    )
    .expect("snippet renders");

    assert!(body.contains("val config = mapper.readValue"), "{body}");
    assert!(body.contains("ProcessConfig::class.java"), "{body}");
    assert!(body.contains("SampleApi.process(\"fn main() {}\", config)"), "{body}");
    assert!(!body.contains("runBlocking"), "{body}");
}

/// Regression: `ArgMapping::optional` on a fixture's `call.args` describes what the fixture's
/// own `input` JSON may leave out — it is not a claim about what the Kotlin target's generated
/// signature declares. When the core IR says the Rust parameter is not `Option<T>`
/// (`ParamDef::optional == false`), the facade signature generator (`facade_param` in
/// `backends::kotlin_android::gen_bindings::module_facade::facade_types`) grants that parameter
/// no default, so it stays non-nullable and required in the generated Kotlin signature. A doc
/// snippet that splices a bare `null` there fails to compile
/// (`Null can not be a value of a non-null type Int`). Before the fix, `build_args_and_setup`
/// only consulted the fixture's own `optional: true` claim and always emitted `null` for a
/// missing value, regardless of what the target actually required.
#[test]
fn android_snippet_passes_a_typed_default_when_the_target_requires_an_argument_the_fixture_marks_optional() {
    let functions = vec![crate::core::ir::FunctionDef {
        name: "extract_batch".into(),
        params: vec![
            crate::core::ir::ParamDef {
                name: "source".into(),
                ty: crate::core::ir::TypeRef::String,
                ..crate::core::ir::ParamDef::default()
            },
            crate::core::ir::ParamDef {
                name: "retry_count".into(),
                ty: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
                optional: false,
                ..crate::core::ir::ParamDef::default()
            },
        ],
        return_type: crate::core::ir::TypeRef::String,
        ..crate::core::ir::FunctionDef::default()
    }];
    let call = CallConfig {
        function: "extract_batch".into(),
        result_var: "result".into(),
        args: vec![
            crate::e2e::config::ArgMapping {
                name: "source".into(),
                field: "source".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
            crate::e2e::config::ArgMapping {
                name: "retry_count".into(),
                field: "retry_count".into(),
                arg_type: "int".into(),
                optional: true,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
        ],
        ..CallConfig::default()
    };
    let fixture = Fixture {
        id: "extract_batch_basic".into(),
        description: "Extract with a default retry count".into(),
        input: serde_json::json!({"source": "doc.pdf"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample_api".into(),
        ..ResolvedCrateConfig::default()
    };

    let body = render_snippet_body_with_ir(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
        &[],
        true,
        &functions,
    )
    .expect("snippet renders");

    assert!(
        body.contains("SampleApi.extractBatch(\"doc.pdf\", 0)"),
        "the target-required arg must get a typed default, not a bare null: {body}"
    );
    assert!(
        !body.contains("null"),
        "a required Kotlin parameter must never receive a bare null: {body}"
    );
}
