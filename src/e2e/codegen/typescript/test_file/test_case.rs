use super::*;

/// Escape a string so it matches itself literally when embedded as the body of a
/// JS/TS regex literal (`/…/`), rather than being interpreted as a regex pattern.
///
/// Escapes the standard JS regex metacharacters, the `/` delimiter (which would
/// otherwise terminate the literal early), and control characters that cannot
/// appear raw inside a regex literal.
pub(in crate::e2e::codegen::typescript::test_file) fn escape_js_regex_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' | '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '/' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::typescript::test_file) fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    client_factory: Option<&str>,
    options_type: Option<&str>,
    e2e_config: &E2eConfig,
    lang: &str,
    nested_types: &std::collections::HashMap<String, String>,
    enum_fields: &std::collections::HashMap<String, String>,
    result_enum_fields: &std::collections::HashMap<String, String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) {
    let mut call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Fallback: if the resolved call has required args missing from input,
    // try to find a better-matching call from the named calls.
    call_config = crate::e2e::codegen::select_best_matching_call(call_config, e2e_config, fixture);
    let (ir_reachable_fields, ir_known_excluded_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields);
    let field_resolver = &call_field_resolver;
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let function_name = resolve_node_function_name(call_config);
    let result_var = call_config.effective_result_var();
    let call_is_async = call_config
        .overrides
        .get(lang)
        .and_then(|o| o.r#async)
        .unwrap_or(call_config.r#async);
    let args = recipe.args;
    let result_is_simple =
        call_config.result_is_simple || call_config.overrides.get(lang).is_some_and(|o| o.result_is_simple);

    // Resolve per-fixture wasm/node override fields (options_type, bigint_fields,
    // nested_types, enum_fields). Per-call overrides win over the file-level
    // default; missing fields fall back to the file-level default. WASM/wasm-bindgen
    // is the primary consumer of `bigint_fields` (u64/i64 setters reject Number).
    let per_call_override = recipe.override_config;
    let effective_options_type: Option<String> = per_call_override
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(|s| s.to_string()))
        .map(|type_name| canonical_ts_type_name(lang, &type_name, config));
    let mut effective_nested_types: std::collections::HashMap<String, String> = nested_types.clone();
    if let Some(o) = per_call_override {
        for (k, v) in &o.nested_types {
            effective_nested_types.insert(k.clone(), v.clone());
        }
    }
    let mut effective_enum_fields: std::collections::HashMap<String, String> = enum_fields.clone();
    if let Some(o) = per_call_override {
        for (k, v) in &o.enum_fields {
            effective_enum_fields.insert(k.clone(), v.clone());
        }
    }
    let mut effective_result_enum_fields: std::collections::HashMap<String, String> = result_enum_fields.clone();
    if let Some(o) = per_call_override {
        for (k, v) in &o.result_enum_fields {
            effective_result_enum_fields.insert(k.clone(), v.clone());
        }
    }
    // Per-language `extra_args` from call overrides — verbatim trailing
    // expressions appended after the configured args (e.g. `undefined` for an
    // optional trailing parameter the fixture cannot supply).
    let extra_args = recipe.extra_args;
    let global_bigint_fields: Vec<String> = e2e_config
        .call
        .overrides
        .get(lang)
        .map(|o| o.bigint_fields.clone())
        .unwrap_or_default();
    let mut effective_bigint_fields: std::collections::BTreeSet<String> = global_bigint_fields.into_iter().collect();
    if let Some(o) = per_call_override {
        for f in &o.bigint_fields {
            effective_bigint_fields.insert(f.clone());
        }
    }

    // Force test to async if we need to read files for bytes args or have trait bridge tests
    let has_trait_bridge = has_trait_bridge_args(args);
    let test_is_async = call_is_async || has_bytes_file_reads(&fixture.input, args) || has_trait_bridge;
    // Also force call to be treated as async for trait bridge tests so we await the calls
    let call_is_async = call_is_async || has_trait_bridge;

    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\\', "\\\\").replace('"', "\\\"");
    let async_kw = if test_is_async { "async " } else { "" };
    let await_kw = if call_is_async { "await " } else { "" };

    let handle_config_type = per_call_override.and_then(|o| o.handle_config_type.clone());

    let (mut setup_lines, mut args_str) = build_args_and_setup(
        &fixture.input,
        args,
        effective_options_type.as_deref(),
        fixture,
        &effective_nested_types,
        lang,
        &effective_enum_fields,
        &effective_bigint_fields,
        handle_config_type.as_deref(),
        type_defs,
        enums,
        wasm_type_prefix,
        config,
        false,
        referenced_enums,
    );

    if !extra_args.is_empty() {
        let extra_str = extra_args.join(", ");
        args_str = if args_str.is_empty() {
            extra_str
        } else {
            format!("{args_str}, {extra_str}")
        };
    }

    let mut visitor_arg = String::new();
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_arg = build_typescript_visitor(&mut setup_lines, visitor_spec);
    }

    let final_args = if visitor_arg.is_empty() {
        args_str
    } else if lang == "wasm" {
        if let Some(binding) = wasm_visitor_binding(config, effective_options_type.as_deref()) {
            apply_wasm_visitor_arg(&args_str, &visitor_arg, &binding)
        } else {
            args_str
        }
    } else if lang == "node" {
        // Node: visitor is read off `options.visitor` by the NAPI binding. Cast through
        // `any` so the plain visitor object satisfies the opaque `VisitorHandle` field type.
        node_visitor_args(&args_str, &visitor_arg)
    } else if args_str.is_empty() {
        format!("{{ visitor: {visitor_arg} }}")
    } else {
        format!("{args_str}, {{ visitor: {visitor_arg} }}")
    };

    let call_expr = if client_factory.is_some() {
        format!("client.{function_name}({final_args})")
    } else {
        format!("{function_name}({final_args})")
    };

    let base_url_expr = if fixture.has_host_root_route() {
        format!(
            "process.env.MOCK_SERVER_{} ?? `${{process.env.MOCK_SERVER_URL}}/fixtures/{}`",
            fixture.id.to_uppercase(),
            fixture.id
        )
    } else {
        format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{}`", fixture.id)
    };

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // A declared error value is matched against EITHER the thrown error's message
    // OR its `name`/type — the same disjunction the Rust and Python generators use
    // (see `crate::e2e::codegen::declared_error_value` doc comment). Building a
    // regex literal from the raw value would let regex metacharacters in the
    // fixture's declared string (e.g. `.`, `(`, `)`) change what the pattern
    // matches, so the value is escaped to match itself literally.
    let error_value_regex = expects_error
        .then(|| crate::e2e::codegen::declared_error_value(fixture))
        .flatten()
        .map(|value| format!("/{}/", escape_js_regex_literal(value)));

    // Build client setup
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let client_setup = if let Some(factory) = client_factory {
        if let Some(var) = api_key_var.filter(|_| has_mock) {
            let mock_url = format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{}`", fixture.id);
            format!(
                "const apiKey = process.env.{var};\n    \
                 const baseUrl = apiKey ? undefined : {mock_url};\n    \
                 console.log(`{id}: ${{apiKey ? 'using real API ({var} is set)' : 'using mock server ({var} not set)'}}`);\n    \
                 const client = {factory}(apiKey ?? 'test-key', baseUrl);",
                id = fixture.id
            )
        } else if has_mock {
            format!("const client = {factory}('test-key', {base_url_expr});")
        } else if let Some(var) = api_key_var {
            // Live-API tests: skip when the env var isn't set so the suite can run
            // without real credentials, matching the python codegen's pattern.
            format!(
                "const apiKey = process.env.{var};\n    \
                 if (!apiKey) {{\n        \
                     return;\n    \
                 }}\n    \
                 const client = {factory}(apiKey);"
            )
        } else {
            format!("const client = {factory}('test-key', {base_url_expr});")
        }
    } else {
        String::new()
    };

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());

    // Streaming-error detection: a fixture that calls a streaming function with an
    // error assertion (e.g. 401, 400 content-policy) — the upstream rejects before
    // any chunks arrive, but the NAPI / wasm binding returns the stream handle
    // synchronously. The HTTP error only surfaces when iterating, so we generate a
    // drain loop inside the `rejects.toThrow()` block so the error propagates
    // before the expect wrapper exits.
    //
    // Triggers in two cases:
    // - Declared streaming call (`call_config.streaming_enabled() = true`) + error fixture.
    // - Heuristic name-based detection (function name contains "stream") for
    //   fixtures that pre-date the explicit `streaming` flag.
    let is_streaming_error_call = expects_error && (is_streaming || function_name.to_lowercase().contains("stream"));

    // Build assertions body. Every assertion (including `not_error`) is passed
    // through — `render_assertion` decides what, if anything, to render for each
    // type. A prior `!call_config.returns_result` guard here skipped `not_error`
    // outright for some calls, but `render_assertion`'s own `not_error` arm was
    // *also* a no-op regardless of that condition, so the guard was a distinction
    // without a difference: both branches produced the same (vacuous) result.
    let mut assertions_body = String::new();
    for assertion in &fixture.assertions {
        render_assertion(
            &mut assertions_body,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            &effective_result_enum_fields,
            lang,
            is_streaming,
        );
    }

    // A fixture that declared at least one assertion but every one of them resolved
    // to a "skipped" comment (all its fields are unavailable on the result type) is
    // otherwise indistinguishable from a fixture with zero declared assertions — an
    // entirely comment-only, vacuously-passing test body. `not_error` already emits
    // a real `expect(...).toBeDefined()` (see `render_assertion`'s `not_error` arm),
    // so this only fires on the remaining gap: real field assertions that all got
    // dropped. TypeScript was the one backend in this defect class with no fallback
    // of any kind for that case — mirror python/php's `apply_vacuous_assertion_fallback`.
    // A fixture with genuinely zero declared assertions is left untouched, matching
    // every other backend's deliberate "just call it" smoke-test contract. ~keep
    apply_vacuous_assertion_fallback(
        &mut assertions_body,
        !fixture.assertions.is_empty(),
        is_streaming,
        result_var,
    );

    // Whether the call's result is worth binding to `const result = ...` rather
    // than discarding with a bare `await callExpr();`. Derived from what
    // `assertions_body` actually contains (a real, non-comment line) instead of a
    // separately maintained predicate over `fixture.assertions` — the previous
    // predicate excluded `not_error` from ever counting as "usable" even after
    // this fix made `render_assertion` emit a real `expect(...).toBeDefined()`
    // for it, which would have silently reintroduced the same drift this
    // regression test guards against. ~keep
    let has_usable_assertion = assertions_body
        .lines()
        .any(|line| !line.trim().is_empty() && !line.trim().starts_with("//"));

    crate::e2e::codegen::fail_on_unavailable_field_markers(&assertions_body, lang, &fixture.id, &fixture.assertions);

    // For streaming fixtures: capture the stream in `stream`, then collect into `chunks`.
    // Pass the actual `lang` (was hardcoded to "node") so wasm gets the
    // explicit-`next()` drain instead of the NAPI `for await` loop.
    let (ts_result_var, collect_snippet) = if is_streaming {
        let snip = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            lang, "stream", "chunks",
        )
        .unwrap_or_default();
        ("stream".to_string(), snip)
    } else {
        (result_var.to_string(), String::new())
    };

    // Extract skip reason if the fixture has a skip directive for this language
    let skip_reason = fixture.skip.as_ref().and_then(|skip| {
        if skip.should_skip(lang) {
            skip.reason.clone()
        } else {
            None
        }
    });

    // Long-running fixtures opt in explicitly through tags, or use slow-grammar timeouts.
    let timeout_ms = if fixture.tags.contains(&"embeddings".to_string()) {
        "600000"
    } else if is_slow_grammar(&fixture.input) {
        "90000"
    } else {
        "30000"
    };

    // For NAPI (Node.js) trait bridge tests, generate cleanup to dispose bridges
    let bridge_cleanup = if lang == "node" && has_trait_bridge {
        extract_bridge_cleanup(&setup_lines)
    } else {
        String::new()
    };

    // ~keep The `expects_error` branch of `typescript/test_function.jinja` renders the
    // `rejects` assertion and nothing else, so every other assertion on an error fixture — most
    // often an `equals` against `error.status_code` — used to leave no trace at all in the
    // generated test.
    let unrenderable_error_assertions = crate::e2e::codegen::error_path_assertions::render(fixture, "\t\t// ", lang);

    let ctx = minijinja::context! {
        test_name => test_name,
        description => description,
        async_kw => async_kw,
        client_setup => client_setup,
        setup_lines => setup_lines,
        call_expr => call_expr,
        has_usable_assertion => has_usable_assertion || is_streaming,
        result_var => ts_result_var,
        await_kw => await_kw,
        collect_snippet => collect_snippet,
        assertions_body => assertions_body,
        expects_error => expects_error,
        unrenderable_error_assertions => unrenderable_error_assertions.trim_end(),
        error_value_regex => error_value_regex,
        is_streaming_error_call => is_streaming_error_call,
        lang => lang,
        skip_reason => skip_reason,
        timeout_ms => timeout_ms,
        bridge_cleanup => bridge_cleanup,
    };
    let rendered = crate::e2e::template_env::render("typescript/test_function.jinja", ctx);
    out.push_str(&rendered);
}

