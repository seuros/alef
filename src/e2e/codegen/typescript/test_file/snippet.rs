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
    infer_enum_fields(recipe.options_type, type_defs, enums, &mut enum_fields);
    for argument in recipe.args {
        infer_enum_fields(argument.element_type.as_deref(), type_defs, enums, &mut enum_fields);
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
        &mut Default::default(),
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
    // Both langs this renders for ignore a crate `error_type` name: node throws a plain
    // global `Error`, and wasm renders `String(error)` via `thrown_value_is_opaque` instead
    // of an `instanceof` check (see that template flag below), so it is never read. ~keep
    let error_type_name = "Error".to_string();
    let mut imports = std::collections::BTreeSet::new();
    imports.insert(effective_factory.unwrap_or(&function_name).to_string());
    // No `else` branch imports an error type here: node throws a plain global
    // `Error` (nothing to import) and wasm-bindgen throws a bare JS string
    // (also nothing to import, and no named error export exists to import in
    // the first place -- see the `thrown_value_is_opaque` template branch).
    imports.extend(visitor_imports);
    let referenced_code = format!("{}\n{args}\n{client_setup}", setup_lines.join("\n"));
    // Every imported type name goes through the same prefixing helper the body
    // uses. For wasm the emitted code constructs prefixed classes
    // (`WasmExtractInput`), so an unprefixed import names a symbol the package
    // does not export -- `render_test_file` prefixes at its own import sites for
    // exactly this reason. Non-wasm languages pass through unchanged.
    let import_name = |name: &str| wasm_prefixed_wrapped_type(lang, name, type_defs, enums, wasm_type_prefix);
    if let Some(name) = options_type.as_deref().map(import_name)
        && referenced_code.contains(&name)
    {
        imports.insert(name);
    }
    imports.extend(
        nested_types
            .into_values()
            .chain(enum_fields.into_values())
            .map(|name| import_name(&name))
            .filter(|name| referenced_code.contains(name)),
    );
    // A trait-bridge stub method returning a named enum annotates its signature with that
    // enum and casts through it (`(): ProcessingStage { return "\"Early\"" as unknown as
    // ProcessingStage; }` — see `emit_test_backend`'s `type_imports`). The enum is a
    // top-level type, so it is reached by neither `nested_types` nor `enum_fields`, and
    // without an import the emitted snippet does not type-check. Both uses are type
    // positions, but the module is imported as values here, which covers them.
    imports.extend(
        enums
            .iter()
            .map(|enum_def| import_name(&enum_def.name))
            .filter(|name| referenced_code.contains(name)),
    );
    for arg in recipe.args {
        if arg.arg_type == "json_object"
            && let Some(type_name) = &arg.element_type
        {
            let type_name = import_name(&canonical_ts_type_name(lang, type_name, config));
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
            error_type => error_type_name.clone(),
            thrown_value_is_opaque => lang == "wasm",
            returns_void => call.returns_void,
            presentation => crate::e2e::codegen::presentation::resolve(fixture, e2e_config, lang),
        },
    )
}

fn infer_enum_fields(
    type_name: Option<&str>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    fields: &mut std::collections::HashMap<String, String>,
) {
    let Some(type_name) = type_name else { return };
    let mut pending = vec![type_name.to_string()];
    let mut visited = std::collections::HashSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(type_def) = type_defs.iter().find(|definition| definition.name == name) else {
            continue;
        };
        for field in &type_def.fields {
            let Some(named) = named_type(&field.ty) else { continue };
            if enums.iter().any(|definition| definition.name == named) {
                // Key by owning-type + field, not the bare field name: this map
                // accumulates entries from every type reachable in the call's whole
                // type graph (see the two `infer_enum_fields` calls in
                // `render_snippet_body`), so two unrelated structs that happen to
                // share a field name (e.g. `TranscriptionConfig.model: WhisperModel`
                // and `LlmConfig.model: String`) must not collide on one key.
                fields
                    .entry(enum_field_key(&type_def.name, &field.name))
                    .or_insert_with(|| named.to_string());
            } else if type_defs.iter().any(|definition| definition.name == named) {
                pending.push(named.to_string());
            }
        }
    }
}

