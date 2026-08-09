use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Result, bail};
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::HashMap;

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    _enums: &[EnumDef],
) -> Result<String> {
    if fixture.is_http_test() {
        bail!(
            "PHP documentation snippets do not support HTTP harness fixture `{}`",
            fixture.id
        );
    }
    let lang = "php";
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
    let extension = config.php_extension_name();
    let default_namespace = crate::backends::php::naming::php_autoload_namespace(config);
    let namespace = override_config
        .and_then(|value| value.module.as_deref())
        .unwrap_or(&default_namespace);
    let class_name = override_config
        .and_then(|value| value.class.as_deref())
        .and_then(|value| value.split('\\').next_back())
        .map(str::to_string)
        .unwrap_or_else(|| extension.to_upper_camel_case());
    let mut function_name = override_config
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_string();
    if override_config.and_then(|value| value.function.as_ref()).is_none() {
        function_name = function_name.to_lower_camel_case();
    }
    let adapter = config.adapters.iter().find(|value| value.name == call.function);
    let request_type = adapter
        .and_then(|value| value.request_type.as_deref())
        .and_then(|value| value.rsplit("::").next());
    let owner_handle = adapter
        .is_some_and(|value| value.owner_type.is_some())
        .then(|| {
            recipe
                .args
                .iter()
                .find(|arg| arg.arg_type == "handle")
                .map(|arg| arg.name.clone())
        })
        .flatten();
    let options_via = override_config
        .and_then(|value| value.options_via.as_deref())
        .unwrap_or("array");
    let mut imports = Vec::new();
    let (mut setup_lines, mut args, _) = super::args::build_args_and_setup(
        &fixture.input,
        recipe.args,
        &class_name,
        &HashMap::new(),
        fixture,
        options_via,
        recipe.options_type,
        request_type,
        namespace,
        owner_handle.is_some(),
        type_defs,
        &config.serde_rename_all_for_language(crate::core::config::Language::Php),
        config,
        &mut imports,
    );
    if let Some(visitor) = &fixture.visitor {
        super::visitor::build_php_visitor(&mut setup_lines, visitor);
        let Some(options_type) = recipe
            .options_type
            .or_else(|| super::stubs::trait_bridge_options_type(config))
        else {
            bail!(
                "PHP documentation snippet `{}` needs an options type for its visitor",
                fixture.id
            );
        };
        if options_via == "from_json" {
            setup_lines.push(format!("$options = \\{namespace}\\{options_type}::from_json('{{}}');"));
            setup_lines.push(format!(
                "$visitorHandle = \\{namespace}\\VisitorHandle::from_php_object($visitor);"
            ));
            setup_lines.push("$options = $options->withVisitor($visitorHandle);".to_string());
        } else {
            setup_lines.push(format!("$builder = \\{namespace}\\{options_type}::builder();"));
            setup_lines.push("$options = $builder->visitor($visitor)->build();".to_string());
        }
        if args != "$options" {
            args = [args, "$options".to_string()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
        }
    }
    if !recipe.extra_args.is_empty() {
        args = [args, recipe.extra_args.join(", ")]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
    }
    let receiver = owner_handle.map(|value| format!("${value}")).unwrap_or_else(|| {
        override_config
            .and_then(|value| value.client_factory.as_ref())
            .map(|_| "$client".to_string())
            .unwrap_or_else(|| class_name.clone())
    });
    let operator = if receiver.starts_with('$') { "->" } else { "::" };
    let call_expr = format!("{receiver}{operator}{function_name}({args})");
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    let client_factory = override_config.and_then(|value| value.client_factory.as_deref());
    Ok(crate::e2e::template_env::render(
        "php/snippet_body.jinja",
        minijinja::context! {
            namespace => namespace, class_name => class_name, setup_lines => setup_lines,
            client_factory => client_factory, call_expr => call_expr, result_var => call.result_var,
            returns_void => call.returns_void, is_streaming => is_streaming,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_native_call_without_phpunit() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        e2e.call.module = "Sample".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).unwrap();
        assert!(body.contains("Sample::loadDocument()"));
        assert!(!body.contains("assert"));
    }
}
