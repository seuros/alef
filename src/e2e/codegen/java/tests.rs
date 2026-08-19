use super::args::{JavaArgsContext, build_args_and_setup};
use super::visitor::{apply_java_visitor_arg, java_visitor_binding};
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{ArgMapping, CallConfig, E2eConfig, SelectWhen};
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;

fn make_fixture_with_input(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Test that resolve_call_for_fixture correctly routes to batchScrape
/// when input has batch_urls and select_when condition matches.
#[test]
fn test_java_select_when_routes_to_batch_scrape() {
    let mut calls = HashMap::new();
    calls.insert(
        "batch_scrape".to_string(),
        CallConfig {
            function: "batchScrape".to_string(),
            module: "com.example.sample_stream".to_string(),
            select_when: Some(SelectWhen {
                input_has: Some("batch_urls".to_string()),
                ..Default::default()
            }),
            ..CallConfig::default()
        },
    );

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "scrape".to_string(),
            module: "com.example.sample_stream".to_string(),
            ..CallConfig::default()
        },
        calls,
        ..E2eConfig::default()
    };

    // Fixture with batch_urls but no explicit call field should route to batch_scrape
    let fixture = make_fixture_with_input("batch_empty_urls", serde_json::json!({ "batch_urls": [] }));

    let resolved_call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    assert_eq!(resolved_call.function, "batchScrape");

    // Fixture without batch_urls should fall back to default scrape
    let fixture_no_batch =
        make_fixture_with_input("simple_scrape", serde_json::json!({ "url": "https://example.com" }));
    let resolved_default = e2e_config.resolve_call_for_fixture(
        fixture_no_batch.call.as_deref(),
        &fixture_no_batch.id,
        &fixture_no_batch.resolved_category(),
        &fixture_no_batch.tags,
        &fixture_no_batch.input,
    );
    assert_eq!(resolved_default.function, "scrape");
}

