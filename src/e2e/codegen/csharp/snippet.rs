use std::collections::HashMap;

use heck::ToUpperCamelCase;

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

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
    let options_via = overrides.and_then(|value| value.options_via.as_deref());
    let mut visitor_declarations = Vec::new();
    let mut teardown_lines = Vec::new();
    let (setup_lines, mut args) = super::setup::build_args_and_setup(
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
    let needs_json = setup_lines.iter().any(|line| line.contains("JsonSerializer")) || args.contains("JsonSerializer");

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
            fixture_id => fixture.id,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride};

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
        assert!(body.contains("using System.Collections.Generic;"));
        assert!(!body.contains("[Fact]"));
        assert!(!body.contains("Assert."));
    }
}
