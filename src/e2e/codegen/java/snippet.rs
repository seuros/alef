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
        .and_then(|value| value.class.as_deref())
        .unwrap_or(&config.name)
        .to_upper_camel_case();
    let function_name = overrides
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_lower_camel_case();
    let options_type = recipe
        .options_type
        .or_else(|| recipe.compatible_options_type(&["kotlin", "csharp", "c", "go", "python"]));
    let mut teardown = String::new();
    let (setup_lines, mut args) = build_args_and_setup(
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
        },
    )
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
    }
}
