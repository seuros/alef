use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::Result;
use std::collections::HashMap;

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    _enums: &[EnumDef],
) -> Result<String> {
    if fixture.is_http_test() {
        return render_http_snippet(fixture);
    }
    let lang = "ruby";
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
    let module = override_config
        .and_then(|value| value.module.as_deref())
        .unwrap_or(&call.module);
    let receiver = super::values::ruby_module_name(module);
    let function = override_config
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function);
    let request_type = config
        .adapters
        .iter()
        .find(|value| value.name == call.function)
        .and_then(|value| value.request_type.as_deref())
        .and_then(|value| value.rsplit("::").next());
    let (mut setup_lines, mut args, _) = super::args::build_args_and_setup(
        &fixture.input,
        recipe.args,
        &receiver,
        module,
        recipe.options_type,
        &HashMap::new(),
        call.result_is_simple,
        fixture,
        request_type,
        config,
        type_defs,
    );
    if let Some(visitor) = &fixture.visitor {
        let visitor_arg = super::visitor::build_ruby_visitor(&mut setup_lines, visitor);
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
    let client_factory = override_config.and_then(|value| value.client_factory.as_deref());
    let call_receiver = if client_factory.is_some() { "client" } else { &receiver };
    let package = e2e_config
        .resolve_package(lang)
        .and_then(|value| value.name)
        .unwrap_or_else(|| config.name.replace('-', "_"));
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    Ok(crate::e2e::template_env::render(
        "ruby/snippet_body.jinja",
        minijinja::context! {
            package => package, receiver => receiver, setup_lines => setup_lines, client_factory => client_factory,
            call_receiver => call_receiver, function => function, args => args, result_var => call.result_var,
            returns_void => call.returns_void, is_streaming => is_streaming,
        },
    ))
}

fn render_http_snippet(fixture: &Fixture) -> Result<String> {
    let http = fixture.http.as_ref().expect("HTTP fixture checked by caller");
    let plan = crate::e2e::codegen::client::http_call::plan_request(http);
    let mut headers = plan.headers;
    if let Some(content_type) = &plan.content_type
        && !headers.keys().any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        headers.insert("Content-Type".into(), content_type.clone());
    }
    if !http.request.cookies.is_empty() {
        headers.insert(
            "Cookie".into(),
            http.request
                .cookies
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    let raw_body = plan.body.as_ref().is_some_and(|body| {
        matches!(body, serde_json::Value::String(_))
            && plan
                .content_type
                .as_deref()
                .is_some_and(crate::e2e::codegen::client::is_raw_text_content_type)
    });
    Ok(crate::e2e::template_env::render(
        "ruby/http_snippet.jinja",
        minijinja::context! {
            method_class => super::http::http_method_class(&http.request.method.to_uppercase()),
            path => format!("/fixtures/{}{}", fixture.id, http.request.path),
            headers => headers.iter().map(|(key, value)| minijinja::context! {
                key => crate::e2e::escape::ruby_string_literal(key),
                value => crate::e2e::escape::ruby_string_literal(value),
            }).collect::<Vec<_>>(),
            body => plan.body.as_ref().map(super::values::json_to_ruby),
            raw_body => raw_body,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_native_call_without_rspec() {
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
        assert!(!body.contains("expect("));
    }

    #[test]
    fn renders_http_request_without_rspec_assertions() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "create_item", "description": "Create item", "input": null,
            "http": {
                "handler": {"route": "/items", "method": "POST"},
                "request": {"method": "POST", "path": "/items", "body": {"name": "sample"}},
                "expected_response": {"status_code": 201}
            }
        }))
        .unwrap();
        let body = render_snippet_body(
            &fixture,
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .unwrap();
        assert!(body.contains("Net::HTTP::Post"));
        assert!(body.contains("/fixtures/create_item/items"));
        assert!(body.contains("request.body = { 'name' => 'sample' }.to_json"), "{body}");
        assert!(body.contains(".to_json\n\nresponse ="), "{body}");
        assert!(!body.contains("expect("));
    }
}