#[test]
fn handle_config_deserialization_uses_resolved_options_type() {
    let args = vec![ArgMapping {
        name: "session".to_string(),
        field: "input.config".to_string(),
        arg_type: "handle".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = make_fixture_with_input("session_fixture", serde_json::json!({ "config": { "limit": 3 } }));
    let mut teardown = String::new();
    let (setup, args_str) = build_args_and_setup(
        &fixture.input,
        &args,
        JavaArgsContext {
            class_name: "Sample",
            options_type: Some("SessionConfig"),
            fixture: &fixture,
            adapter_request_type: None,
            owner_handle_is_receiver: false,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
            enums: &[],
            target_params: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
            teardown_block: &mut teardown,
        },
    );

    let rendered = setup.join("\n");
    assert_eq!(args_str, "session");
    assert!(rendered.contains("MAPPER.readValue(\"{\\\"limit\\\":3}\", SessionConfig.class)"));
    assert!(rendered.contains("Sample.createSession(sessionConfig)"));
    assert!(!rendered.contains("CrawlConfig"));
}

#[test]
fn java_visitor_arg_uses_trait_bridge_options_metadata() {
    use crate::core::config::{BridgeBinding, TraitBridgeConfig};

    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "Renderer".to_string(),
            type_alias: Some("RenderHandle".to_string()),
            param_name: Some("renderer".to_string()),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some("RenderOptions".to_string()),
            options_field: Some("callback".to_string()),
            context_type: Some("RenderContext".to_string()),
            result_type: Some("RenderDecision".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let binding = java_visitor_binding(&config, &[], None, None).expect("visitor binding");
    assert_eq!(binding.options_type, "RenderOptions");
    assert_eq!(binding.options_field, "callback");
    assert_eq!(binding.trait_type, "Renderer");
    assert_eq!(binding.context_type, "RenderContext");
    assert_eq!(binding.result_type, "RenderDecision");

    let args = apply_java_visitor_arg(&mut Vec::new(), "html, null", &[], "visitor", &binding);
    assert_eq!(args, "html, RenderOptions.builder().withCallback(visitor).build()");
    assert!(!args.contains("DefaultOptions"));
}

#[test]
fn test_java_harness_main_uses_default_port_not_random_probe() {
    use super::project::render_harness_main;
    use crate::e2e::config::HarnessConfig;
    use crate::e2e::fixture::FixtureGroup;

    let e2e_config = E2eConfig {
        harness: HarnessConfig {
            host: "127.0.0.1".to_string(),
            port: 8000,
            app_class: Some("App".to_string()),
            run_method: Some("run".to_string()),
            register_method: Some("registerAppRoute".to_string()),
            response_body_field: "body".to_string(),
            ..Default::default()
        },
        call: CallConfig::default(),
        ..E2eConfig::default()
    };

    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![],
    }];

    let rendered = render_harness_main(&e2e_config, &groups, "dev.example", "dev.example.app");

    // Verify that the rendered output does NOT contain ServerSocket(0) probe
    assert!(
        !rendered.contains("ServerSocket(0"),
        "HarnessMain should not probe for random port via ServerSocket(0)"
    );

    // Verify that when SUT_URL is unset, it falls back to the default port
    assert!(
        rendered.contains("effectivePort = 8000"),
        "HarnessMain should set effectivePort to 8000 (alef default) when SUT_URL is unset"
    );

    // Verify that the rendered output uses the default_port variable in the SUT_URL parsing fallback
    assert!(
        rendered.contains("effectivePort = uri.getPort() > 0 ? uri.getPort() : 8000"),
        "HarnessMain should use default_port in SUT_URL URI parsing"
    );
}

#[test]
fn test_java_env_entries_empty_produces_no_init_env() {
    use super::test_file::render_test_file;

    let fixture = make_fixture_with_input("basic", serde_json::json!({}));
    let fixtures = vec![&fixture];

    let e2e_config = E2eConfig {
        env: HashMap::new(),
        call: CallConfig::default(),
        ..E2eConfig::default()
    };

    let rendered = render_test_file(
        "test",
        &fixtures,
        "TestClass",
        "testFunc",
        "com.example",
        "com.example.binding",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        true,
        &[],
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
        &[],
        false,
    );

    // Should not contain initEnv when env is empty
    assert!(
        !rendered.contains("initEnv"),
        "empty env should not emit initEnv method"
    );
}

#[test]
fn test_java_env_entries_renders_sorted_system_properties() {
    use super::test_file::render_test_file;

    let fixture = make_fixture_with_input("basic", serde_json::json!({}));
    let fixtures = vec![&fixture];

    let mut env = HashMap::new();
    env.insert("ZEBRA_FLAG".to_string(), "zebra_value".to_string());
    env.insert("ALPHA_FLAG".to_string(), "alpha_value".to_string());
    env.insert("BETA_FLAG".to_string(), "beta_value".to_string());

    let e2e_config = E2eConfig {
        env,
        call: CallConfig::default(),
        ..E2eConfig::default()
    };

    let rendered = render_test_file(
        "test",
        &fixtures,
        "TestClass",
        "testFunc",
        "com.example",
        "com.example.binding",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        true,
        &[],
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
        &[],
        false,
    );

    // Should contain initEnv method
    assert!(rendered.contains("static void initEnv()"), "should emit initEnv method");

    // Should contain each property with null check
    assert!(
        rendered.contains("System.getProperty(\"ALPHA_FLAG\")"),
        "should check ALPHA_FLAG"
    );
    assert!(
        rendered.contains("System.setProperty(\"ALPHA_FLAG\", \"alpha_value\")"),
        "should set ALPHA_FLAG"
    );
    assert!(
        rendered.contains("System.getProperty(\"BETA_FLAG\")"),
        "should check BETA_FLAG"
    );
    assert!(
        rendered.contains("System.setProperty(\"BETA_FLAG\", \"beta_value\")"),
        "should set BETA_FLAG"
    );
    assert!(
        rendered.contains("System.getProperty(\"ZEBRA_FLAG\")"),
        "should check ZEBRA_FLAG"
    );
    assert!(
        rendered.contains("System.setProperty(\"ZEBRA_FLAG\", \"zebra_value\")"),
        "should set ZEBRA_FLAG"
    );

    // Verify alphabetical ordering by finding positions
    let alpha_pos = rendered.find("ALPHA_FLAG").expect("ALPHA_FLAG should be present");
    let beta_pos = rendered.find("BETA_FLAG").expect("BETA_FLAG should be present");
    let zebra_pos = rendered.find("ZEBRA_FLAG").expect("ZEBRA_FLAG should be present");
    assert!(
        alpha_pos < beta_pos && beta_pos < zebra_pos,
        "env keys should be sorted alphabetically"
    );
}

#[test]
fn java_fixture_middleware_remaps_cors_allow_to_allowed() {
    use crate::e2e::fixture::{CorsConfig, HttpMiddleware};
    let mw = Some(HttpMiddleware {
        cors: Some(CorsConfig {
            allow_origins: vec!["https://example.com".to_string()],
            allow_methods: vec!["GET".to_string(), "POST".to_string()],
            allow_headers: vec!["Content-Type".to_string()],
            max_age: Some(600),
            allow_credentials: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    let value = super::build_middleware_value(&mw);
    let cors = &value["cors"];
    // Keys must be remapped allow_* -> allowed_* so the java harness can
    // deserialize straight into the binding's CorsConfig.
    assert_eq!(cors["allowed_origins"], serde_json::json!(["https://example.com"]));
    assert_eq!(cors["allowed_methods"], serde_json::json!(["GET", "POST"]));
    assert_eq!(cors["allowed_headers"], serde_json::json!(["Content-Type"]));
    assert_eq!(cors["max_age"], serde_json::json!(600));
    assert_eq!(cors["allow_credentials"], serde_json::json!(true));
    // The unremapped allow_* keys must NOT leak through.
    assert!(
        cors.get("allow_origins").is_none(),
        "legacy allow_origins must not be emitted"
    );
}

#[test]
fn java_fixture_middleware_is_null_without_cors() {
    assert_eq!(super::build_middleware_value(&None), serde_json::Value::Null);
    // Middleware present but no cors -> still Null (harness's middleware.cors is a missing node).
    let mw = Some(crate::e2e::fixture::HttpMiddleware::default());
    assert_eq!(super::build_middleware_value(&mw), serde_json::Value::Null);
}

/// One `ArgMapping`, `arg_type` left at its `"string"` default, used by every test below so the
/// only variable is what the IR declares about the parameter it fills. ~keep
fn default_typed_arg(name: &str) -> ArgMapping {
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

fn args_for(
    args: &[ArgMapping],
    fixture: &Fixture,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    target_params: crate::e2e::codegen::call_ir::TargetParams<'_>,
) -> String {
    let mut teardown = String::new();
    let config = ResolvedCrateConfig::default();
    let (_setup, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        JavaArgsContext {
            class_name: "Sample",
            options_type: None,
            fixture,
            adapter_request_type: None,
            owner_handle_is_receiver: false,
            config: &config,
            type_defs,
            enums,
            target_params,
            teardown_block: &mut teardown,
        },
    );
    args_str
}

/// The defect: a fixture object bound for a DTO-typed parameter used to become a *quoted JSON
/// string literal*, which `javac` rejects. With the declared type resolved it deserializes. ~keep
#[test]
fn an_object_value_for_an_ir_struct_parameter_deserializes_instead_of_stringifying() {
    use crate::core::ir::{ParamDef, TypeDef, TypeRef};
    let args = vec![default_typed_arg("request")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "request": { "prompt": "hi" } }));
    let params = [ParamDef {
        name: "request".to_string(),
        ty: TypeRef::Named("CompletionRequest".to_string()),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "CompletionRequest".to_string(),
        ..TypeDef::default()
    }];
    let rendered = args_for(
        &args,
        &fixture,
        &type_defs,
        &[],
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
    );
    let package = ResolvedCrateConfig::default().java_package();
    assert_eq!(
        rendered,
        format!("{package}.JsonUtil.fromJson(\"{{\\\"prompt\\\":\\\"hi\\\"}}\", {package}.CompletionRequest.class)")
    );
}

/// An enum-typed parameter takes the generated enum's `@JsonCreator fromValue`, not a bare
/// string and not a guessed constant name. ~keep
#[test]
fn a_string_value_for_an_ir_enum_parameter_uses_from_value() {
    use crate::core::ir::{EnumDef, ParamDef, TypeRef};
    let args = vec![default_typed_arg("mode")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "mode": "fast_path" }));
    let params = [ParamDef {
        name: "mode".to_string(),
        ty: TypeRef::Named("Mode".to_string()),
        ..ParamDef::default()
    }];
    let enums = [EnumDef {
        name: "Mode".to_string(),
        ..EnumDef::default()
    }];
    let rendered = args_for(
        &args,
        &fixture,
        &[],
        &enums,
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
    );
    let package = ResolvedCrateConfig::default().java_package();
    assert_eq!(rendered, format!("{package}.Mode.fromValue(\"fast_path\")"));
}

/// The other half of the three-state trade. Identical arg and fixture value, `IrAbsent` instead
/// of `Known`: the pre-seam lowering must survive verbatim, or every IR-less caller (the
/// snippet path, and every test that renders without an IR) regresses silently. ~keep
#[test]
fn the_same_object_value_still_stringifies_when_the_ir_is_absent() {
    let args = vec![default_typed_arg("request")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "request": { "prompt": "hi" } }));
    let rendered = args_for(
        &args,
        &fixture,
        &[],
        &[],
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
    );
    assert_eq!(rendered, "\"{\\\"prompt\\\":\\\"hi\\\"}\"");
}