fn named_type(value: &crate::core::ir::TypeRef) -> Option<&str> {
    match value {
        crate::core::ir::TypeRef::Named(name) => Some(name),
        crate::core::ir::TypeRef::Optional(inner) | crate::core::ir::TypeRef::Vec(inner) => named_type(inner),
        _ => None,
    }
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
            preserve_input_urls: false,
            assertions: Vec::new(),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
        }
    }

    #[test]
    fn same_named_field_in_unrelated_struct_is_not_inferred_as_enum() {
        // Regression for #578: `infer_enum_fields` used to key its result on the
        // bare field name, so a genuinely enum-typed field on one struct
        // (`TranscriptionConfig.model: WhisperModel`) poisoned an unrelated
        // same-named `String` field on a different struct (`LlmConfig.model`)
        // reachable from the same call's type graph.
        let type_defs = [
            TypeDef {
                name: "TranscriptionConfig".into(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "model".into(),
                    ty: crate::core::ir::TypeRef::Named("WhisperModel".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            TypeDef {
                name: "LlmConfig".into(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "model".into(),
                    ty: crate::core::ir::TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ];
        let enums = [EnumDef {
            name: "WhisperModel".into(),
            ..Default::default()
        }];

        // Mirrors `render_snippet_body`, which accumulates `infer_enum_fields`
        // results from multiple type roots (the call's options type and each
        // argument's element type) into one shared map.
        let mut enum_fields = std::collections::HashMap::new();
        infer_enum_fields(Some("TranscriptionConfig"), &type_defs, &enums, &mut enum_fields);
        infer_enum_fields(Some("LlmConfig"), &type_defs, &enums, &mut enum_fields);

        let llm_object = serde_json::json!({"model": "openai/gpt-4o-mini"});
        let llm_expression = ts_builder_expression(
            llm_object.as_object().expect("object"),
            "LlmConfig",
            &Default::default(),
            "node",
            &enum_fields,
            &Default::default(),
            &type_defs,
            &enums,
            "",
            &[],
            &mut Default::default(),
        );
        assert_eq!(llm_expression, "{ model: \"openai/gpt-4o-mini\" } as LlmConfig");

        let transcription_object = serde_json::json!({"model": "base"});
        let transcription_expression = ts_builder_expression(
            transcription_object.as_object().expect("object"),
            "TranscriptionConfig",
            &Default::default(),
            "node",
            &enum_fields,
            &Default::default(),
            &type_defs,
            &enums,
            "",
            &[],
            &mut Default::default(),
        );
        assert_eq!(
            transcription_expression,
            "{ model: WhisperModel.Base } as TranscriptionConfig"
        );
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
            input: None,
            shows: Vec::new(),
            error: None,
            presentation: Some(crate::e2e::fixture::FixtureDocsPresentation {
                call: None,
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
                files: Vec::new(),
                operations: Vec::new(),
            }),
            client: None,
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
    fn docs_bytes_read_the_presented_relative_path() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({"content": "document.pdf"});
        fixture.args = vec![crate::e2e::config::ArgMapping {
            name: "content".into(),
            field: "content".into(),
            arg_type: "bytes".into(),
            optional: false,
            owned: true,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
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

        assert!(body.contains("readFile(\"document.pdf\")"), "{body}");
        assert!(body.contains("await loadDocument(_content_content)"), "{body}");
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
        assert!(body.contains("error instanceof Error"));
        assert!(!body.contains("expected call to fail"));
        assert!(!body.contains("const result = await"));
    }

    /// napi-rs (the node target's FFI boundary) converts every Rust error into a
    /// plain JS `Error` -- it never generates a named error class. A crate's
    /// `error_type` config (e.g. `error_type = "XbergError"` in alef.toml, used
    /// by every other language's docs snippets to build an idiomatic
    /// `import`/`instanceof` pair) does not apply to node: importing and
    /// `instanceof`-checking a class that the node package never exports fails
    /// with `TS2305: Module has no exported member 'XbergError'`.
    ///
    /// Before this fix, node snippets used `config.error_type_name()`
    /// unconditionally, so any crate with a custom `error_type` broke every
    /// generated node docs snippet that expects an error (see
    /// error_empty_mime.md, error_unsupported_mime.md, and others).
    #[test]
    fn node_error_snippet_uses_builtin_error_not_the_crate_error_type() {
        let mut fixture = fixture();
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            ..Default::default()
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "parse".into();
        e2e.call.r#async = true;
        let config = crate::core::config::ResolvedCrateConfig {
            error_type: Some("XbergError".into()),
            ..Default::default()
        };

        let node_body = render_snippet_body(SnippetContext {
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
        assert!(
            node_body.contains("error instanceof Error"),
            "node must fall back to the built-in Error, got: {node_body}"
        );
        assert!(
            !node_body.contains("XbergError"),
            "node must never reference the crate error type, got: {node_body}"
        );
        assert!(
            !node_body.contains("import { Error"),
            "Error is a global -- it must not be imported, got: {node_body}"
        );
    }

    /// `crates/xberg-wasm` throws a bare JS string from every fallible export
    /// (`.map_err(|e| JsValue::from_str(&e.to_string()))`) -- there is no
    /// `#[wasm_bindgen]` `XbergError`/`WasmXbergError` export to `instanceof`
    /// against, and `error.name`/`error.message` are always `undefined` on a
    /// thrown string. A crate's `error_type` config must not leak into the
    /// wasm catch block: the only value that reads correctly off a thrown
    /// wasm error is `String(error)`.
    ///
    /// Before this fix, wasm snippets emitted `if (error instanceof
    /// XbergError) { console.error(...) }` and imported `XbergError` from a
    /// module that never exports it, so every generated wasm docs snippet
    /// that expects an error failed at both import resolution and runtime
    /// (the check is always false, so the catch block is silently a no-op).
    #[test]
    fn wasm_error_snippet_reads_the_thrown_value_as_a_string() {
        let mut fixture = fixture();
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            ..Default::default()
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "parse".into();
        e2e.call.r#async = true;
        let config = crate::core::config::ResolvedCrateConfig {
            error_type: Some("XbergError".into()),
            ..Default::default()
        };

        let wasm_body = render_snippet_body(SnippetContext {
            lang: "wasm",
            fixture: &fixture,
            module: "@example/library",
            client_factory: None,
            e2e_config: &e2e,
            type_defs: &[],
            enums: &[],
            wasm_type_prefix: "",
            config: &config,
        });
        assert!(
            wasm_body.contains("console.error(String(error));"),
            "wasm must read the thrown value as a string, got: {wasm_body}"
        );
        assert!(
            !wasm_body.contains("instanceof"),
            "wasm has no named error class to instanceof-check, got: {wasm_body}"
        );
        assert!(
            !wasm_body.contains("XbergError"),
            "wasm must never reference the crate error type -- it isn't exported, got: {wasm_body}"
        );
        assert!(
            !wasm_body.contains("import { XbergError"),
            "wasm has nothing to import for errors, got: {wasm_body}"
        );
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
