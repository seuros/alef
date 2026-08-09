use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Result, bail};
use std::collections::{HashMap, HashSet};

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<String> {
    if fixture.is_http_test() {
        bail!(
            "Elixir documentation snippets do not support HTTP harness fixture `{}`",
            fixture.id
        );
    }
    let lang = "elixir";
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call, type_defs);
    let override_config = recipe.override_config;
    let raw_module = override_config
        .and_then(|value| value.module.as_deref())
        .unwrap_or(&call.module);
    let module = if raw_module.contains('.') || raw_module.chars().next().is_some_and(char::is_uppercase) {
        raw_module.to_string()
    } else {
        super::values::elixir_module_name(raw_module)
    };
    let mut function = override_config
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_string();
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    if call.r#async && !function.ends_with("_async") && !is_streaming {
        function.push_str("_async");
    }
    let request_type = config
        .adapters
        .iter()
        .find(|value| value.name == call.function)
        .and_then(|value| value.request_type.as_deref())
        .and_then(|value| value.rsplit("::").next());
    let (mut setup_lines, mut args, _) = super::args::build_args_and_setup(
        &fixture.input,
        recipe.args,
        &module,
        recipe.options_type,
        override_config.and_then(|value| value.options_via.as_deref()),
        &HashMap::new(),
        fixture,
        override_config.and_then(|value| value.handle_struct_type.as_deref()),
        &HashSet::new(),
        &e2e_config.test_documents_relative_from(0),
        request_type,
        enums,
        config,
        type_defs,
        false,
    );
    if let Some(visitor) = &fixture.visitor {
        let visitor_arg = super::visitor::build_elixir_visitor(&mut setup_lines, visitor);
        args = [args, visitor_arg]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
    }
    if !recipe.extra_args.is_empty() {
        args = [args, recipe.extra_args.join(", ")]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
    }
    let call_expr = format!("{module}.{function}({args})");
    Ok(crate::e2e::template_env::render(
        "elixir/snippet_body.jinja",
        minijinja::context! {
            setup_lines => setup_lines, call_expr => call_expr, result_var => call.result_var,
            returns_void => call.returns_void, is_streaming => is_streaming,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_native_call_without_exunit() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        e2e.call.module = "sample".into();
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).unwrap();
        assert!(body.contains("Sample.load_document()"));
        assert!(!body.contains("assert"));
    }
}
