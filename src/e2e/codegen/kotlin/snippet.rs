use heck::{ToLowerCamelCase, ToUpperCamelCase};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::TypeDef;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureEnv};

use super::args::{KotlinArgsContext, build_args_and_setup};

/// `build_args_and_setup` is shared with the e2e test emitter, whose generated test
/// class declares a companion-object `MAPPER` constant that its output references.
/// A docs snippet is a standalone `fun main()` with no such companion object, and no
/// binding exposes a public `MAPPER`, so every reference has to be rebound onto the
/// local `mapper` the snippet template declares for itself. This applies to the
/// argument list as much as to the setup lines: a typed-array argument is emitted
/// inline as `listOf(MAPPER.readValue(...), ...)` and never reaches `setup_lines`.
const TEST_CLASS_MAPPER_REFERENCE: &str = "MAPPER.";
const SNIPPET_MAPPER_REFERENCE: &str = "mapper.";

fn rebind_mapper_references(source: &str) -> String {
    source.replace(TEST_CLASS_MAPPER_REFERENCE, SNIPPET_MAPPER_REFERENCE)
}

pub(crate) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    kotlin_android_style: bool,
) -> String {
    let lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call, type_defs);
    let overrides = recipe.override_config;
    let class_name = overrides
        .and_then(|value| value.class.as_deref())
        .unwrap_or(&config.name)
        .rsplit('.')
        .next()
        .unwrap_or(&config.name)
        .to_upper_camel_case();
    let function_name = overrides
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_lower_camel_case();
    let options_type = recipe
        .options_type
        .or_else(|| recipe.compatible_options_type(&["kotlin", "kotlin_android", "java", "csharp"]));

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on
    // the owner handle (`engine.streamItems(req)`), not as static facade
    // methods — mirrors the test-generation path in test_method.rs and the
    // Java e2e backend. The docs snippet must show the same real call shape
    // callers will actually write, so it is wrong here for the same reason it
    // was wrong in the generated tests.
    let adapter = config.adapters.iter().find(|a| a.name == call.function.as_str());
    let is_streaming_owner_adapter = adapter.is_some_and(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming) && a.owner_type.is_some()
    });
    let streaming_owner_handle: Option<String> = if is_streaming_owner_adapter {
        recipe
            .args
            .iter()
            .find(|a| a.arg_type == "handle")
            .map(|a| a.name.clone())
    } else {
        None
    };

    let (setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        recipe.args,
        KotlinArgsContext {
            fixture,
            class_name: &class_name,
            options_type,
            fixture_id: &fixture.id,
            kotlin_android_style,
            config,
            type_defs,
            owner_handle_is_receiver: streaming_owner_handle.is_some(),
        },
    );
    let mut setup_lines = setup_lines
        .into_iter()
        .map(|line| rebind_mapper_references(&line))
        .collect::<Vec<_>>();
    if let Some(visitor) = &fixture.visitor
        && let Some(visitor_args) = super::visitor::attach_visitor(&mut setup_lines, &args, visitor, config, type_defs)
    {
        args = visitor_args;
    }
    if !recipe.extra_args.is_empty() {
        args = if args.is_empty() {
            recipe.extra_args.join(", ")
        } else {
            format!("{args}, {}", recipe.extra_args.join(", "))
        };
    }
    args = rebind_mapper_references(&args);
    let client_factory = overrides.and_then(|value| value.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|value| value.client_factory.as_deref())
    });
    let needs_mapper = args.contains(SNIPPET_MAPPER_REFERENCE)
        || setup_lines.iter().any(|line| line.contains(SNIPPET_MAPPER_REFERENCE));
    let is_async = client_factory.is_some() || call.r#async;
    let package_name = if kotlin_android_style {
        config
            .kotlin_android
            .as_ref()
            .and_then(|value| value.package.clone())
            .unwrap_or_else(|| config.kotlin_package())
    } else {
        config.kotlin_package()
    };
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let api_key_var = FixtureEnv::api_key_var_or_default(fixture.env.as_ref());

    // The template renders the call as `{{ class_name }}.{{ function_name }}(...)`
    // (or, with `client_factory`, constructs a client via
    // `{{ class_name }}.{{ client_factory }}(...)` first and then calls
    // `client.{{ function_name }}(...)`). For a flat-call streaming owner_type
    // adapter, substitute the owner handle's variable name for `class_name` so
    // the call renders as the real instance-method invocation. `client_factory`
    // fixtures already dispatch on `client`, not `class_name`, so this
    // substitution is scoped to the flat-call path where it actually changes
    // the rendered call target.
    let call_target_class_name = if client_factory.is_none() {
        streaming_owner_handle.unwrap_or(class_name)
    } else {
        class_name
    };

    crate::e2e::template_env::render(
        "kotlin/snippet_body.jinja",
        minijinja::context! {
            package_name => package_name,
            needs_mapper => needs_mapper,
            setup_lines => setup_lines,
            client_factory => client_factory.map(ToLowerCamelCase::to_lower_camel_case),
            class_name => call_target_class_name,
            function_name => function_name,
            args => args,
            result_var => call.result_var,
            returns_void => call.returns_void,
            is_async => is_async,
            fixture_id => fixture.id,
            expects_error => expects_error,
            api_key_var => api_key_var,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride};

    fn fixture() -> Fixture {
        Fixture {
            id: "quick_start".into(),
            description: "Quick start".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        }
    }

    const BYTES_ELEMENT: &str = r#"mapper.readValue("{\"kind\":\"bytes\"}", ExtractInput::class.java)"#;
    const URI_ELEMENT: &str = r#"mapper.readValue("{\"kind\":\"uri\"}", ExtractInput::class.java)"#;

    fn line_containing<'a>(body: &'a str, needle: &str) -> &'a str {
        body.lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no line contains {needle} in:\n{body}"))
            .trim()
    }

    /// A batch call whose typed-array argument is materialised inline in the call
    /// expression rather than in a setup line.
    fn batch_call() -> CallConfig {
        CallConfig {
            function: "extract_batch".into(),
            result_var: "result".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "inputs".into(),
                field: "inputs".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: Some("ExtractInput".into()),
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        }
    }

    fn batch_fixture() -> Fixture {
        Fixture {
            id: "extract_batch_bytes_happy".into(),
            description: "Extract several documents in one batch".into(),
            input: serde_json::json!({"inputs": [{"kind": "bytes"}, {"kind": "uri"}]}),
            ..Fixture::default()
        }
    }

    fn batch_snippet(kotlin_android_style: bool) -> String {
        let config = ResolvedCrateConfig {
            name: "xberg".into(),
            ..ResolvedCrateConfig::default()
        };
        render_snippet_body(
            &batch_fixture(),
            &E2eConfig {
                call: batch_call(),
                ..E2eConfig::default()
            },
            &config,
            &[],
            kotlin_android_style,
        )
    }

    #[test]
    fn android_batch_snippet_binds_list_elements_to_the_locally_declared_mapper() {
        let body = batch_snippet(true);

        assert!(
            !body.contains("MAPPER"),
            "the snippet must not reference the test class's private MAPPER, got:\n{body}"
        );
        assert!(
            body.contains("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper"),
            "{body}"
        );
        assert!(body.contains("    val mapper = jacksonObjectMapper()"), "{body}");
        assert_eq!(
            line_containing(&body, "extractBatch"),
            format!("val result = Xberg.extractBatch(listOf({BYTES_ELEMENT}, {URI_ELEMENT}))")
        );
    }

    #[test]
    fn jvm_batch_snippet_binds_list_elements_to_the_locally_declared_mapper() {
        let body = batch_snippet(false);

        assert!(
            !body.contains("MAPPER"),
            "the snippet must not reference the test class's private MAPPER, got:\n{body}"
        );
        assert!(
            body.contains("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper"),
            "{body}"
        );
        assert!(body.contains("    val mapper = jacksonObjectMapper()"), "{body}");
        assert_eq!(
            line_containing(&body, "extractBatch"),
            format!("val result = Xberg.extractBatch(listOf({BYTES_ELEMENT}, {URI_ELEMENT}))")
        );
    }

    #[test]
    fn snippet_keeps_the_native_call_without_the_test_harness() {
        let mut call = CallConfig {
            function: "load_document".into(),
            result_var: "document".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                class: Some("DocumentApi".into()),
                ..CallOverride::default()
            },
        );
        let body = render_snippet_body(
            &fixture(),
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
            false,
        );

        assert!(body.contains("DocumentApi.loadDocument()"));
        assert!(body.contains("println(document)"), "{body}");
        assert!(body.contains("fun main()"));
        assert!(!body.contains("@Test"));
        assert!(!body.contains("assert"));
    }

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
            true,
        );

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
            true,
        );

        assert!(body.contains("val config = mapper.readValue"), "{body}");
        assert!(body.contains("ProcessConfig::class.java"), "{body}");
        assert!(body.contains("SampleApi.process(\"fn main() {}\", config)"), "{body}");
        assert!(!body.contains("runBlocking"), "{body}");
    }

    #[test]
    fn snippet_renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], false);

        assert!(body.contains("catch (error: Exception)"), "{body}");
        assert!(body.contains("error::class.simpleName"), "{body}");
        assert!(!body.contains("AssertionError"), "{body}");
    }

    #[test]
    fn snippet_reads_nested_typed_dto_files() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "document_input",
            "description": "Read a document",
            "input": {"request": {"content": "ignored"}},
            "assertions": [],
            "docs": {
                "topic": "documents",
                "presentation": {"files": [{"field": "/request/content", "path": "document.pdf"}]}
            }
        }))
        .expect("fixture");
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "request".into(),
                field: "request".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                options_type: Some("DocumentRequest".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture.docs_call_fixture(),
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
            false,
        );

        assert!(
            body.contains("Files.readAllBytes(java.nio.file.Path.of(\"document.pdf\"))"),
            "{body}"
        );
        assert!(body.contains("Base64.getEncoder().encodeToString"), "{body}");
        assert!(body.contains("DocumentRequest::class.java"), "{body}");
    }

    #[test]
    fn snippet_deserializes_generic_typed_dto_without_file_metadata() {
        let fixture = Fixture {
            id: "document_input".into(),
            description: "Process a document".into(),
            input: serde_json::json!({"kind": "uri", "uri": "document.txt"}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "input".into(),
                field: "input".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: Some("DocumentInput".into()),
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin_android".into(),
            CallOverride {
                options_type: Some("ExtractionConfig".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
            true,
        );

        assert!(body.contains("val input = mapper.readValue("), "{body}");
        assert!(body.contains("DocumentInput::class.java"), "{body}");
        assert!(!body.contains("ExtractionConfig::class.java"), "{body}");
        assert!(body.contains(".process(input)"), "{body}");
        assert!(body.contains("jacksonObjectMapper"), "{body}");
    }

    #[test]
    fn snippet_uses_nested_centralized_wire_names() {
        let fixture = Fixture {
            id: "document_input".into(),
            description: "Process a document".into(),
            input: serde_json::json!({"request_id": "one", "details": {"page_count": 2}}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "input".into(),
                field: "input".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin_android".into(),
            CallOverride {
                options_type: Some("DocumentInput".into()),
                ..CallOverride::default()
            },
        );
        let type_defs = vec![
            crate::core::ir::TypeDef {
                name: "DocumentInput".into(),
                fields: vec![
                    crate::core::ir::FieldDef {
                        name: "request_id".into(),
                        serde_rename: Some("request-id".into()),
                        ..Default::default()
                    },
                    crate::core::ir::FieldDef {
                        name: "details".into(),
                        ty: crate::core::ir::TypeRef::Named("DocumentDetails".into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            crate::core::ir::TypeDef {
                name: "DocumentDetails".into(),
                serde_rename_all: Some("camelCase".into()),
                fields: vec![crate::core::ir::FieldDef {
                    name: "page_count".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ];

        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &type_defs,
            true,
        );

        assert!(body.contains(r#"\"request-id\":\"one\""#), "{body}");
        assert!(body.contains(r#"\"pageCount\":2"#), "{body}");
        assert!(body.contains("val mapper = jacksonObjectMapper()"), "{body}");
    }
}
