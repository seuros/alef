use heck::{ToLowerCamelCase, ToUpperCamelCase};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureEnv};

use super::args::{KotlinArgsContext, build_args_and_setup};

/// `build_args_and_setup` is shared with the e2e test emitter, whose generated test
/// class declares a companion-object `MAPPER` constant that its output references.
/// A docs snippet is a standalone `fun main()` with no such companion object, and no
/// binding exposes a public `MAPPER`, so every reference has to be rebound onto the
/// local `mapper` the snippet template declares for itself. This applies to the
/// argument list as much as to the setup lines: a typed-array argument is emitted
/// inline as `listOf(MAPPER.readValue(...), ...)` and never reaches `setup_lines`.
const TEST_CLASS_MAPPER_REFERENCE: &str = "MAPPER.";
const SNIPPET_MAPPER_REFERENCE: &str = "mapper.";

fn rebind_mapper_references(source: &str) -> String {
    source.replace(TEST_CLASS_MAPPER_REFERENCE, SNIPPET_MAPPER_REFERENCE)
}

pub(crate) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    kotlin_android_style: bool,
) -> anyhow::Result<String> {
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
        .rsplit('.')
        .next()
        .unwrap_or(&config.name)
        .to_upper_camel_case();
    let function_name = overrides
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_lower_camel_case();
    let options_type = recipe
        .options_type
        .or_else(|| recipe.compatible_options_type(&["kotlin", "kotlin_android", "java", "csharp"]));

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on
    // the owner handle (`engine.streamItems(req)`), not as static facade
    // methods — mirrors the test-generation path in test_method.rs and the
    // Java e2e backend. The docs snippet must show the same real call shape
    // callers will actually write, so it is wrong here for the same reason it
    // was wrong in the generated tests.
    let adapter_lookup_name = call.core_lookup_name(lang);
    let adapter = adapter_lookup_name
        .as_deref()
        .and_then(|name| config.adapters.iter().find(|a| a.name == name));
    let is_streaming_owner_adapter = adapter.is_some_and(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming) && a.owner_type.is_some()
    });
    let streaming_owner_handle: Option<String> = if is_streaming_owner_adapter {
        recipe
            .args
            .iter()
            .find(|a| a.arg_type == "handle")
            .map(|a| a.name.clone())
    } else {
        None
    };
    // The adapter-declared request parameter (`[[crates.adapters.params]]`), if any.
    // Mirrors `test_method.rs::streaming_request`: for an owner_type streaming
    // adapter the facade signature takes exactly this declared param, built from
    // the whole fixture input, not from any per-arg ArgMapping the fixture's
    // `call.args` might otherwise declare (those describe the flat-call shape a
    // non-owner_type adapter would use). ~keep
    let streaming_request = adapter.and_then(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming)
            .then(|| a.params.first())
            .flatten()
    });
    // When the request is built from the adapter param below, drop every non-handle
    // ArgMapping before building setup/args — otherwise `build_args_and_setup` would
    // bind and pass those fields a second time, and (per the fallback path) the
    // resulting call would reference the un-rebuilt request as a raw string/JSON
    // fragment instead of the required typed request object. ~keep
    let call_args: std::borrow::Cow<'_, [crate::e2e::config::ArgMapping]> =
        if streaming_owner_handle.is_some() && streaming_request.is_some() {
            std::borrow::Cow::Owned(recipe.args.iter().filter(|a| a.arg_type == "handle").cloned().collect())
        } else {
            std::borrow::Cow::Borrowed(recipe.args)
        };

    let (setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        &call_args,
        KotlinArgsContext {
            fixture,
            class_name: &class_name,
            options_type,
            fixture_id: &fixture.id,
            kotlin_android_style,
            config,
            type_defs,
            owner_handle_is_receiver: streaming_owner_handle.is_some(),
        },
    )?;
    let mut setup_lines = setup_lines
        .into_iter()
        .map(|line| rebind_mapper_references(&line))
        .collect::<Vec<_>>();
    // Build and bind the declared request object from the fixture input (minus the
    // handle's own fields) — the call target is the owner handle instance, and its
    // method takes this single typed parameter. Without this, `args` from
    // `build_args_and_setup` above is empty (the handle is the receiver, not a
    // positional argument), so the rendered call would be missing its required
    // argument entirely. Mirrors `test_method.rs`'s identical construction for the
    // generated JUnit test. ~keep
    if streaming_owner_handle.is_some()
        && let Some(request) = streaming_request
    {
        let request_name = request.name.to_lower_camel_case();
        let request_type = request.ty.rsplit("::").next().unwrap_or(&request.ty);
        let mut request_input = fixture.input.clone();
        if let Some(object) = request_input.as_object_mut() {
            for handle in recipe.args.iter().filter(|arg| arg.arg_type == "handle") {
                let field = handle.field.strip_prefix("input.").unwrap_or(&handle.field);
                object.remove(field);
            }
        }
        let normalized = crate::e2e::codegen::transform_json_keys_for_language(&request_input, "snake_case");
        let request_json = serde_json::to_string(&normalized).unwrap_or_default();
        // A full literal expression -- quoted, or `+`-chunked when `request_json` is long
        // enough to threaten the JVM's 65535-byte constant cap -- not just escaped content.
        // See `values::kotlin_string_literal`.
        let literal = super::values::kotlin_string_literal(&request_json);
        if crate::e2e::codegen::value_contains_mock_url_placeholder(&normalized) {
            let env_key = crate::e2e::codegen::mock_url_env_key(&fixture.id);
            setup_lines.push(format!(
                "val {request_name}Json = {literal}.replace(\"{}\", System.getProperty(\"mockServer.{}\", System.getenv(\"{env_key}\") ?: \"\"))",
                crate::e2e::escape::escape_kotlin(crate::e2e::codegen::MOCK_URL_PLACEHOLDER),
                fixture.id,
            ));
            setup_lines.push(format!(
                "val {request_name} = mapper.readValue({request_name}Json, {request_type}::class.java)"
            ));
        } else {
            setup_lines.push(format!(
                "val {request_name} = mapper.readValue({literal}, {request_type}::class.java)"
            ));
        }
        args = request_name;
    }
    if let Some(visitor) = &fixture.visitor
        && let Some(visitor_args) =
            super::visitor::attach_visitor(&mut setup_lines, &args, visitor, config, type_defs, enums)
    {
        args = visitor_args;
    }
    if !recipe.extra_args.is_empty() {
        args = if args.is_empty() {
            recipe.extra_args.join(", ")
        } else {
            format!("{args}, {}", recipe.extra_args.join(", "))
        };
    }
    args = rebind_mapper_references(&args);
    let client_factory = overrides.and_then(|value| value.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|value| value.client_factory.as_deref())
    });
    let needs_mapper = args.contains(SNIPPET_MAPPER_REFERENCE)
        || setup_lines.iter().any(|line| line.contains(SNIPPET_MAPPER_REFERENCE));
    let is_async = client_factory.is_some() || call.r#async;
    let package_name = if kotlin_android_style {
        config
            .kotlin_android
            .as_ref()
            .and_then(|value| value.package.clone())
            .unwrap_or_else(|| config.kotlin_package())
    } else {
        config.kotlin_package()
    };
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let api_key_var = FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
    let base_url = crate::e2e::codegen::client_factory::docs_base_url(fixture.docs_client())
        .map(crate::e2e::escape::escape_kotlin);

    // The template renders the call as `{{ class_name }}.{{ function_name }}(...)`
    // (or, with `client_factory`, constructs a client via
    // `{{ class_name }}.{{ client_factory }}(...)` first and then calls
    // `client.{{ function_name }}(...)`). For a flat-call streaming owner_type
    // adapter, substitute the owner handle's variable name for `class_name` so
    // the call renders as the real instance-method invocation. `client_factory`
    // fixtures already dispatch on `client`, not `class_name`, so this
    // substitution is scoped to the flat-call path where it actually changes
    // the rendered call target.
    let call_target_class_name = if client_factory.is_none() {
        streaming_owner_handle.unwrap_or(class_name)
    } else {
        class_name
    };

    let presentation = crate::e2e::codegen::presentation::resolve(fixture, e2e_config, lang, type_defs);
    let result_var = call.effective_result_var();
    Ok(crate::e2e::template_env::render(
        "kotlin/snippet_body.jinja",
        minijinja::context! {
            package_name => package_name,
            needs_mapper => needs_mapper,
            setup_lines => setup_lines,
            client_factory => client_factory.map(ToLowerCamelCase::to_lower_camel_case),
            class_name => call_target_class_name,
            function_name => function_name,
            args => args,
            result_var => result_var,
            returns_void => call.returns_void,
            is_async => is_async,
            fixture_id => fixture.id,
            expects_error => expects_error,
            api_key_var => api_key_var,
            presentation => presentation,
            base_url => base_url,
        },
    ))
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

    const BYTES_ELEMENT: &str = r#"mapper.readValue("{\"kind\":\"bytes\"}", ExtractInput::class.java)"#;
    const URI_ELEMENT: &str = r#"mapper.readValue("{\"kind\":\"uri\"}", ExtractInput::class.java)"#;

    fn line_containing<'a>(body: &'a str, needle: &str) -> &'a str {
        body.lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no line contains {needle} in:\n{body}"))
            .trim()
    }

    /// A batch call whose typed-array argument is materialised inline in the call
    /// expression rather than in a setup line.
    fn batch_call() -> CallConfig {
        CallConfig {
            function: "extract_batch".into(),
            result_var: "result".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "inputs".into(),
                field: "inputs".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: Some("ExtractInput".into()),
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        }
    }

    fn batch_fixture() -> Fixture {
        Fixture {
            id: "extract_batch_bytes_happy".into(),
            description: "Extract several documents in one batch".into(),
            input: serde_json::json!({"inputs": [{"kind": "bytes"}, {"kind": "uri"}]}),
            ..Fixture::default()
        }
    }

    fn batch_snippet(kotlin_android_style: bool) -> String {
        let config = ResolvedCrateConfig {
            name: "xberg".into(),
            ..ResolvedCrateConfig::default()
        };
        render_snippet_body(
            &batch_fixture(),
            &E2eConfig {
                call: batch_call(),
                ..E2eConfig::default()
            },
            &config,
            &[],
            &[],
            kotlin_android_style,
        )
        .expect("snippet renders")
    }

    #[test]
    fn android_batch_snippet_binds_list_elements_to_the_locally_declared_mapper() {
        let body = batch_snippet(true);

        assert!(
            !body.contains("MAPPER"),
            "the snippet must not reference the test class's private MAPPER, got:\n{body}"
        );
        assert!(
            body.contains("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper"),
            "{body}"
        );
        assert!(body.contains("    val mapper = jacksonObjectMapper()"), "{body}");
        assert_eq!(
            line_containing(&body, "extractBatch"),
            format!("val result = Xberg.extractBatch(listOf({BYTES_ELEMENT}, {URI_ELEMENT}))")
        );
    }

    #[test]
    fn jvm_batch_snippet_binds_list_elements_to_the_locally_declared_mapper() {
        let body = batch_snippet(false);

        assert!(
            !body.contains("MAPPER"),
            "the snippet must not reference the test class's private MAPPER, got:\n{body}"
        );
        assert!(
            body.contains("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper"),
            "{body}"
        );
        assert!(body.contains("    val mapper = jacksonObjectMapper()"), "{body}");
        assert_eq!(
            line_containing(&body, "extractBatch"),
            format!("val result = Xberg.extractBatch(listOf({BYTES_ELEMENT}, {URI_ELEMENT}))")
        );
    }

    #[test]
    fn documented_presentation_binds_the_result_and_reads_the_shown_fields() {
        let documented: Fixture = serde_json::from_value(serde_json::json!({
            "id": "present_items", "description": "Present returned items", "input": null,
            "docs": {"topic": "guides", "presentation": {"operations": [
                {"op": "show", "path": "summary", "display": true},
                {"op": "iterate", "path": "items", "item": "item", "fields": ["label"]}
            ]}}
        }))
        .expect("fixture");
        let e2e = E2eConfig {
            call: CallConfig {
                function: "process".into(),
                result_var: "result".into(),
                ..CallConfig::default()
            },
            result_fields: ["summary".to_string(), "items".to_string()].into_iter().collect(),
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let body = render_snippet_body(&documented, &e2e, &config, &[], &[], false).expect("snippet renders");

        assert!(body.contains("val result = Sample.process()"), "{body}");
        assert!(body.contains("println(result.summary())"), "{body}");
        assert!(body.contains("for (item in result.items()) {"), "{body}");
        assert!(body.contains("println(item.label())"), "{body}");
        assert!(
            !body.contains("println(result)"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
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
            &[],
            false,
        )
        .expect("snippet renders");

        assert!(body.contains("DocumentApi.loadDocument()"));
        assert!(body.contains("println(document)"), "{body}");
        assert!(body.contains("fun main()"));
        assert!(!body.contains("@Test"));
        assert!(!body.contains("assert"));
    }

    /// Pins that a `client_factory` docs snippet reads the credential from the
    /// environment and never points the reader at the e2e mock server: no
    /// `MOCK_SERVER` env var, no `mockServer` system property (both used by the
    /// generated JUnit test's mock-mode branch in `test_method.rs`), no
    /// `/fixtures/<id>` route, and no inlined `"test-key"` credential.
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
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
            &[],
            false,
        )
        .expect("snippet renders");

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(!body.contains("mockServer"), "mock-server property leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            body.contains("System.getenv(\"API_KEY\")"),
            "credential is not read from the environment:\n{body}"
        );
        assert!(
            body.contains("createClient(apiKey = apiKey)"),
            "an unconfigured project must construct the client without a mock base URL:\n{body}"
        );
    }

    fn client_release_snippet(expects_error: bool) -> String {
        let mut fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rated limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        if expects_error {
            fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
        }
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &config,
            &[],
            &[],
            false,
        )
        .expect("snippet renders")
    }

    /// The generated Kotlin client wrapper implements `AutoCloseable` and delegates `close()` to
    /// the underlying Java facade (`client_class_header.jinja` / `client_close_method.jinja` in
    /// the Kotlin backend), so `kotlin.use` is the correct release for a client a docs snippet
    /// constructs — it is the stdlib idiom for exactly this type, and it closes on both normal
    /// return and a thrown exception, unlike a bare trailing `client.close()`. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_in_a_use_block() {
        let body = client_release_snippet(false);

        assert!(
            body.contains("Sample.createClient(apiKey = apiKey).use { client -> client.chat() }"),
            "the client must be released via a `use` block around the call:\n{body}"
        );
        assert!(
            !body.contains("client.close()"),
            "no bare close() call must remain:\n{body}"
        );
    }

    /// The error-path half of `client_factory_snippet_releases_the_client_in_a_use_block`.
    /// `kotlin.use` is implemented as try/finally internally, so it releases the client
    /// whether `client.chat(...)` returns or throws — unlike the pre-existing template, whose
    /// `client.close()` sat on its own line after the call and was skipped whenever the call
    /// itself threw, leaking the client on every failed request. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_on_the_error_path() {
        let body = client_release_snippet(true);

        let try_open = body
            .find("    try {")
            .expect("expects-error snippet still opens a try block");
        let use_block = body
            .find("Sample.createClient(apiKey = apiKey).use { client -> client.chat() }")
            .expect("client construction moves inside the try, unchanged in shape");
        let catch_clause = body.find("catch (error: Exception)").expect("catch clause present");
        assert!(
            try_open < use_block && use_block < catch_clause,
            "the use block must sit inside the try that the catch closes:\n{body}"
        );
        assert!(
            !body.contains("client.close()"),
            "no bare close() call must remain:\n{body}"
        );
    }

    /// Negative control for the two tests above, and the pin that keeps this change scoped: a
    /// fixture with no `client_factory` constructs no client, so its snippet must be byte-for-byte
    /// what it was — no `.use {`, no `close()`, just the plain call. A change that wraps every
    /// snippet's call in a `use` block unconditionally would fail here.
    #[test]
    fn snippet_without_a_client_factory_is_unchanged() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[], false).expect("snippet renders");

        assert!(
            !body.contains(".use {"),
            "a snippet that constructs no client must emit no use block:\n{body}"
        );
        assert!(
            !body.contains("close()"),
            "a snippet that constructs no client must emit no close call:\n{body}"
        );
        assert!(
            body.contains("    val result = Sample.process()"),
            "the plain call must be unchanged:\n{body}"
        );
    }

    /// Companion of `client_factory_snippet_never_points_the_reader_at_the_mock_server`:
    /// a fixture whose `docs.client.base_url` names an endpoint must show that endpoint
    /// as a named `baseUrl` argument on the client-construction call.
    #[test]
    fn a_snippet_renders_the_base_url_the_fixture_documents() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "custom_base_url",
            "description": "Custom base URL",
            "input": null,
            "docs": {
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            }
        }))
        .expect("fixture");
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
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
            &[],
            false,
        )
        .expect("snippet renders");

        assert!(
            body.contains("createClient(apiKey = apiKey, baseUrl = \"https://llm.internal.example.com/v1\")"),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
        );
    }

    /// Negative control for the base-URL wiring above: a fixture with no `docs.client`
    /// must keep rendering the bare call, unchanged by the new optional argument.
    #[test]
    fn a_fixture_without_a_docs_client_keeps_the_bare_client_construction_call() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
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
            &[],
            false,
        )
        .expect("snippet renders");

        assert!(
            body.contains("createClient(apiKey = apiKey)"),
            "an unconfigured project must construct the client without a base URL:\n{body}"
        );
        assert!(
            !body.contains("baseUrl"),
            "no docs client must mean no baseUrl argument:\n{body}"
        );
    }

    #[test]
    fn android_snippet_uses_simple_class_name_and_sync_main() {
        let mut call = CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin_android".into(),
            CallOverride {
                class: Some("dev.sample.SampleApi".into()),
                ..CallOverride::default()
            },
        );
        let mut config = ResolvedCrateConfig::default();
        config.kotlin_android = Some(crate::core::config::KotlinAndroidConfig {
            package: Some("dev.sample".into()),
            ..Default::default()
        });

        let body = render_snippet_body(
            &fixture(),
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &config,
            &[],
            &[],
            true,
        )
        .expect("snippet renders");

        assert!(body.contains("import dev.sample.*"), "{body}");
        assert!(body.contains("SampleApi.convert()"), "{body}");
        assert!(body.contains("fun main() {"), "{body}");
        assert!(!body.contains("runBlocking"), "{body}");
        assert!(!body.contains("DevSampleSampleApi"), "{body}");
    }

    #[test]
    fn android_snippet_declares_typed_config_without_coroutine_wrapper() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "process_source",
            "description": "Process source",
            "input": {
                "source_code": "fn main() {}",
                "config": {"language": "rust"}
            }
        }))
        .expect("fixture parses");
        let mut call = CallConfig {
            function: "process".into(),
            result_var: "result".into(),
            args: vec![
                crate::e2e::config::ArgMapping {
                    name: "source".into(),
                    field: "source_code".into(),
                    arg_type: "string".into(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
                crate::e2e::config::ArgMapping {
                    name: "config".into(),
                    field: "config".into(),
                    arg_type: "json_object".into(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
            ],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                options_type: Some("ProcessConfig".into()),
                ..Default::default()
            },
        );

        let config = ResolvedCrateConfig {
            name: "sample_api".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &config,
            &[],
            &[],
            true,
        )
        .expect("snippet renders");

        assert!(body.contains("val config = mapper.readValue"), "{body}");
        assert!(body.contains("ProcessConfig::class.java"), "{body}");
        assert!(body.contains("SampleApi.process(\"fn main() {}\", config)"), "{body}");
        assert!(!body.contains("runBlocking"), "{body}");
    }

    #[test]
    fn snippet_renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], false)
            .expect("snippet renders");

        assert!(body.contains("catch (error: Exception)"), "{body}");
        assert!(body.contains("error::class.simpleName"), "{body}");
        assert!(!body.contains("AssertionError"), "{body}");
    }

    /// Regression twin of the Java fix (alef task #180): Kotlin compiles to the same JVM
    /// bytecode as Java and inherits the identical 65535-byte `CONSTANT_Utf8` cap on a single
    /// string literal. A fixture whose `json_object` field carries a value long enough to
    /// threaten that cap must never render as a single Kotlin string literal above the limit.
    /// Neutral synthetic payload (`project-agnostic-codegen`): not any real consumer's fixture.
    /// ~keep
    #[test]
    fn a_doc_snippet_never_emits_a_single_kotlin_literal_over_the_jvm_constant_cap() {
        let oversized_payload = "abcdefghij".repeat(10_000); // 100,000 bytes
        let fixture = Fixture {
            id: "large_payload".into(),
            description: "Process a large payload".into(),
            input: serde_json::json!({"content": oversized_payload}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "options".into(),
                field: "content".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                options_type: Some("PayloadOptions".into()),
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
            &[],
            false,
        )
        .expect("snippet renders");

        assert!(
            body.contains(&oversized_payload[..100]),
            "the snippet must still contain the fixture content, just not as one literal:\n{}",
            &body[..body.len().min(500)]
        );
        for segment in body.split('"') {
            assert!(
                segment.len() <= 65_535,
                "a generated Kotlin doc snippet must never contain a single quoted-literal \
                 segment above the JVM's 65535-byte CONSTANT_Utf8 cap: got {} bytes",
                segment.len()
            );
        }
    }

    #[test]
    fn snippet_reads_nested_typed_dto_files() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "document_input",
            "description": "Read a document",
            "input": {"request": {"content": "ignored"}},
            "assertions": [],
            "docs": {
                "topic": "documents",
                "presentation": {"files": [{"field": "/request/content", "path": "document.pdf"}]}
            }
        }))
        .expect("fixture");
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "request".into(),
                field: "request".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                options_type: Some("DocumentRequest".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture.docs_call_fixture(),
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            false,
        )
        .expect("snippet renders");

        assert!(
            body.contains("Files.readAllBytes(java.nio.file.Path.of(\"document.pdf\"))"),
            "{body}"
        );
        assert!(body.contains("Base64.getEncoder().encodeToString"), "{body}");
        assert!(body.contains("DocumentRequest::class.java"), "{body}");
    }

    #[test]
    fn snippet_deserializes_generic_typed_dto_without_file_metadata() {
        let fixture = Fixture {
            id: "document_input".into(),
            description: "Process a document".into(),
            input: serde_json::json!({"kind": "uri", "uri": "document.txt"}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "input".into(),
                field: "input".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: Some("DocumentInput".into()),
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin_android".into(),
            CallOverride {
                options_type: Some("ExtractionConfig".into()),
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
            &[],
            true,
        )
        .expect("snippet renders");

        assert!(body.contains("val input = mapper.readValue("), "{body}");
        assert!(body.contains("DocumentInput::class.java"), "{body}");
        assert!(!body.contains("ExtractionConfig::class.java"), "{body}");
        assert!(body.contains(".process(input)"), "{body}");
        assert!(body.contains("jacksonObjectMapper"), "{body}");
    }

    #[test]
    fn snippet_uses_nested_centralized_wire_names() {
        let fixture = Fixture {
            id: "document_input".into(),
            description: "Process a document".into(),
            input: serde_json::json!({"request_id": "one", "details": {"page_count": 2}}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "input".into(),
                field: "input".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin_android".into(),
            CallOverride {
                options_type: Some("DocumentInput".into()),
                ..CallOverride::default()
            },
        );
        let type_defs = vec![
            crate::core::ir::TypeDef {
                name: "DocumentInput".into(),
                fields: vec![
                    crate::core::ir::FieldDef {
                        name: "request_id".into(),
                        serde_rename: Some("request-id".into()),
                        ..Default::default()
                    },
                    crate::core::ir::FieldDef {
                        name: "details".into(),
                        ty: crate::core::ir::TypeRef::Named("DocumentDetails".into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            crate::core::ir::TypeDef {
                name: "DocumentDetails".into(),
                serde_rename_all: Some("camelCase".into()),
                fields: vec![crate::core::ir::FieldDef {
                    name: "page_count".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ];

        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &type_defs,
            &[],
            true,
        )
        .expect("snippet renders");

        assert!(body.contains(r#"\"request-id\":\"one\""#), "{body}");
        assert!(body.contains(r#"\"pageCount\":2"#), "{body}");
        assert!(body.contains("val mapper = jacksonObjectMapper()"), "{body}");
    }

    /// Synthetic streaming `owner_type` adapter whose facade signature takes exactly
    /// the declared `request` param, matching the shape declared by
    /// `[[crates.adapters]]` in `alef.toml`. Mirrors
    /// `test_method::tests::streaming_owner_adapter`.
    fn streaming_owner_adapter(function_name: &str, owner_type: &str) -> crate::core::config::extras::AdapterConfig {
        crate::core::config::extras::AdapterConfig {
            name: function_name.to_string(),
            pattern: crate::core::config::extras::AdapterPattern::Streaming,
            core_path: format!("test_core::{function_name}"),
            params: vec![crate::core::config::extras::AdapterParam {
                name: "request".to_string(),
                ty: "sample::StreamRequest".to_string(),
                optional: false,
            }],
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

    fn streaming_owner_call() -> CallConfig {
        CallConfig {
            function: "stream_items".into(),
            result_var: "result".into(),
            args: vec![
                crate::e2e::config::ArgMapping {
                    name: "handle".into(),
                    field: "input.handle".into(),
                    arg_type: "handle".into(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
                crate::e2e::config::ArgMapping {
                    name: "url".into(),
                    field: "input.url".into(),
                    arg_type: "string".into(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
            ],
            ..CallConfig::default()
        }
    }

    fn streaming_owner_fixture() -> Fixture {
        Fixture {
            id: "stream_basic".into(),
            description: "Stream items".into(),
            input: serde_json::json!({ "handle": {}, "url": "https://example.com" }),
            ..Fixture::default()
        }
    }

    /// Regression for #149: an owner_type streaming adapter's declared request
    /// param (`[[crates.adapters.params]]`) must be built from the fixture input
    /// and bound to a local `val request = ...` before the call references it —
    /// mirroring `test_method.rs`'s identical construction for the generated
    /// JUnit test. Before this fix `render_snippet_body` never resolved the
    /// adapter's `streaming_request` at all: the handle-typed arg was still
    /// treated as the call receiver (so it never reached `args`), but nothing
    /// rebuilt or bound the request in its place, leaving the call either
    /// missing its required argument or passing an un-rebuilt raw value instead
    /// of the declared `request` identifier this test pins.
    #[test]
    fn kotlin_snippet_binds_the_declared_request_before_the_call() {
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            adapters: vec![streaming_owner_adapter("stream_items", "Engine")],
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(
            &streaming_owner_fixture(),
            &E2eConfig {
                call: streaming_owner_call(),
                ..E2eConfig::default()
            },
            &config,
            &[],
            &[],
            false,
        )
        .expect("snippet renders");

        assert_eq!(
            line_containing(&body, "val request ="),
            r#"val request = mapper.readValue("{\"url\":\"https://example.com\"}", StreamRequest::class.java)"#
        );
        assert_eq!(
            line_containing(&body, "val result ="),
            "val result = handle.streamItems(request)"
        );
        assert!(
            !body.contains("\\\"handle\\\""),
            "owner handle config must not leak into the request JSON:\n{body}"
        );
        assert!(
            !body.contains("streamItems(\"https://example.com\")"),
            "the raw url must not be passed positionally in place of the declared request:\n{body}"
        );
    }

    #[test]
    fn kotlin_android_snippet_binds_the_declared_request_before_the_call() {
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            adapters: vec![streaming_owner_adapter("stream_items", "Engine")],
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(
            &streaming_owner_fixture(),
            &E2eConfig {
                call: streaming_owner_call(),
                ..E2eConfig::default()
            },
            &config,
            &[],
            &[],
            true,
        )
        .expect("snippet renders");

        assert_eq!(
            line_containing(&body, "val request ="),
            r#"val request = mapper.readValue("{\"url\":\"https://example.com\"}", StreamRequest::class.java)"#
        );
        assert_eq!(
            line_containing(&body, "val result ="),
            "val result = handle.streamItems(request)"
        );
        assert!(
            !body.contains("\\\"handle\\\""),
            "owner handle config must not leak into the request JSON:\n{body}"
        );
    }
}
