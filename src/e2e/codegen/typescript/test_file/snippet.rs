use super::*;

pub(crate) struct SnippetContext<'a> {
    pub lang: &'a str,
    pub fixture: &'a Fixture,
    pub module: &'a str,
    pub client_factory: Option<&'a str>,
    pub e2e_config: &'a E2eConfig,
    pub type_defs: &'a [TypeDef],
    pub enums: &'a [EnumDef],
    pub wasm_type_prefix: &'a str,
    pub config: &'a crate::core::config::ResolvedCrateConfig,
}

pub(crate) fn render_snippet_body(context: SnippetContext<'_>) -> String {
    let SnippetContext {
        lang,
        fixture,
        module,
        client_factory,
        e2e_config,
        type_defs,
        enums,
        wasm_type_prefix,
        config,
    } = context;
    let docs_fixture = fixture.docs_call_fixture();
    let fixture = &docs_fixture;
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
    let options_type = recipe
        .options_type
        .map(|name| canonical_ts_type_name(lang, name, config));
    let mut nested_types = e2e_config
        .call
        .overrides
        .get(lang)
        .map(|value| value.nested_types.clone())
        .unwrap_or_default();
    let mut enum_fields = e2e_config
        .call
        .overrides
        .get(lang)
        .map(|value| value.enum_fields.clone())
        .unwrap_or_default();
    let mut bigint_fields: std::collections::BTreeSet<String> = e2e_config
        .call
        .overrides
        .get(lang)
        .map(|value| value.bigint_fields.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(value) = override_config {
        nested_types.extend(value.nested_types.clone());
        enum_fields.extend(value.enum_fields.clone());
        bigint_fields.extend(value.bigint_fields.iter().cloned());
    }
    let handle_config_type = override_config.and_then(|value| value.handle_config_type.as_deref());
    let (mut setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        recipe.args,
        options_type.as_deref(),
        fixture,
        &nested_types,
        lang,
        &enum_fields,
        &bigint_fields,
        handle_config_type,
        type_defs,
        enums,
        wasm_type_prefix,
        config,
        true,
    );
    if !recipe.extra_args.is_empty() {
        let extras = recipe.extra_args.join(", ");
        args = if args.is_empty() {
            extras
        } else {
            format!("{args}, {extras}")
        };
    }
    let mut visitor_imports = Vec::new();
    if let Some(visitor_spec) = &fixture.visitor {
        let visitor_arg = build_typescript_visitor(&mut setup_lines, visitor_spec);
        if lang == "wasm"
            && let Some(binding) = wasm_visitor_binding(config, options_type.as_deref())
        {
            visitor_imports.extend([binding.options_type.clone(), binding.handle_type.clone()]);
            args = apply_wasm_visitor_arg(&args, &visitor_arg, &binding);
        } else if lang == "node" {
            args = node_visitor_args(&args, &visitor_arg);
        }
    }

    let function_name = resolve_node_function_name(call);
    let effective_factory = override_config
        .and_then(|value| value.client_factory.as_deref())
        .or(client_factory);
    let call_expr = if effective_factory.is_some() {
        format!("client.{function_name}({args})")
    } else {
        format!("{function_name}({args})")
    };
    let client_setup = effective_factory
        .map(|factory| format!("const client = {factory}(\"your-api-key\");"))
        .unwrap_or_default();
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let mut imports = std::collections::BTreeSet::new();
    imports.insert(effective_factory.unwrap_or(&function_name).to_string());
    imports.extend(visitor_imports);
    let referenced_code = format!("{}\n{args}\n{client_setup}", setup_lines.join("\n"));
    if let Some(name) = options_type
        && referenced_code.contains(&name)
    {
        imports.insert(name);
    }
    imports.extend(
        nested_types
            .into_values()
            .chain(enum_fields.into_values())
            .filter(|name| referenced_code.contains(name)),
    );
    for arg in recipe.args {
        if arg.arg_type == "json_object"
            && let Some(type_name) = &arg.element_type
        {
            let type_name = canonical_ts_type_name(lang, type_name, config);
            if referenced_code.contains(&type_name) {
                imports.insert(type_name);
            }
        }
        if arg.arg_type == "handle" {
            imports.insert(format!("create{}", arg.name.to_upper_camel_case()));
        }
    }
    if let Some(name) = handle_config_type
        && referenced_code.contains(name)
    {
        imports.insert(name.to_string());
    }

    crate::e2e::template_env::render(
        "typescript/snippet_body.jinja",
        minijinja::context! {
            imports => imports.into_iter().collect::<Vec<_>>(), module => module,
            setup_lines => setup_lines, client_setup => client_setup, call_expr => call_expr,
            result_var => call.result_var, is_async => override_config.and_then(|value| value.r#async).unwrap_or(call.r#async),
            expects_error => expects_error,
            presentation => crate::e2e::codegen::presentation::resolve(fixture, e2e_config, lang),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::CallConfig;

    fn fixture() -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "quick_start".to_string(),
            category: None,
            description: "Quick start".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            assertions: Vec::new(),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
        }
    }

    #[test]
    fn async_snippet_reuses_the_test_call_shape_without_test_harness() {
        let mut e2e = E2eConfig {
            call: CallConfig {
                function: "load_document".to_string(),
                module: "@example/library".to_string(),
                result_var: "document".to_string(),
                r#async: true,
                ..CallConfig::default()
            },
            ..E2eConfig::default()
        };
        e2e.call
            .overrides
            .entry("node".into())
            .or_default()
            .enum_fields
            .insert("mode".into(), "UnusedMode".into());
        let fixture = fixture();
        let config = crate::core::config::ResolvedCrateConfig::default();
        let body = render_snippet_body(SnippetContext {
            lang: "node",
            fixture: &fixture,
            module: "@example/library",
            client_factory: None,
            e2e_config: &e2e,
            type_defs: &[],
            enums: &[],
            wasm_type_prefix: "",
            config: &config,
        });

        assert!(body.contains("import { loadDocument } from \"@example/library\";"));
        assert!(body.contains("const document = await loadDocument();"));
        assert!(!body.contains("vitest"));
        assert!(!body.contains("expect("));
        assert!(!body.contains("UnusedMode"));
    }

    #[test]
    fn docs_argument_override_imports_its_referenced_input_type() {
        let mut fixture = fixture();
        fixture.docs = Some(crate::e2e::fixture::FixtureDocs {
            topic: "guides".into(),
            stem: None,
            paths: Default::default(),
            title: None,
            description: None,
            presentation: Some(crate::e2e::fixture::FixtureDocsPresentation {
                input: Some(serde_json::json!({"source": {"kind": "uri", "uri": "guide.txt"}})),
                args: Some(vec![crate::e2e::config::ArgMapping {
                    name: "source".into(),
                    field: "source".into(),
                    arg_type: "json_object".into(),
                    optional: false,
                    owned: true,
                    element_type: Some("DocumentInput".into()),
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                }]),
                operations: Vec::new(),
            }),
            side_effects: crate::e2e::fixture::SideEffectClass::Safe,
            coverage_exceptions: Default::default(),
        });
        let e2e = E2eConfig {
            call: CallConfig {
                function: "load_document".into(),
                module: "@example/library".into(),
                result_var: "document".into(),
                r#async: true,
                ..CallConfig::default()
            },
            ..E2eConfig::default()
        };
        let config = crate::core::config::ResolvedCrateConfig::default();

        let body = render_snippet_body(SnippetContext {
            lang: "node",
            fixture: &fixture,
            module: "@example/library",
            client_factory: None,
            e2e_config: &e2e,
            type_defs: &[],
            enums: &[],
            wasm_type_prefix: "",
            config: &config,
        });

        assert!(body.contains("import { DocumentInput, loadDocument }"), "{body}");
        assert!(body.contains("const source: DocumentInput ="), "{body}");
        assert!(!body.contains("as DocumentInput"), "{body}");
    }

    #[test]
    fn expected_error_snippet_handles_the_rejected_call() {
        let mut fixture = fixture();
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            ..Default::default()
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "parse".into();
        e2e.call.r#async = true;
        let config = crate::core::config::ResolvedCrateConfig::default();
        let body = render_snippet_body(SnippetContext {
            lang: "node",
            fixture: &fixture,
            module: "@example/library",
            client_factory: None,
            e2e_config: &e2e,
            type_defs: &[],
            enums: &[],
            wasm_type_prefix: "",
            config: &config,
        });
        assert!(body.contains("try {"));
        assert!(body.contains("Call failed as expected"));
        assert!(!body.contains("const result = await"));
    }

    #[test]
    fn wasm_visitor_snippet_builds_and_attaches_the_real_bridge() {
        let mut fixture = fixture();
        fixture.visitor = Some(crate::e2e::fixture::VisitorSpec {
            callbacks: [("visit_text".into(), crate::e2e::fixture::CallbackAction::Continue)].into(),
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "render_document".into();
        e2e.call.overrides.entry("wasm".into()).or_default().options_type = Some("RenderOptions".into());
        let config = crate::core::config::ResolvedCrateConfig {
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "DocumentVisitor".into(),
                type_alias: Some("VisitorHandle".into()),
                options_type: Some("RenderOptions".into()),
                options_field: Some("visitor".into()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let body = render_snippet_body(SnippetContext {
            lang: "wasm",
            fixture: &fixture,
            module: "@example/wasm",
            client_factory: None,
            e2e_config: &e2e,
            type_defs: &[],
            enums: &[],
            wasm_type_prefix: "",
            config: &config,
        });

        assert!(body.contains("visitText(ctx: any, text: any)"));
        assert!(body.contains("new WasmVisitorHandle(_testVisitor)"));
        assert!(body.contains("RenderOptions.default()"));
        assert!(body.contains("VisitorHandle"));
    }
}
