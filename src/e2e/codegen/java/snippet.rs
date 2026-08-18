use heck::{ToLowerCamelCase, ToUpperCamelCase};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::TypeDef;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureEnv};

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
    let client_args = render_client_factory_args(fixture, e2e_config, call);
    let package_name = overrides
        .and_then(|value| value.module.clone())
        .unwrap_or_else(|| config.java_package());
    let needs_mapper = setup_lines.iter().any(|line| line.contains("MAPPER"));
    let presentation = crate::e2e::codegen::presentation::resolve(fixture, e2e_config, "java");
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let exception_class = format!("{class_name}Exception");
    let api_key_var = FixtureEnv::api_key_var_or_default(fixture.env.as_ref());

    crate::e2e::template_env::render(
        "java/snippet_body.jinja",
        minijinja::context! {
            package_name => package_name,
            class_name => class_name,
            setup_lines => setup_lines,
            client_factory => client_factory,
            client_args => client_args,
            function_name => function_name,
            args => args,
            result_var => call.effective_result_var(),
            returns_void => call.returns_void,
            needs_mapper => needs_mapper,
            fixture_id => fixture.id,
            presentation => presentation,
            expects_error => expects_error,
            exception_class => exception_class,
            api_key_var => api_key_var,
        },
    )
}

/// Argument list appended to a `client_factory` call when the project configures no
/// `[e2e.call.overrides.java] client_factory_trailing_args`.
///
/// These were hardcoded into `java/snippet_body.jinja` before the override was wired
/// up and remain the default, so a project that has not adopted the key keeps the
/// argument list it renders today.
const JAVA_CLIENT_FACTORY_FALLBACK_ARGS: [&str; 3] = ["null", "null", "null"];

/// The full argument list for a snippet's `client_factory` call: the credential, the
/// base URL, and whatever trails them.
///
/// The credential is always the `apiKey` local the template declares just above the
/// call — a snippet must read it from the environment rather than inline a literal.
fn render_client_factory_args(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call: &crate::e2e::config::CallConfig,
) -> String {
    let docs_client = fixture.docs_client();
    let base_url = match crate::e2e::codegen::client_factory::docs_base_url(docs_client) {
        Some(url) => format!("\"{}\"", crate::e2e::escape::escape_java(url)),
        None => "null".to_string(),
    };
    let trailing = crate::e2e::codegen::client_factory::trailing_args(
        docs_client,
        e2e_config,
        call,
        "java",
        &JAVA_CLIENT_FACTORY_FALLBACK_ARGS,
    );
    let mut args = vec!["apiKey".to_string(), base_url];
    args.extend(trailing);
    args.join(", ")
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
            Some(crate::e2e::template_env::render(
                "java/snippet_json_object_setup.jinja",
                minijinja::context! {
                    variable => arg.name,
                    json => crate::e2e::escape::escape_java(&json),
                    type_name => type_name,
                    file_reads => file_reads,
                },
            ))
        })
        .flat_map(|block| split_rendered_lines(&block))
        .collect()
}

