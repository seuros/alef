use heck::{ToLowerCamelCase, ToUpperCamelCase};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::TypeDef;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use super::args::{JavaArgsContext, build_args_and_setup};

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
) -> String {
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("java", fixture, call, type_defs);
    let overrides = recipe.override_config;
    let class_name = overrides
        .and_then(|value| value.class.clone())
        .unwrap_or_else(|| config.name.to_upper_camel_case());
    let function_name = overrides
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_lower_camel_case();
    let options_type = recipe
        .options_type
        .or_else(|| recipe.compatible_options_type(&["kotlin", "csharp", "c", "go", "python"]));
    let mut teardown = String::new();
    let (mut setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        recipe.args,
        JavaArgsContext {
            class_name: &class_name,
            options_type,
            fixture,
            adapter_request_type: None,
            owner_handle_is_receiver: false,
            config,
            type_defs,
            teardown_block: &mut teardown,
        },
    );
    setup_lines.splice(0..0, render_json_object_setup(fixture, recipe.args, options_type));
    if let Some(visitor_spec) = &fixture.visitor
        && let Some(binding) = super::visitor::java_visitor_binding(config, type_defs, Some(visitor_spec), options_type)
    {
        let visitor = super::visitor::build_java_visitor(&mut setup_lines, visitor_spec, &class_name, &binding);
        args = super::visitor::apply_java_visitor_arg(&mut setup_lines, &args, recipe.args, &visitor, &binding);
    }
    if !recipe.extra_args.is_empty() {
        args = if args.is_empty() {
            recipe.extra_args.join(", ")
        } else {
            format!("{args}, {}", recipe.extra_args.join(", "))
        };
    }
    let client_factory = overrides
        .and_then(|value| value.client_factory.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get("java")
                .and_then(|value| value.client_factory.as_deref())
        })
        .map(ToLowerCamelCase::to_lower_camel_case);
    let package_name = overrides
        .and_then(|value| value.module.clone())
        .unwrap_or_else(|| config.java_package());
    let needs_mapper = setup_lines.iter().any(|line| line.contains("MAPPER"));
    let presentation = crate::e2e::codegen::presentation::resolve(fixture, e2e_config, "java");
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");

    crate::e2e::template_env::render(
        "java/snippet_body.jinja",
        minijinja::context! {
            package_name => package_name,
            class_name => class_name,
            setup_lines => setup_lines,
            client_factory => client_factory,
            function_name => function_name,
            args => args,
            result_var => call.result_var,
            returns_void => call.returns_void,
            needs_mapper => needs_mapper,
            fixture_id => fixture.id,
            presentation => presentation,
            expects_error => expects_error,
        },
    )
}

fn render_json_object_setup(
    fixture: &Fixture,
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
) -> Vec<String> {
    args.iter()
        .filter_map(|arg| {
            if arg.arg_type != "json_object" {
                return None;
            }
            let value = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);
            let type_name = crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, value)?;
            if value.is_null() || value.is_array() {
                return None;
            }
            let mut normalized = crate::e2e::codegen::transform_json_keys_for_language(value, "snake_case");
            let files = fixture.docs_files_for_arg(&arg.field);
            let file_reads = files
                .iter()
                .enumerate()
                .filter_map(|(index, file)| {
                    let marker = format!("__ALEF_DOC_FILE_{index}__");
                    let target = if file.field.is_empty() {
                        Some(&mut normalized)
                    } else {
                        normalized.pointer_mut(&file.field)
                    }?;
                    *target = serde_json::Value::String(marker.clone());
                    Some((index, marker, file.path.clone()))
                })
                .collect::<Vec<_>>();
            let json = serde_json::to_string(&normalized).unwrap_or_default();
            Some(
                crate::e2e::template_env::render(
                    "java/snippet_json_object_setup.jinja",
                    minijinja::context! {
                        variable => arg.name,
                        json => crate::e2e::escape::escape_java(&json),
                        type_name => type_name,
                        file_reads => file_reads,
                    },
                )
                .trim_end()
                .to_string(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride};

    #[test]
    fn snippet_keeps_native_call_without_junit_harness() {
        let fixture = Fixture {
            id: "quick_start".into(),
            description: "Quick start".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "load_document".into(),
            result_var: "document".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                class: Some("DocumentApi".into()),
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
        );

        assert!(body.contains("DocumentApi.loadDocument()"));
        assert!(body.contains("public static void main(String[] args) throws Exception"));
        assert!(!body.contains("@Test"));
        assert!(!body.contains("assert"));
        assert!(body.contains("System.out.println(document);"));
    }

    #[test]
    fn snippet_renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let body = render_snippet_body(&fixture, &E2eConfig::default(), &ResolvedCrateConfig::default(), &[]);

        assert!(body.contains("catch (Exception error)"), "{body}");
        assert!(
            body.contains("throw new AssertionError(\"expected call to fail\")"),
            "{body}"
        );
    }

    #[test]
    fn snippet_presents_selected_result_fields() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "list_items",
            "description": "List items",
            "input": null,
            "assertions": [],
            "docs": {
                "topic": "items",
                "presentation": {
                    "operations": [{"op": "iterate", "path": "items", "item": "item", "fields": ["name"]}]
                }
            }
        }))
        .expect("fixture");
        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call: CallConfig {
                    function: "list_items".into(),
                    result_var: "result".into(),
                    ..CallConfig::default()
                },
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(body.contains("for (var item : result.items())"), "{body}");
        assert!(body.contains("System.out.println(item.name());"), "{body}");
        assert!(!body.contains("System.out.println(result);"), "{body}");
    }

    #[test]
    fn snippet_declares_typed_json_arguments_and_preserves_qualified_service_name() {
        let fixture = Fixture {
            id: "configured_call".into(),
            description: "Configured call".into(),
            input: serde_json::json!({"source": "example", "options": {"mode": "fast"}}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
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
                    name: "options".into(),
                    field: "options".into(),
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
                class: Some("dev.example.SampleService".into()),
                options_type: Some("ProcessOptions".into()),
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
        );

        assert!(body.contains("var optionsJson = \"{\\\"mode\\\":\\\"fast\\\"}\";"));
        assert!(body.contains("var options = JsonUtil.fromJson(optionsJson, ProcessOptions.class);"));
        assert!(body.contains("dev.example.SampleService.process(\"example\", options)"));
        assert!(!body.contains("DevExampleSampleService"));
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
            "java".into(),
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
        );

        assert!(
            body.contains("Files.readAllBytes(java.nio.file.Path.of(\"document.pdf\"))"),
            "{body}"
        );
        assert!(body.contains("Base64.getEncoder().encodeToString"), "{body}");
        assert!(body.contains("DocumentRequest.class"), "{body}");
    }
}
