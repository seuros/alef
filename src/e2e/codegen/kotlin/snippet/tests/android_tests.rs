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