/// A declared type absent from both IR registries is not a licence to invent a deserializer:
/// it may be a newtype the Java binding flattens to a `String`. ~keep
#[test]
fn a_declared_type_unknown_to_the_ir_keeps_the_existing_lowering() {
    use crate::core::ir::{ParamDef, TypeRef};
    let args = vec![default_typed_arg("request")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "request": "hi" }));
    let params = [ParamDef {
        name: "request".to_string(),
        ty: TypeRef::Named("PromptText".to_string()),
        ..ParamDef::default()
    }];
    let rendered = args_for(
        &args,
        &fixture,
        &[],
        &[],
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
    );
    assert_eq!(rendered, "\"hi\"");
}

/// An `Optional<T>`/`Vec<T>` parameter wants a wrapper this expression does not build, so the
/// seam must decline rather than unwrap to `T` and trade one compile error for another. ~keep
#[test]
fn a_wrapped_named_parameter_is_left_to_the_existing_lowering() {
    use crate::core::ir::{ParamDef, TypeDef, TypeRef};
    let args = vec![default_typed_arg("request")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "request": { "prompt": "hi" } }));
    let params = [ParamDef {
        name: "request".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Named("CompletionRequest".to_string()))),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "CompletionRequest".to_string(),
        ..TypeDef::default()
    }];
    let rendered = args_for(
        &args,
        &fixture,
        &type_defs,
        &[],
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
    );
    assert_eq!(rendered, "\"{\\\"prompt\\\":\\\"hi\\\"}\"");
}
