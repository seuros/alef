use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<String> {
    if fixture.is_http_test() {
        return render_http_snippet(fixture);
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
        args = inject_visitor_into_options(&mut setup_lines, &args, &visitor_arg);
    }
    if !recipe.extra_args.is_empty() {
        args = [args, recipe.extra_args.join(", ")]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
    }
    let call_expr = format!("{module}.{function}({args})");
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    Ok(crate::e2e::template_env::render(
        "elixir/snippet_body.jinja",
        minijinja::context! {
            setup_lines => setup_lines, call_expr => call_expr, result_var => call.result_var,
            returns_void => call.returns_void, is_streaming => is_streaming,
            expects_error => expects_error,
        },
    ))
}

fn inject_visitor_into_options(setup_lines: &mut Vec<String>, args: &str, visitor: &str) -> String {
    let parts = args.split(", ").collect::<Vec<_>>();
    match parts.as_slice() {
        [input, "nil"] => format!("{input}, %{{visitor: {visitor}}}"),
        [input, options] => {
            setup_lines.push(format!("{options} = Map.put({options}, :visitor, {visitor})"));
            format!("{input}, {options}")
        }
        [input] => format!("{input}, %{{visitor: {visitor}}}"),
        _ => args.to_string(),
    }
}

fn render_http_snippet(fixture: &Fixture) -> Result<String> {
    let http = fixture.http.as_ref().expect("HTTP fixture checked by caller");
    let plan = crate::e2e::codegen::client::http_call::plan_request(http);
    let mut headers = plan.headers;
    if let Some(content_type) = &plan.content_type
        && !headers.keys().any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        headers.insert("content-type".into(), content_type.clone());
    }
    if !http.request.cookies.is_empty() {
        headers.insert(
            "cookie".into(),
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
        "elixir/http_snippet.jinja",
        minijinja::context! {
            method => http.request.method.to_lowercase(),
            path => format!("/fixtures/{}{}", fixture.id, http.request.path),
            headers => headers.iter().map(|(key, value)| minijinja::context! {
                key => crate::e2e::escape::escape_elixir(key), value => crate::e2e::escape::escape_elixir(value),
            }).collect::<Vec<_>>(),
            body => plan.body.as_ref().map(super::values::json_to_elixir),
            raw_body => raw_body,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::StreamingConfig;
    use crate::e2e::config::{ArgMapping, CallOverride};

    #[test]
    fn nests_visitor_in_options_argument() {
        let mut setup = Vec::new();
        assert_eq!(
            inject_visitor_into_options(&mut setup, "html, nil", "visitor"),
            "html, %{visitor: visitor}"
        );
        assert_eq!(
            inject_visitor_into_options(&mut setup, "html, options", "visitor"),
            "html, options"
        );
        assert_eq!(setup, ["options = Map.put(options, :visitor, visitor)"]);
    }

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

    #[test]
    fn streaming_snippet_inspects_the_variable_it_bound() {
        let fixture = Fixture {
            id: "sample_stream".into(),
            description: "Sample stream".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "stream_items".into();
        e2e.call.module = "sample".into();
        e2e.call.result_var = "stream_result".into();
        e2e.call.streaming = Some(StreamingConfig::Enabled(true));

        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).unwrap();

        assert!(body.contains("stream_result = Sample.stream_items() |> Enum.to_list()"), "{body}");
        assert!(body.contains("IO.inspect(stream_result)"), "{body}");
        assert!(!body.contains("chunks ="), "{body}");
    }

    #[test]
    fn renders_expected_error_as_an_executable_example() {
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
        .expect("snippet");

        assert!(body.contains("rescue\n  error ->"), "{body}");
        assert!(!body.contains("expected call to fail"), "{body}");
    }

    #[test]
    fn renders_http_request_without_exunit_assertions() {
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
        assert!(body.contains("Req.request"));
        assert!(body.contains("/fixtures/create_item/items"));
        assert!(body.contains("json: %{\"name\" => \"sample\"}"));
        assert!(!body.contains("assert"));
    }

    #[test]
    fn reads_nested_typed_dto_files() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "document_input", "description": "Read a document",
            "input": {"request": {"content": "ignored"}}, "assertions": [],
            "docs": {"topic": "documents", "presentation": {
                "files": [{"field": "/request/content", "path": "document.pdf"}]
            }}
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        e2e.call.module = "Sample".into();
        e2e.call.args = vec![ArgMapping {
            name: "request".into(),
            field: "request".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e.call.overrides.insert(
            "elixir".into(),
            CallOverride {
                options_type: Some("DocumentRequest".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture.docs_call_fixture(),
            &e2e,
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet");

        assert!(
            body.contains("%Sample.DocumentRequest{content: File.read!(\"document.pdf\")}"),
            "{body}"
        );
    }
}
