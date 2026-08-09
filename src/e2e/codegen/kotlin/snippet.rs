use heck::{ToLowerCamelCase, ToUpperCamelCase};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::TypeDef;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use super::args::{KotlinArgsContext, build_args_and_setup};

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
        .to_upper_camel_case();
    let function_name = overrides
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_lower_camel_case();
    let options_type = recipe
        .options_type
        .or_else(|| recipe.compatible_options_type(&["kotlin", "kotlin_android", "java", "csharp"]));
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
        },
    );
    if !recipe.extra_args.is_empty() {
        args = if args.is_empty() {
            recipe.extra_args.join(", ")
        } else {
            format!("{args}, {}", recipe.extra_args.join(", "))
        };
    }
    let client_factory = overrides.and_then(|value| value.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|value| value.client_factory.as_deref())
    });
    let needs_mapper = setup_lines.iter().any(|line| line.contains("MAPPER"));
    let is_async = client_factory.is_some() || kotlin_android_style || call.r#async;
    let package_name = if kotlin_android_style {
        config
            .kotlin_android
            .as_ref()
            .and_then(|value| value.package.clone())
            .unwrap_or_else(|| config.kotlin_package())
    } else {
        config.kotlin_package()
    };

    crate::e2e::template_env::render(
        "kotlin/snippet_body.jinja",
        minijinja::context! {
            package_name => package_name,
            needs_mapper => needs_mapper,
            setup_lines => setup_lines,
            client_factory => client_factory.map(ToLowerCamelCase::to_lower_camel_case),
            class_name => class_name,
            function_name => function_name,
            args => args,
            result_var => call.result_var,
            returns_void => call.returns_void,
            is_async => is_async,
            fixture_id => fixture.id,
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
        assert!(!body.contains("@Test"));
        assert!(!body.contains("assert"));
    }
}
