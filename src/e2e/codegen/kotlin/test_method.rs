use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

use super::args::{KotlinArgsContext, build_args_and_setup};
use super::assertions::render_assertion;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    class_name: &str,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    type_enum_fields: &std::collections::HashMap<String, HashSet<String>>,
    kotlin_android_style: bool,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    // Delegate HTTP fixtures to the HTTP-specific renderer.
    if let Some(http) = &fixture.http {
        super::http::render_http_test_method(out, fixture, http);
        return;
    }

    // Resolve per-fixture call config (supports named calls via fixture.call field).
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Build per-call field resolver using the effective field sets for this call.
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone());
    let field_resolver = &call_field_resolver;
    let enum_fields = e2e_config.effective_fields_enum(call_config);
    let json_scalar_fields = e2e_config.effective_fields_json_scalar(call_config);
    let lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let call_overrides = call_config.overrides.get(lang);

    // Check for client_factory — when set, use instance-method call style.
    // Falls back to the global `[e2e.call.overrides.kotlin]` `client_factory` when
    // a per-call override is absent, matching the dart/swift renderers.
    //
    // For `kotlin_android_style`, also check `kotlin_android` and then `java`
    // overrides when neither a `kotlin` per-call nor a `kotlin` global override
    // is present. kotlin_android shares the same JNI bridge entry-points as the
    // Java facade, so a `java` `client_factory` applies equally.
    let client_factory = call_overrides
        .and_then(|o| o.client_factory.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|o| o.client_factory.as_deref())
        })
        .or_else(|| {
            if !kotlin_android_style {
                return None;
            }
            // kotlin_android fallback: check per-call kotlin_android → java, then
            // global kotlin_android → java overrides.
            call_config
                .overrides
                .get("kotlin_android")
                .and_then(|o| o.client_factory.as_deref())
                .or_else(|| {
                    call_config
                        .overrides
                        .get("java")
                        .and_then(|o| o.client_factory.as_deref())
                })
                .or_else(|| {
                    e2e_config
                        .call
                        .overrides
                        .get("kotlin_android")
                        .and_then(|o| o.client_factory.as_deref())
                })
                .or_else(|| {
                    e2e_config
                        .call
                        .overrides
                        .get("java")
                        .and_then(|o| o.client_factory.as_deref())
                })
        });

    let effective_function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.to_lower_camel_case());
    // Resolve per-fixture class name: prefer the kotlin_android call override, then
    // fall back to the global class_name. For trait bridge calls like register_document_extractor,
    // the override specifies the bridge class (e.g., DocumentExtractorBridge).
    let effective_class_name = call_overrides
        .and_then(|o| o.class.as_ref())
        .cloned()
        .or_else(|| {
            // For kotlin_android, also check the kotlin_android override.
            if kotlin_android_style {
                call_config
                    .overrides
                    .get("kotlin_android")
                    .and_then(|o| o.class.as_ref())
                    .cloned()
            } else {
                None
            }
        })
        .unwrap_or_else(|| class_name.to_string());
    let effective_result_var = &call_config.result_var;
    let function_name = effective_function_name.as_str();
    let class_name_for_call = effective_class_name.as_str();
    let result_var = effective_result_var.as_str();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args: &[crate::e2e::config::ArgMapping] = recipe.args;
    // Resolve per-fixture options_type using per-fixture call config resolution.
    // Per-fixture kotlin overrides take precedence, then fall back to class-level,
    // then to any other language's options_type for the same call (kotlin_android, java, csharp, etc.).
    // This mirrors the Python e2e codegen pattern where fixture_opts_type is resolved
    // per-fixture from the call config overrides, ensuring enums and types are correctly
    // imported and constructed.
    let compatible_options_languages: &[&str] = if kotlin_android_style {
        &["kotlin_android", "kotlin", "java", "csharp", "c", "go", "php", "python"]
    } else {
        &["csharp", "c", "go", "php", "python"]
    };
    let fixture_options_type: Option<String> = call_overrides
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(str::to_string))
        .or_else(|| {
            recipe
                .compatible_options_type(compatible_options_languages)
                .map(str::to_string)
        });
    let options_type = fixture_options_type.as_deref();

    // Resolve per-fixture result_is_simple: prefer the kotlin override, then the
    // class-level default, then any sibling language override (java/csharp/go).
    // The Kotlin facade shares its return-type shape with the Java facade, so a
    // declaration in any of those bindings applies to Kotlin too.
    let effective_result_is_simple = call_overrides.is_some_and(|o| o.result_is_simple)
        || call_config.result_is_simple
        || result_is_simple
        || ["java", "csharp", "go"]
            .iter()
            .any(|cand| call_config.overrides.get(*cand).is_some_and(|o| o.result_is_simple));
    let result_is_simple = effective_result_is_simple;

    // Resolve per-fixture result_is_option: prefer the kotlin override, then the
    // call-level default. When set the function returns `T?` and bare-result
    // emptiness assertions must use a null-check instead of `.isEmpty()`.
    let result_is_option = call_overrides.is_some_and(|o| o.result_is_option) || call_config.result_is_option;

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    let stream_lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let collect_snippet = if is_streaming && !expects_error {
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            stream_lang,
            result_var,
            "chunks",
        )
        .unwrap_or_default()
    } else {
        String::new()
    };

    // Check if this test needs ObjectMapper deserialization for json_object args.
    // Uses `resolve_field` so that `field = "input"` resolves to the whole fixture
    // input (and not a nested key called "input"), matching dart/swift behavior.
    // Also include tests with array element types, which are deserialized inline.
    let needs_deser = args.iter().any(|arg| {
        let val = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);
        arg.arg_type == "json_object"
            && !val.is_null()
            && crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, val).is_some()
    }) || args.iter().any(|arg| {
        arg.arg_type == "json_object"
            && arg.element_type.is_some()
            && !crate::e2e::codegen::resolve_field(&fixture.input, &arg.field).is_null()
    });

    // Merge per-call kotlin enum_fields (HashMap key = field path, value = enum type name)
    // into the global fields_enum set so that call-specific enum-typed result fields
    // (e.g. `status` on BatchObject) route through `.getValue()` in assertions even
    // when absent from the global `fields_enum` list.  Mirrors the Java codegen at
    // codegen/java.rs where per-call overrides are merged before assertion rendering.
    //
    // Additionally, auto-detect enum-typed fields by looking up the call's result type
    // in `type_enum_fields` (built from the IR TypeDef list). This handles the common
    // case where a field's Rust type is a `Named(EnumName)` that was never explicitly
    // listed in the alef.toml `enum_fields` table.
    let effective_enum_fields: std::borrow::Cow<HashSet<String>> = {
        // Resolve the result type name for this call. Prefer the kotlin override, then
        // java, then c — the Kotlin facade re-exports Java facade types unchanged.
        let result_type_name: Option<&str> = call_overrides
            .and_then(|co| co.result_type.as_deref())
            .or_else(|| call_config.overrides.get("java").and_then(|o| o.result_type.as_deref()))
            .or_else(|| call_config.overrides.get("c").and_then(|o| o.result_type.as_deref()));
        let auto_enum_fields: Option<&HashSet<String>> = result_type_name.and_then(|name| type_enum_fields.get(name));
        // For kotlin_android, also pull enum_fields from the `java` and
        // `kotlin_android` per-call overrides, since those binding layers share
        // the same JNI bridge and response types.
        let java_call_overrides = if kotlin_android_style {
            call_config
                .overrides
                .get("java")
                .or_else(|| call_config.overrides.get("kotlin_android"))
        } else {
            None
        };
        let has_per_call = call_overrides.is_some_and(|co| !co.enum_fields.is_empty())
            || java_call_overrides.is_some_and(|co| !co.enum_fields.is_empty());
        let has_auto = auto_enum_fields.is_some_and(|f| !f.is_empty());
        if has_per_call || has_auto {
            let mut merged = enum_fields.clone();
            if let Some(co) = call_overrides {
                merged.extend(co.enum_fields.keys().cloned());
            }
            if let Some(co) = java_call_overrides {
                merged.extend(co.enum_fields.keys().cloned());
            }
            if let Some(auto_fields) = auto_enum_fields {
                merged.extend(auto_fields.iter().cloned());
            }
            std::borrow::Cow::Owned(merged)
        } else {
            std::borrow::Cow::Borrowed(enum_fields)
        }
    };
    let enum_fields: &HashSet<String> = &effective_enum_fields;

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on the
    // owner handle (`engine.streamItems(req)`), not as static/client facade
    // methods — mirrors the Java e2e backend (java/test_method.rs), which
    // deliberately emits no static/client streaming methods for these adapters.
    // Capture the owner handle variable so the call is rendered as an
    // instance-method invocation instead of falling through to the flat-function
    // (or client-factory) call style.
    let adapter = config.adapters.iter().find(|a| a.name == call_config.function.as_str());
    let is_streaming_owner_adapter = adapter.is_some_and(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming) && a.owner_type.is_some()
    });
    let streaming_owner_handle: Option<String> = if is_streaming_owner_adapter {
        args.iter().find(|a| a.arg_type == "handle").map(|a| a.name.clone())
    } else {
        None
    };

    let _ = writeln!(out, "    @Test");
    if client_factory.is_some() || kotlin_android_style {
        let _ = writeln!(out, "    fun test{method_name}() = runBlocking {{");
    } else {
        let _ = writeln!(out, "    fun test{method_name}() {{");
    }
    let _ = writeln!(out, "        // {description}");

    // Collect ObjectMapper deserialization bindings for json_object args.
    // Object args use the configured `options_type`. Array args carrying
    // `element_type` are emitted as inline List<T> literals below
    // (build_args_and_setup), so no deser binding is needed.
    //
    // For error tests we want these `val xxx = MAPPER.readValue(...)` lines
    // INSIDE the assertFailsWith block, so that Jackson validation errors on
    // the request literal (e.g. an unknown enum like `purpose: "invalid"`)
    // are caught by the test instead of bubbling up as test failures. So
    // collect into a Vec and let the caller decide where to emit them.
    let mut deser_lines: Vec<String> = Vec::new();
    if needs_deser {
        for arg in args {
            if arg.arg_type != "json_object" {
                continue;
            }
            let val = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);
            if val.is_null() {
                continue;
            }
            // Skip arrays that we materialise inline rather than deserialising via Jackson.
            if val.is_array() && arg.element_type.is_some() {
                continue;
            }
            let Some(opts_type) = crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, val)
            else {
                continue;
            };
            let normalized = crate::e2e::codegen::transform_json_keys_for_language(val, "snake_case");
            let json_str = serde_json::to_string(&normalized).unwrap_or_default();
            let var_name = &arg.name;
            if crate::e2e::codegen::value_contains_mock_url_placeholder(&normalized) {
                let env_key = crate::e2e::codegen::mock_url_env_key(&fixture.id);
                deser_lines.push(format!(
                    "val {var_name}MockBaseUrl = System.getProperty(\"mockServer.{fixture_id}\", System.getenv(\"{env_key}\") ?: ((System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\"))",
                    fixture_id = fixture.id,
                ));
                deser_lines.push(format!(
                    "val {var_name}Json = \"{}\".replace(\"{}\", {var_name}MockBaseUrl)",
                    crate::e2e::escape::escape_kotlin(&json_str),
                    crate::e2e::escape::escape_kotlin(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                ));
                deser_lines.push(format!(
                    "val {var_name} = MAPPER.readValue({var_name}Json, {opts_type}::class.java)"
                ));
            } else {
                deser_lines.push(format!(
                    "val {var_name} = MAPPER.readValue(\"{}\", {opts_type}::class.java)",
                    crate::e2e::escape::escape_kotlin(&json_str)
                ));
            }
        }
    }
    if !expects_error {
        for line in &deser_lines {
            let _ = writeln!(out, "        {line}");
        }
    }

    let (setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        KotlinArgsContext {
            fixture,
            class_name,
            options_type,
            fixture_id: &fixture.id,
            kotlin_android_style,
            config,
            type_defs,
            owner_handle_is_receiver: streaming_owner_handle.is_some(),
        },
    );

    // When client_factory is set, emit client-object instantiation + instance method call.
    // The factory name is a function on the Kotlin facade object
    // that constructs the coroutine-friendly Kotlin client wrapper from the
    // raw apiKey + baseUrl pair the test owns.
    if let Some(factory) = client_factory {
        let fixture_id = &fixture.id;
        // Prefer system properties set by MockServerListener (which spawns the
        // mock-server in-process when MOCK_SERVER_URL isn't pre-set). The
        // per-fixture property holds the full URL; fall back to the base URL
        // (mockServerUrl or env var) with the /fixtures/<id> suffix appended.
        let mock_url_expr = format!(
            "System.getProperty(\"mockServer.{fixture_id}\", (System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\")"
        );
        // For a streaming owner_type adapter the call is dispatched on the owner
        // handle instance, not on the mock-backed `client` the factory constructs.
        let call_receiver = streaming_owner_handle.as_deref().unwrap_or("client");
        if expects_error {
            // Wrap setup + client construction + call in assertFailsWith so
            // validation errors thrown during request construction
            // (e.g. Jackson rejecting an unknown enum literal) are also caught.
            // Mirrors the flat-function path's behaviour below.
            // For streaming functions, the error fires during collection, not at
            // call time — append the collect suffix so the flow is consumed.
            let call_expr = if is_streaming {
                let collect_suffix = if kotlin_android_style {
                    ".toList()"
                } else {
                    ".asSequence().toList()"
                };
                format!("{call_receiver}.{function_name}({args_str}){collect_suffix}")
            } else {
                format!("{call_receiver}.{function_name}({args_str})")
            };
            let _ = writeln!(out, "        assertFailsWith<Exception> {{");
            for line in &deser_lines {
                let _ = writeln!(out, "            {line}");
            }
            for line in &setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            let _ = writeln!(
                out,
                "            val client = {class_name_for_call}.{factory}(apiKey = \"test-key\", baseUrl = {mock_url_expr})"
            );
            let _ = writeln!(out, "            {call_expr}");
            let _ = writeln!(out, "            client.close()");
            let _ = writeln!(out, "        }}");
            // Trailing `Unit` so the runBlocking { ... } lambda's final
            // expression is Unit (not the Exception returned by assertFailsWith).
            // The enclosing `fun ... = runBlocking { ... }` then infers Unit
            // as the test function's return type — JUnit 5 silently skips
            // any @Test method whose return type is not void/Unit.
            let _ = writeln!(out, "        Unit");
            let _ = writeln!(out, "    }}");
            return;
        }
        for line in &setup_lines {
            let _ = writeln!(out, "        {line}");
        }
        let _ = writeln!(
            out,
            "        val client = {class_name_for_call}.{factory}(apiKey = \"test-key\", baseUrl = {mock_url_expr})"
        );
        let _ = writeln!(
            out,
            "        val {result_var} = {call_receiver}.{function_name}({args_str})"
        );
        if !collect_snippet.is_empty() {
            let _ = writeln!(out, "        {collect_snippet}");
        }
        for assertion in &fixture.assertions {
            render_assertion(
                out,
                assertion,
                result_var,
                class_name,
                field_resolver,
                result_is_simple,
                result_is_option,
                enum_fields,
                json_scalar_fields,
                e2e_config.effective_fields_c_types(call_config),
                is_streaming,
                kotlin_android_style,
            );
        }
        let _ = writeln!(out, "        client.close()");
        let _ = writeln!(out, "    }}");
        return;
    }

    // Flat-function call style (no client_factory). For a streaming owner_type
    // adapter the call is an instance method on the owner handle rather than a
    // static facade call.
    let call_receiver = streaming_owner_handle.as_deref().unwrap_or(class_name_for_call);
    if expects_error {
        // Wrap setup + call in assertFailsWith so validation errors thrown
        // during engine creation are also caught (mirrors Java's assertThrows).
        let _ = writeln!(out, "        assertFailsWith<Exception> {{");
        for line in &deser_lines {
            let _ = writeln!(out, "            {line}");
        }
        for line in &setup_lines {
            let _ = writeln!(out, "            {line}");
        }
        let _ = writeln!(out, "            {call_receiver}.{function_name}({args_str})");
        let _ = writeln!(out, "        }}");
        // Trailing Unit — see comment in the client-factory branch above.
        let _ = writeln!(out, "        Unit");
        let _ = writeln!(out, "    }}");
        return;
    }

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    let _ = writeln!(
        out,
        "        val {result_var} = {call_receiver}.{function_name}({args_str})"
    );

    if !collect_snippet.is_empty() {
        let _ = writeln!(out, "        {collect_snippet}");
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            class_name,
            field_resolver,
            result_is_simple,
            result_is_option,
            enum_fields,
            json_scalar_fields,
            &e2e_config.fields_c_types,
            is_streaming,
            kotlin_android_style,
        );
    }

    let _ = writeln!(out, "    }}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::extras::{AdapterConfig, AdapterPattern};
    use crate::e2e::config::{ArgMapping, CallConfig};

    fn handle_arg(name: &str) -> ArgMapping {
        ArgMapping {
            name: name.to_string(),
            field: format!("input.{name}"),
            arg_type: "handle".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn string_arg(name: &str) -> ArgMapping {
        ArgMapping {
            name: name.to_string(),
            field: format!("input.{name}"),
            arg_type: "string".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    /// Synthetic streaming `owner_type` adapter for `function_name`, matching
    /// the shape declared by `[[crates.adapters]]` in `alef.toml`.
    fn streaming_owner_adapter(function_name: &str, owner_type: &str) -> AdapterConfig {
        AdapterConfig {
            name: function_name.to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: format!("test_core::{function_name}"),
            params: Vec::new(),
            returns: None,
            error_type: None,
            owner_type: Some(owner_type.to_string()),
            item_type: Some("Item".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: Vec::new(),
        }
    }

    fn render(call: CallConfig, adapters: Vec<AdapterConfig>) -> String {
        let fixture = Fixture {
            id: "call_fixture".to_string(),
            description: "call fixture".to_string(),
            input: serde_json::json!({ "handle": {}, "url": "https://example.com" }),
            ..Fixture::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig {
            adapters,
            ..ResolvedCrateConfig::default()
        };
        let mut out = String::new();
        render_test_method(
            &mut out,
            &fixture,
            "Facade",
            "",
            "",
            &[],
            None,
            false,
            &e2e_config,
            &std::collections::HashMap::new(),
            false,
            &config,
            &[],
        );
        out
    }

    /// A streaming call routed through a `[[crates.adapters]]` entry with
    /// `owner_type` set must be rendered as an instance method on the owner
    /// handle (`handle.streamItems(...)`), never as a static facade call that
    /// passes the handle as a positional argument. Mirrors the Java e2e
    /// backend's `streaming_owner_handle` behavior (java/test_method.rs).
    #[test]
    fn streaming_owner_type_adapter_uses_handle_as_instance_receiver() {
        let call = CallConfig {
            function: "stream_items".to_string(),
            result_var: "result".to_string(),
            args: vec![handle_arg("handle"), string_arg("url")],
            ..CallConfig::default()
        };
        let out = render(call, vec![streaming_owner_adapter("stream_items", "Engine")]);

        assert!(
            out.contains("val result = handle.streamItems(\"https://example.com\")"),
            "expected an instance call on the owner handle, got:\n{out}"
        );
        assert!(
            !out.contains("Facade.streamItems"),
            "must not emit a static facade call for a streaming owner_type adapter, got:\n{out}"
        );
        assert!(
            out.contains("val handle = Facade.createHandle(null)"),
            "the handle's construction line must still be emitted, got:\n{out}"
        );
        assert!(
            !out.contains("streamItems(handle, "),
            "the handle must not also appear as a positional argument, got:\n{out}"
        );
    }

    /// Regression guard: a call with no adapter entry (or an adapter that is
    /// not both `Streaming` and `owner_type`-bearing) must keep the plain
    /// static-facade call shape, with the handle passed positionally.
    #[test]
    fn non_owner_type_call_keeps_static_facade_call_with_handle_positional() {
        let call = CallConfig {
            function: "do_thing".to_string(),
            result_var: "result".to_string(),
            args: vec![handle_arg("handle"), string_arg("url")],
            ..CallConfig::default()
        };

        // No adapters at all.
        let out_no_adapter = render(call.clone(), Vec::new());
        assert!(
            out_no_adapter.contains("val result = Facade.doThing(handle, \"https://example.com\")"),
            "expected an unchanged static facade call, got:\n{out_no_adapter}"
        );

        // A non-streaming adapter for the same function must not trigger the
        // instance-receiver path either.
        let mut non_streaming = streaming_owner_adapter("do_thing", "Engine");
        non_streaming.pattern = AdapterPattern::SyncFunction;
        let out_sync = render(call.clone(), vec![non_streaming]);
        assert!(
            out_sync.contains("val result = Facade.doThing(handle, \"https://example.com\")"),
            "a non-streaming adapter must not switch to an instance receiver, got:\n{out_sync}"
        );

        // A streaming adapter with no owner_type must not trigger it either.
        let mut streaming_no_owner = streaming_owner_adapter("do_thing", "Engine");
        streaming_no_owner.owner_type = None;
        let out_no_owner = render(call, vec![streaming_no_owner]);
        assert!(
            out_no_owner.contains("val result = Facade.doThing(handle, \"https://example.com\")"),
            "a streaming adapter without owner_type must not switch to an instance receiver, got:\n{out_no_owner}"
        );
    }
}
