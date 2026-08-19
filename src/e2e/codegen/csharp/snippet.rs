use std::collections::HashMap;

use anyhow::{Result, bail};
use heck::{ToSnakeCase, ToUpperCamelCase};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureEnv};

/// Render a C# documentation snippet without any core IR to consult.
///
/// Kept as the five-argument entry point every existing caller and test already uses: with no
/// `functions` the seam resolves to `TargetParams::IrAbsent`, which is exactly the state this path
/// was always in, so its output is unchanged by the seam. ~keep
pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<String> {
    render_snippet_body_with_ir(fixture, e2e_config, config, type_defs, enums, &[])
}

pub(super) fn render_snippet_body_with_ir(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<String> {
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("csharp", fixture, call, type_defs)
        .with_functions(functions);
    let target_params = recipe.target_params("csharp");
    let overrides = recipe.override_config;
    let class_name = crate::codegen::naming::csharp_wrapper_class_name(&config.name, "");
    let mut function_name = overrides
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_upper_camel_case();
    let is_async = overrides.and_then(|value| value.r#async).unwrap_or(call.r#async);
    if is_async && !function_name.ends_with("Async") {
        function_name.push_str("Async");
    }
    let options_type = recipe.options_type.or_else(|| {
        e2e_config
            .call
            .overrides
            .get("csharp")
            .and_then(|value| value.options_type.as_deref())
    });
    let options_via = overrides
        .and_then(|value| value.options_via.as_deref())
        .filter(|value| *value != "from_json");
    let mut visitor_declarations = Vec::new();
    let mut teardown_lines = Vec::new();
    let (mut setup_lines, mut args) = super::setup::build_args_and_setup(
        &fixture.input,
        recipe.args,
        &class_name,
        options_type,
        options_via,
        &HashMap::new(),
        &HashMap::new(),
        fixture,
        None,
        config,
        type_defs,
        enums,
        target_params,
        &mut visitor_declarations,
        &mut teardown_lines,
    );
    if let Some(visitor_spec) = &fixture.visitor {
        // A fixture that declares a visitor with no options type to bind it to is a
        // configuration defect, not a legitimate shape: there is nowhere to attach the
        // visitor. Fail closed here — the snippet pipeline records this as an
        // undocumented coverage gap naming the fixture — rather than fabricating a type
        // name, which publishes a documentation example that does not compile. Matches
        // `php::snippet` and `go::snippet`. Intentional omissions belong in the
        // fixture's `docs.coverage_exceptions`, where the reason is visible. ~keep
        let Some(options_type) =
            options_type.or_else(|| crate::e2e::codegen::recipe::trait_bridge_options_type(config))
        else {
            bail!(
                "C# documentation snippet `{}` needs an options type for its visitor",
                fixture.id
            );
        };
        let visitor_config = super::visitor::resolve_csharp_visitor_config(config, overrides, type_defs, visitor_spec);
        let visitor = super::visitor::build_csharp_visitor(
            &mut setup_lines,
            &mut visitor_declarations,
            &fixture.id,
            visitor_spec,
            &visitor_config,
        );
        setup_lines.push(format!("var options = new {options_type} {{ Visitor = {visitor} }};"));
        args = replace_or_append_options(&args, options_type);
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
                .get("csharp")
                .and_then(|value| value.client_factory.as_deref())
        })
        .map(ToUpperCamelCase::to_upper_camel_case);
    let client_args = render_client_factory_args(fixture, e2e_config, call);
    let namespace = overrides
        .and_then(|value| value.module.clone())
        .or_else(|| config.csharp.as_ref().and_then(|value| value.namespace.clone()))
        .unwrap_or_else(|| config.name.to_upper_camel_case());
    // Classify on the resolved name, snake-cased: the registry heuristic below is written in
    // Rust spelling, but a call whose base `function` is empty carries its only name in
    // `overrides.csharp.function`, spelled the C# way (`ClearValidators`). Reading the raw
    // base there yields `""`, which matches no prefix and misclassifies every registry call
    // as value-returning. ~keep
    let registry_name = call
        .effective_function("csharp")
        .map(|function| function.to_snake_case())
        .unwrap_or_default();
    let returns_void = call.returns_void
        || matches!(registry_name.as_str(), "initialize" | "shutdown")
        || ["register_", "unregister_", "clear_"]
            .iter()
            .any(|prefix| registry_name.starts_with(prefix));
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let api_key_var = FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
    let needs_json = setup_lines.iter().any(|line| line.contains("JsonSerializer")) || args.contains("JsonSerializer");
    let needs_system = expects_error
        || !returns_void
        || client_factory.is_some()
        || setup_lines.iter().any(|line| line.contains("Environment."));
    let needs_collections = setup_lines
        .iter()
        .any(|line| line.contains("List<") || line.contains("Dictionary<"));
    let presentation = crate::e2e::codegen::presentation::resolve(fixture, e2e_config, "csharp", type_defs);
    Ok(crate::e2e::template_env::render(
        "csharp/snippet_body.jinja",
        minijinja::context! {
            namespace => namespace,
            setup_lines => setup_lines,
            client_factory => client_factory,
            class_name => class_name,
            client_args => client_args,
            function_name => function_name,
            args => args,
            result_var => call.effective_result_var(),
            returns_void => returns_void,
            is_async => is_async,
            needs_json => needs_json,
            needs_system => needs_system,
            needs_collections => needs_collections,
            fixture_id => fixture.id,
            api_key_var => api_key_var,
            expects_error => expects_error,
            // `build_csharp_visitor` indents its class by four spaces so the e2e test file can nest
            // it inside the test class. A snippet is top-level statements followed by file-scope
            // declarations, where that indent is just wrong — and it was load-bearing wrong: the
            // batch validator's statement/declaration split keyed on column, so an indented class
            // stayed inside the wrapper method and failed to compile. ~keep
            visitor_declarations => visitor_declarations
                .iter()
                .map(|declaration| dedent_file_scope_declaration(declaration))
                .collect::<Vec<_>>(),
            presentation => presentation,
        },
    ))
}

/// Argument list appended to a `client_factory` call when the project configures no
/// `[e2e.call.overrides.csharp] client_factory_trailing_args`.
///
/// These were hardcoded into `csharp/snippet_body.jinja` before the override was wired
/// up and remain the default, so a project that has not adopted the key keeps the
/// argument list it renders today.
const CSHARP_CLIENT_FACTORY_FALLBACK_ARGS: [&str; 3] = ["null", "null", "null"];

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
        Some(url) => format!("\"{}\"", crate::e2e::escape::escape_csharp(url)),
        None => "null".to_string(),
    };
    let trailing = crate::e2e::codegen::client_factory::trailing_args(
        docs_client,
        e2e_config,
        call,
        "csharp",
        &CSHARP_CLIENT_FACTORY_FALLBACK_ARGS,
    );
    let mut args = vec!["apiKey".to_string(), base_url];
    args.extend(trailing);
    args.join(", ")
}

