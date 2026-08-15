use std::collections::HashMap;

use heck::ToUpperCamelCase;

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureEnv};

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> String {
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("csharp", fixture, call, type_defs);
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
        &mut visitor_declarations,
        &mut teardown_lines,
    );
    if let Some(visitor_spec) = &fixture.visitor {
        let visitor_config = super::visitor::resolve_csharp_visitor_config(config, overrides, type_defs, visitor_spec);
        let visitor = super::visitor::build_csharp_visitor(
            &mut setup_lines,
            &mut visitor_declarations,
            &fixture.id,
            visitor_spec,
            &visitor_config,
        );
        let options_type = options_type
            .or_else(|| crate::e2e::codegen::recipe::trait_bridge_options_type(config))
            .unwrap_or("Options");
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
    let namespace = overrides
        .and_then(|value| value.module.clone())
        .or_else(|| config.csharp.as_ref().and_then(|value| value.namespace.clone()))
        .unwrap_or_else(|| config.name.to_upper_camel_case());
    let returns_void = call.returns_void
        || matches!(call.function.as_str(), "initialize" | "shutdown")
        || ["register_", "unregister_", "clear_"]
            .iter()
            .any(|prefix| call.function.starts_with(prefix));
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
    crate::e2e::template_env::render(
        "csharp/snippet_body.jinja",
        minijinja::context! {
            namespace => namespace,
            setup_lines => setup_lines,
            client_factory => client_factory,
            class_name => class_name,
            function_name => function_name,
            args => args,
            result_var => call.result_var,
            returns_void => returns_void,
            is_async => is_async,
            needs_json => needs_json,
            needs_system => needs_system,
            needs_collections => needs_collections,
            fixture_id => fixture.id,
            api_key_var => api_key_var,
            expects_error => expects_error,
            visitor_declarations => visitor_declarations,
        },
    )
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

#[cfg(test)]
mod tests {
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
        );

        assert!(body.contains("await SampleCoreConverter.LoadDocumentAsync()"));
        assert!(!body.contains("using System.Collections.Generic;"));
        assert!(body.contains("using System;"));
        assert!(body.contains("Console.WriteLine(document);"));
        assert!(!body.contains("[Fact]"));
        assert!(!body.contains("Assert."));
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
        );

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
        );

        assert!(body.contains("new SampleInput { Label = \"sample\" }"), "{body}");
        assert!(!body.contains("FromJson"), "{body}");
        assert!(!body.contains("JsonSerializer"), "{body}");
    }
}