/// ~keep `java/snippet_body.jinja` indents `setup_lines` by prepending the method
/// body's indent to each *entry* once (`        {{ line }}`), not to every physical
/// line inside it. A producer that renders a multi-statement block (this file's own
/// `snippet_json_object_setup.jinja` emits a `varJson = "...";` line and a separate
/// `var = JsonUtil.fromJson(...)` line, plus a base64-file-read block spanning three
/// lines) and pushes it as one `String` therefore gets the indent on only the first
/// physical line — every line after it renders flush left. Regression: every
/// generated Java snippet with a `json_object` arg had its `JsonUtil.fromJson(...)`
/// line at column 0 instead of the surrounding method's indent. Splitting the
/// rendered block here, so each physical line becomes its own `setup_lines` entry,
/// lets the template's per-entry indent apply uniformly while preserving each
/// producer's own relative indentation between lines (e.g. the base64 read's
/// continuation line and closing `);`).
pub(super) fn split_rendered_lines(block: &str) -> Vec<String> {
    block.trim_end().lines().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride};

    fn line_containing<'a>(body: &'a str, needle: &str) -> &'a str {
        body.lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no line contains {needle} in:\n{body}"))
    }

    fn leading_spaces(line: &str) -> usize {
        line.len() - line.trim_start().len()
    }

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
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
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

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(!body.contains("mockServer"), "mock-server property leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            body.contains("System.getenv(\"API_KEY\")"),
            "credential is not read from the environment:\n{body}"
        );
        assert!(
            body.contains("createClient(apiKey, null, null, null, null)"),
            "an unconfigured project must keep the argument list it renders today:\n{body}"
        );
    }

    fn client_release_snippet(expects_error: bool) -> String {
        let mut fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        if expects_error {
            fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
        }
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &config,
            &[],
        )
    }

    /// The generated Java facade class implements `AutoCloseable` (`service_close.jinja` /
    /// `opaque_handle_header.jinja` in the Java backend), so try-with-resources is the correct,
    /// idiomatic release for a client a docs snippet constructs. Mirrors the C# lane's `using var
    /// client`, which is the same construct for the same reason. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_in_a_try_with_resources_block() {
        let body = client_release_snippet(false);

        assert!(
            body.contains("try (var client = Sample.createClient(apiKey, null, null, null, null)) {"),
            "the client must be constructed inside a try-with-resources header:\n{body}"
        );
        assert!(
            body.contains("var result = client.chat();"),
            "the call moves onto the client the try-with-resources declares:\n{body}"
        );
    }

    /// The error-path half of `client_factory_snippet_releases_the_client_in_a_try_with_resources_block`.
    /// A `try (resource) { ... } catch (...) { ... }` releases the resource before the catch runs
    /// regardless of which statement inside the try threw — unlike the pre-existing Kotlin
    /// template, whose bare `client.close()` sat after the call and was skipped whenever the call
    /// itself threw. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_on_the_error_path() {
        let body = client_release_snippet(true);

        let try_with_resources = body
            .find("try (var client = Sample.createClient")
            .expect("expects-error snippet still constructs the client via try-with-resources");
        let catch_clause = body
            .find("catch (SampleException error)")
            .expect("catch clause present");
        assert!(
            try_with_resources < catch_clause,
            "the catch must follow the try-with-resources that declares the client:\n{body}"
        );
        assert!(
            !body.contains("} catch (SampleException error)"),
            "the try-with-resources closes on its own line before catch, not on the same brace:\n{body}"
        );
    }

    /// Negative control for the two tests above, and the pin that keeps this change scoped: a
    /// fixture with no `client_factory` constructs no client, so its snippet must be byte-for-byte
    /// what it was — no `try (`, no `AutoCloseable` resource, and the existing plain `try { ... }
    /// catch` shape for `expects_error`. A change that wraps every snippet's call in
    /// try-with-resources unconditionally would fail here. ~keep
    #[test]
    fn snippet_without_a_client_factory_is_unchanged() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(&fixture, &E2eConfig::default(), &config, &[]);

        assert!(
            !body.contains("try ("),
            "a snippet that constructs no client must emit no try-with-resources:\n{body}"
        );
        assert!(
            body.contains("        try {\n"),
            "the plain expects_error try block must be unchanged:\n{body}"
        );
        assert!(
            body.contains("        } catch (SampleException error) {\n"),
            "the plain expects_error catch must still close the try on the same line:\n{body}"
        );
    }

    fn client_snippet(docs: Option<serde_json::Value>, trailing_args: &[&str]) -> String {
        let mut fixture = Fixture {
            id: "custom_base_url".into(),
            description: "Custom base URL".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        fixture.docs = docs.map(|value| serde_json::from_value(value).expect("fixture docs"));
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                client_factory_trailing_args: trailing_args.iter().map(|arg| (*arg).to_string()).collect(),
                ..CallOverride::default()
            },
        );
        render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        )
    }

    #[test]
    fn configured_trailing_args_replace_the_templates_hardcoded_nulls() {
        let body = client_snippet(None, &["java.time.Duration.ofSeconds(30)", "2", "null"]);
        assert!(
            body.contains("createClient(apiKey, null, java.time.Duration.ofSeconds(30), 2, null)"),
            "{body}"
        );
    }

    #[test]
    fn a_snippet_renders_the_base_url_the_fixture_documents() {
        let body = client_snippet(
            Some(serde_json::json!({
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            })),
            &[],
        );
        assert!(
            body.contains("createClient(apiKey, \"https://llm.internal.example.com/v1\", null, null, null)"),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
        );
    }

    #[test]
    fn a_fixture_scoped_argument_list_outranks_the_configured_one() {
        let body = client_snippet(
            Some(serde_json::json!({
                "topic": "configuration",
                "client": {"args": {"java": ["30", "null", "null"]}}
            })),
            &["1", "1", "1"],
        );
        assert!(body.contains("createClient(apiKey, null, 30, null, null)"), "{body}");
    }

    #[test]
    fn a_documentation_client_declared_for_another_language_does_not_reach_java() {
        let body = client_snippet(
            Some(serde_json::json!({
                "topic": "configuration",
                "client": {"args": {"rust": ["Some(30)", "None", "None"]}}
            })),
            &[],
        );
        assert!(body.contains("createClient(apiKey, null, null, null, null)"), "{body}");
        assert!(
            !body.contains("Some(30)"),
            "rust syntax leaked into a java snippet:\n{body}"
        );
    }

    #[test]
    fn snippet_renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(&fixture, &E2eConfig::default(), &config, &[]);

        assert!(body.contains("catch (SampleException error)"), "{body}");
        assert!(!body.contains("AssertionError"), "{body}");
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

        // ~keep Regression: `snippet_json_object_setup.jinja` renders a two-statement
        // block (the JSON-literal line, then the `JsonUtil.fromJson(...)` line) as one
        // string; `java/snippet_body.jinja` only prepends the method body's indent to
        // the first physical line of each `setup_lines` entry, so an un-split block put
        // the second statement at column 0 in every generated Java snippet with a
        // `json_object` arg.
        let json_line = line_containing(&body, "var optionsJson =");
        let deserialize_line = line_containing(&body, "JsonUtil.fromJson(optionsJson");
        assert_eq!(
            leading_spaces(json_line),
            leading_spaces(deserialize_line),
            "json literal and its deserialize call must share the method body's indent:\n{body}"
        );
        assert_eq!(
            leading_spaces(deserialize_line),
            8,
            "expected the `public static void main` body's 8-space indent:\n{body}"
        );
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

        // ~keep Regression: the base64 file-read is a three-line block (opening call,
        // an indented continuation line, a closing `);`). Every physical line must
        // land at the method body's indent, with the continuation line one level
        // deeper — the same defect class as the two-statement json_object setup above.
        let open_line = line_containing(&body, "Base64.getEncoder().encodeToString(");
        let continuation_line = line_containing(&body, "Files.readAllBytes(java.nio.file.Path.of(\"document.pdf\"))");
        assert_eq!(leading_spaces(open_line), 8, "{body}");
        assert_eq!(leading_spaces(continuation_line), 12, "{body}");
    }
}