/// When a fixture declares at least one assertion but the rendered body has no
/// executable statement — every field assertion resolved to a "skipped" comment —
/// inject a real assertion instead of leaving the test vacuous. `not_error`
/// already renders a real `expect(...).toBeDefined()` on its own (see
/// `render_assertion`'s `not_error` arm), so this only fires on the remaining
/// gap: declared field assertions that all turned out unavailable. Fixtures that
/// declare NO assertions at all are left untouched — a deliberate "just call it"
/// smoke test, matching every other backend in this defect class. ~keep
fn apply_vacuous_assertion_fallback(
    assertions_body: &mut String,
    has_declared_assertions: bool,
    is_streaming: bool,
    result_var: &str,
) {
    let has_real_assertion = assertions_body
        .lines()
        .any(|line| !line.trim().is_empty() && !line.trim().starts_with("//"));
    if !has_declared_assertions || has_real_assertion {
        return;
    }
    if is_streaming {
        assertions_body.push_str("    expect(chunks).toBeDefined();\n");
    } else {
        assertions_body.push_str(&format!("    expect({result_var}).toBeDefined();\n"));
    }
}

/// Check if a grammar has slow load times and needs extended timeout.
/// Tree-sitter grammars with complex scanner.c or large parser.c files
/// may take significantly longer to load and parse on first invocation.
fn is_slow_grammar(input: &serde_json::Value) -> bool {
    // Extract language from nested input.config.language
    let language = input
        .get("config")
        .and_then(|config| config.get("language"))
        .and_then(|lang| lang.as_str());

    // Grammars with slow parse times: known slow compilation or heavy scanner logic
    const SLOW_GRAMMARS: &[&str] = &["earthfile", "perl", "vb"];

    language.is_some_and(|lang| SLOW_GRAMMARS.contains(&lang))
}