fn replace_or_append_options(args: &str, options_type: &str) -> String {
    if let Some(prefix) = args.strip_suffix(", null") {
        return format!("{prefix}, options");
    }
    let default_options = format!("new {options_type}()");
    if args == default_options {
        return "options".to_string();
    }
    if let Some(prefix) = args.strip_suffix(&format!(", {default_options}")) {
        return format!("{prefix}, options");
    }
    if args.is_empty() {
        "options".to_string()
    } else {
        format!("{args}, options")
    }
}

/// Strip the uniform four-space indent `build_csharp_visitor` adds for nesting inside an e2e test
/// class, so the same declaration reads correctly at a snippet's file scope. Lines that do not
/// carry the indent (blank ones) are passed through unchanged.
fn dedent_file_scope_declaration(declaration: &str) -> String {
    declaration
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::dedent_file_scope_declaration;

    /// `build_csharp_visitor` indents its class for nesting inside an e2e test class. A snippet puts
    /// it at file scope after top-level statements, and the stray indent was not merely cosmetic:
    /// the batch validator's statement/declaration split keyed on column, so the indented class
    /// stayed inside the wrapper method and 54 of one consumer's snippets failed to compile. ~keep
    #[test]
    fn a_file_scope_declaration_loses_the_nesting_indent() {
        let nested = "    sealed class ExampleVisitor : IHtmlVisitor\n    {\n        public int Value => 1;\n    }";

        assert_eq!(
            dedent_file_scope_declaration(nested),
            "sealed class ExampleVisitor : IHtmlVisitor\n{\n    public int Value => 1;\n}"
        );
    }

    #[test]
    fn a_blank_line_survives_dedenting_unchanged() {
        assert_eq!(
            dedent_file_scope_declaration("    class A\n\n    {\n    }"),
            "class A\n\n{\n}"
        );
    }

    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride};

    #[test]
    fn visitor_options_replace_the_placeholder_argument() {
        assert_eq!(
            replace_or_append_options("html, null", "ConversionOptions"),
            "html, options"
        );
        assert_eq!(
            replace_or_append_options("html, new ConversionOptions()", "ConversionOptions"),
            "html, options"
        );
    }

    #[test]
    fn snippet_keeps_async_native_call_without_xunit_harness() {
        let fixture = Fixture {
            id: "quick_start".into(),
            description: "Quick start".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "load_document".into(),
            result_var: "document".into(),
            r#async: true,
            ..CallConfig::default()
        };
        call.overrides.insert("csharp".into(), CallOverride::default());
        let config = ResolvedCrateConfig {
            name: "sample_core".into(),
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
        )
        .expect("snippet renders");

        assert!(body.contains("await SampleCoreConverter.LoadDocumentAsync()"));
        assert!(!body.contains("using System.Collections.Generic;"));
        assert!(body.contains("using System;"));
        assert!(body.contains("Console.WriteLine(document);"));
        assert!(!body.contains("[Fact]"));
        assert!(!body.contains("Assert."));
    }

    /// Render the C# snippet for a `clear_*` registry call spelled `spelling`, placed either at
    /// the call's base `function` or only in its `overrides.csharp.function`.
    fn registry_snippet(base: &str, csharp_override: Option<&str>) -> String {
        let fixture = Fixture {
            id: "clear_validators".into(),
            description: "Clear all validators".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: base.into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "csharp".into(),
            CallOverride {
                function: csharp_override.map(str::to_string),
                ..CallOverride::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample_core".into(),
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
            &[],
        )
        .expect("snippet renders")
    }

    #[test]
    fn a_registry_call_named_only_by_its_csharp_override_still_reads_as_void_returning() {
        // `function = ""` plus one override per language is the shape a trait-bridge registry
        // call takes when the bindings disagree on spelling. Classifying `returns_void` from the
        // raw base saw `""`, matched no `clear_` prefix, and bound a result from a void method.
        let body = registry_snippet("", Some("ClearValidators"));

        assert!(body.contains("SampleCoreConverter.ClearValidators();"), "{body}");
        assert!(
            !body.contains("var result ="),
            "a void C# method must not have its return value bound:\n{body}"
        );
        assert!(!body.contains("Console.WriteLine(result);"), "{body}");
    }

    #[test]
    fn a_registry_call_named_by_its_base_is_classified_exactly_as_before() {
        let body = registry_snippet("clear_validators", None);

        assert!(body.contains("SampleCoreConverter.ClearValidators();"), "{body}");
        assert!(!body.contains("var result ="), "{body}");
    }

    #[test]
    fn a_value_returning_call_is_not_swept_up_by_the_registry_prefixes() {
        let body = registry_snippet("", Some("LoadDocument"));

        assert!(
            body.contains("var result = SampleCoreConverter.LoadDocument();"),
            "resolving the override must not turn every call into a void one:\n{body}"
        );
    }

    #[test]
    fn documented_presentation_binds_the_result_and_reads_the_shown_fields() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "present_items", "description": "Present returned items", "input": null,
            "docs": {"topic": "guides", "presentation": {"operations": [
                {"op": "show", "path": "summary", "display": true},
                {"op": "iterate", "path": "items", "item": "item", "fields": ["label"]}
            ]}}
        }))
        .expect("fixture");
        let e2e = E2eConfig {
            call: CallConfig {
                function: "process".into(),
                result_var: "result".into(),
                ..CallConfig::default()
            },
            result_fields: ["summary".to_string(), "items".to_string()].into_iter().collect(),
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample_core".into(),
            ..ResolvedCrateConfig::default()
        };

        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");

        assert!(body.contains("var result = SampleCoreConverter.Process();"), "{body}");
        assert!(body.contains("Console.WriteLine(result.Summary);"), "{body}");
        assert!(body.contains("foreach (var item in result.Items)"), "{body}");
        assert!(body.contains("Console.WriteLine(item.Label);"), "{body}");
        assert!(
            !body.contains("Console.WriteLine(result);"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
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
            "csharp".into(),
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
            &[],
        )
        .expect("snippet renders");

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            body.contains("Environment.GetEnvironmentVariable(\"API_KEY\")"),
            "credential is not read from the environment:\n{body}"
        );
    }

    fn client_snippet(docs: Option<serde_json::Value>) -> String {
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
            "csharp".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
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
            &[],
        )
        .expect("snippet renders")
    }

    #[test]
    fn client_factory_snippet_renders_the_base_url_the_fixture_documents() {
        let body = client_snippet(Some(serde_json::json!({
            "topic": "configuration",
            "client": {"base_url": "https://llm.internal.example.com/v1"}
        })));

        assert!(
            body.contains("CreateClient(apiKey, \"https://llm.internal.example.com/v1\", null, null, null)"),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
        );
    }

    #[test]
    fn client_factory_snippet_without_a_docs_client_keeps_the_bare_call() {
        let body = client_snippet(None);

        assert!(
            body.contains("CreateClient(apiKey, null, null, null, null)"),
            "a fixture with no docs client must render the unconfigured argument list:\n{body}"
        );
    }

    #[test]
    fn snippet_renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let body = render_snippet_body(
            &fixture,
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(body.contains("catch (Exception error)"), "{body}");
        assert!(!body.contains("InvalidOperationException"), "{body}");
    }

    #[test]
    fn snippet_constructs_known_dto_without_json_round_trip() {
        let fixture = Fixture {
            id: "typed_input".into(),
            description: "Typed input".into(),
            input: serde_json::json!({"payload": {"label": "sample"}}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "payload".into(),
                field: "input.payload".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: Some("SampleInput".into()),
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "csharp".into(),
            CallOverride {
                options_via: Some("from_json".into()),
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
            &[TypeDef {
                name: "SampleInput".into(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "label".into(),
                    ty: crate::core::ir::TypeRef::String,
                    ..Default::default()
                }],
                ..TypeDef::default()
            }],
            &[],
        )
        .expect("snippet renders");

        assert!(body.contains("new SampleInput { Label = \"sample\" }"), "{body}");
        assert!(!body.contains("FromJson"), "{body}");
        assert!(!body.contains("JsonSerializer"), "{body}");
    }

    fn visitor_fixture() -> Fixture {
        serde_json::from_value(serde_json::json!({
            "id": "visitor_link_rewrite",
            "description": "Visitor rewrites links",
            "input": {"html": "<a href=\"a\">a</a>"},
            "visitor": {"callbacks": {"visit_link": {"action": "skip"}}}
        }))
        .expect("fixture")
    }

    fn visitor_call() -> CallConfig {
        CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "html".into(),
                field: "input.html".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        }
    }

    fn bridge_config(options_type: Option<&str>) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "sample_core".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "HtmlVisitor".into(),
                type_alias: Some("VisitorHandle".into()),
                param_name: Some("visitor".into()),
                options_type: options_type.map(str::to_string),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    /// Regression: a visitor fixture with no resolvable options type used to fall back to
    /// the literal type name `Options`, publishing `new Options { Visitor = .. }` — a
    /// documentation example naming a type that does not exist. It must fail closed
    /// instead, matching PHP and Go. ~keep
    #[test]
    fn visitor_without_a_trait_bridge_options_type_fails_instead_of_fabricating_one() {
        let error = render_snippet_body(
            &visitor_fixture(),
            &E2eConfig {
                call: visitor_call(),
                ..E2eConfig::default()
            },
            &bridge_config(None),
            &[],
            &[],
        )
        .expect_err("a visitor with no options type must not render");

        assert_eq!(
            format!("{error}"),
            "C# documentation snippet `visitor_link_rewrite` needs an options type for its visitor"
        );
    }

    /// Positive control for the above: with the bridge's `options_type` configured, the
    /// ordinary visitor path is unchanged and names the real type. ~keep
    #[test]
    fn visitor_with_a_trait_bridge_options_type_still_names_the_real_type() {
        let body = render_snippet_body(
            &visitor_fixture(),
            &E2eConfig {
                call: visitor_call(),
                ..E2eConfig::default()
            },
            &bridge_config(Some("ConversionOptions")),
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            body.contains("var options = new ConversionOptions { Visitor = _visitor_visitor_link_rewrite };"),
            "{body}"
        );
        assert!(!body.contains("new Options"), "{body}");
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
            "csharp".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
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
            &[],
        )
        .expect("snippet renders")
    }

    /// Every C# opaque handle alef generates is `IDisposable` over an owning `SafeHandle`, so a
    /// snippet holding a bare `var client` defers release to finalization. A `using` declaration
    /// binds it to the enclosing scope instead, which the compiler lowers to a `try`/`finally` —
    /// the reason this is a declaration and not a trailing `client.Dispose();`. ~keep
    #[test]
    fn client_factory_snippet_scopes_the_client_to_a_using_declaration() {
        let body = client_release_snippet(false);

        assert!(
            body.contains("using var client = "),
            "a constructed client must be scoped to a using declaration:\n{body}"
        );
        assert!(
            !body.contains("\nvar client = ") && !body.contains("  var client = "),
            "the unscoped declaration must be replaced, not duplicated:\n{body}"
        );
    }

    /// The error-path half of `client_factory_snippet_scopes_the_client_to_a_using_declaration`.
    /// The `expects_error` arm wraps the body in `try`/`catch`, and the whole point of a `using`
    /// declaration over a trailing `Dispose()` is that the release still runs when the call
    /// throws — so pin that the declaration lands inside the `try` the failing call sits in. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_on_the_error_path() {
        let body = client_release_snippet(true);

        let try_block = body.find("try\n{").expect("expects-error snippet opens a try block");
        let declaration = body.find("using var client = ").expect("using declaration");
        let catch_block = body.find("catch (Exception error)").expect("catch block");
        assert!(
            try_block < declaration && declaration < catch_block,
            "the using declaration must sit inside the try the failing call runs in:\n{body}"
        );
    }

    /// Negative control for the two tests above, and the pin that keeps this change scoped: a
    /// fixture with no `client_factory` constructs no client, so its snippet must be untouched.
    /// `using System;` and the namespace import are using *directives*, not declarations, so this
    /// asserts on `using var` specifically — an unconditional change would fail here. ~keep
    #[test]
    fn snippet_without_a_client_factory_emits_no_using_declaration() {
        let body = render_snippet_body(
            &Fixture {
                id: "quick_start".into(),
                description: "Quick start".into(),
                input: serde_json::Value::Null,
                ..Fixture::default()
            },
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            !body.contains("using var"),
            "a snippet that constructs no client must emit no using declaration:\n{body}"
        );
        assert!(
            !body.contains("var client"),
            "a snippet that constructs no client must not declare one:\n{body}"
        );
    }
}
