use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Context, Result};

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
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let mut fixture_without_assertions = fixture.clone();
    fixture_without_assertions.assertions.clear();
    // Trait-bridge fixtures (e.g. `register_validator: trait bridge`) reference a
    // `_create<Stub>Wrapper()` factory in the call expression below. Dart forbids class
    // definitions inside a function, so — mirroring the full e2e test-file emitter — the
    // stub class and its factory function must be hoisted to module scope, above
    // `main()`. Without this the snippet calls a factory function that is never defined.
    let mut stub_classes = String::new();
    super::stubs::collect_test_stub_classes(
        &mut stub_classes,
        &fixture_without_assertions,
        e2e_config,
        config,
        type_defs,
        enums,
    );
    let stub_classes = stub_classes.trim_end().to_string();
    let bridge_class = config.dart_bridge_class_name();
    let first_class_map = super::values::build_dart_first_class_map(type_defs, enums, e2e_config);
    let mut test_case = String::new();
    super::test_case::render_test_case(
        &mut test_case,
        &fixture_without_assertions,
        super::test_case::DartTestCaseContext {
            e2e_config,
            lang: "dart",
            bridge_class: &bridge_class,
            dart_first_class_map: &first_class_map,
            adapters: &config.adapters,
            config,
            type_defs,
            enums,
            native_typed_dtos: true,
            is_snippet: true,
        },
    );
    let statements = extract_test_statements(&test_case)
        .with_context(|| format!("extracting Dart snippet body for fixture `{}`", fixture.id))?;
    let package = e2e_config
        .resolve_package("dart")
        .and_then(|value| value.name)
        .unwrap_or_else(|| config.dart_pubspec_name());
    let module = config.dart_library_name();
    let bridge_module = format!("{}_bridge_generated", config.name.replace('-', "_"));
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let needs_json = statements
        .iter()
        .any(|statement| statement.contains("jsonDecode(") || statement.contains("jsonEncode("));
    let needs_io =
        expects_error || !call.returns_void || statements.iter().any(|statement| statement.contains("File("));
    // Trait-bridge stubs with `Vec<u8>`/bytes-typed methods (e.g. OcrBackend.processImage)
    // emit `Uint8List`, which requires `dart:typed_data` — mirrors `has_batch_byte_items` in
    // the full e2e test-file emitter (test_file.rs).
    let needs_typed_data = stub_classes.contains("Uint8List");
    Ok(crate::e2e::template_env::render(
        "dart/snippet_body.jinja",
        minijinja::context! {
            package => package, module => module, bridge_module => bridge_module,
            statements => statements, needs_json => needs_json,
            needs_io => needs_io,
            needs_typed_data => needs_typed_data,
            expects_error => expects_error,
            error_type => config.error_type_name(),
            result_var => call.result_var,
            returns_void => call.returns_void,
            stub_classes => stub_classes,
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
        "dart/http_snippet.jinja",
        minijinja::context! {
            method => http.request.method.to_uppercase(),
            path => format!("/fixtures/{}{}", fixture.id, http.request.path),
            headers => headers.iter().map(|(key, value)| minijinja::context! {
                key => super::values::escape_dart(key), value => super::values::escape_dart(value),
            }).collect::<Vec<_>>(),
            body_json => plan.body.as_ref().map(serde_json::to_string).transpose()?,
            raw_body => raw_body,
        },
    ))
}

fn extract_test_statements(rendered: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = rendered.lines().collect();
    let start = lines.iter().position(|line| line.trim_start().starts_with("test("))? + 1;
    let end = lines.iter().rposition(|line| line.trim() == "});")?;
    Some(
        lines[start..end]
            .iter()
            .map(|line| line.strip_prefix("    ").unwrap_or(line).to_string())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_the_native_test_body() {
        let rendered = "  test('sample', () async {\n    final value = await Api.load();\n  });\n";
        assert_eq!(
            extract_test_statements(rendered),
            Some(vec!["final value = await Api.load();".to_string()])
        );
    }

    #[test]
    fn renders_native_call_without_test_harness() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).unwrap();
        assert!(body.contains("loadDocument()"));
        assert!(body.contains("Future<void> main() async"));
        assert!(body.contains("show RustLib"));
        assert!(body.contains("await RustLib.init()"));
        assert!(body.contains("RustLib.dispose()"));
        assert!(!body.contains("test("));
        assert!(!body.contains("expect("));
    }

    // Regression test: 188 of 190 Dart doc snippets failed `dart analyze` with
    // "The function '_fixtureUrl' isn't defined." A `client_factory` call (e.g.
    // `createClient`) makes `render_test_case` emit `final _mockUrl = _fixtureUrl(...)`
    // plus a `baseUrl: _mockUrl` argument — `_fixtureUrl` is only ever defined by the
    // full e2e test-file emitter, never by the standalone snippet emitter. The snippet
    // must strip the mock-URL harness entirely, matching the PHP/Ruby/Go/TypeScript
    // emitters, which all construct their client without a baseUrl override.
    #[test]
    fn snippet_omits_undefined_fixture_url_helper_from_client_factory_call() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "edge_batch_empty_list", "description": "Empty batch list", "input": null
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "create_client".into();
        e2e_config.call.overrides.insert(
            "dart".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("createClient".into()),
                ..Default::default()
            },
        );

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(
            !body.contains("_fixtureUrl"),
            "snippet must not reference the undefined _fixtureUrl helper:\n{body}"
        );
        assert!(
            !body.contains("baseUrl:"),
            "snippet must not pass a mock baseUrl override:\n{body}"
        );
        assert!(
            body.contains(".createClient('test-key');"),
            "snippet must construct the client with only the api key:\n{body}"
        );
    }

    #[test]
    fn snippet_uses_package_reference_and_configured_library_entrypoint() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        e2e.packages.insert(
            "dart".into(),
            crate::core::config::e2e::PackageRef {
                name: Some("sample_harness_dep".into()),
                ..Default::default()
            },
        );
        let mut config = ResolvedCrateConfig::default();
        config.name = "sample-core".into();
        config.dart = Some(crate::core::config::languages::DartConfig {
            lib_name: Some("sample_entrypoint".into()),
            ..Default::default()
        });

        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet");

        assert!(
            body.contains("import 'package:sample_harness_dep/sample_entrypoint.dart';"),
            "{body}"
        );
        assert!(
            body.contains("package:sample_harness_dep/src/sample_core_bridge_generated/frb_generated.dart"),
            "{body}"
        );
        assert!(!body.contains("sample_core.dart"), "{body}");
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

        assert!(body.contains("on Error catch (error)"), "{body}");
        assert!(!body.contains("expected call to fail"), "{body}");
    }

    #[test]
    fn renders_http_request_without_test_harness_assertions() {
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
        assert!(body.contains("HttpClient"));
        assert!(body.contains("/fixtures/create_item/items"));
        assert!(body.contains("request.write(jsonEncode(jsonDecode"));
        assert!(!body.contains("expect("));
    }

    fn make_trait_bridge(trait_name: &str) -> crate::core::config::TraitBridgeConfig {
        crate::core::config::TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            ..Default::default()
        }
    }

    fn make_method(name: &str) -> crate::core::ir::MethodDef {
        crate::core::ir::MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
            is_async: true,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    // Regression test for defect #543/1: a trait-bridge snippet (register_validator,
    // register_ocr_backend, ...) references a `_createTestStub<Fixture>Wrapper()` factory
    // function in its call expression. The full e2e test-file emitter hoists the stub
    // class + factory to module scope via a separate pass (`collect_test_stub_classes`);
    // the doc-snippet emitter must run the same pass, or the emitted snippet calls a
    // function that is never defined. This fixture's call is void-returning to isolate
    // the stub-emission defect from the separate void-binding defect (see the sibling
    // test below) — proving neither test can pass while only the other defect is fixed.
    #[test]
    fn snippet_hoists_test_backend_stub_class_and_factory_above_main() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "register_backend", "description": "register: trait bridge", "input": null
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "registerBackend".into();
        e2e_config.call.returns_void = true;
        e2e_config.call.args.push(crate::e2e::config::ArgMapping {
            name: "backend".into(),
            field: "input.backend".into(),
            arg_type: "test_backend".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some("TestTrait".into()),
        });
        let mut config = ResolvedCrateConfig::default();
        config.trait_bridges.push(make_trait_bridge("TestTrait"));
        let type_defs = [crate::core::ir::TypeDef {
            name: "TestTrait".into(),
            methods: vec![make_method("doWork")],
            ..Default::default()
        }];

        let body = render_snippet_body(&fixture, &e2e_config, &config, &type_defs, &[]).expect("snippet");

        assert!(
            body.contains("class TestStubRegisterBackend extends TestTrait"),
            "stub class must be emitted at module scope:\n{body}"
        );
        assert!(
            body.contains("Future<TestTraitDartImpl> _createTestStubRegisterBackendWrapper()"),
            "factory function must be defined, not just referenced:\n{body}"
        );
        assert!(
            body.contains("await _createTestStubRegisterBackendWrapper()"),
            "call site must invoke the now-defined factory:\n{body}"
        );
        // The stub class must appear before `main()`; Dart forbids local class declarations.
        let class_pos = body.find("class TestStubRegisterBackend").expect("class present");
        let main_pos = body.find("Future<void> main()").expect("main present");
        assert!(class_pos < main_pos, "stub class must precede main():\n{body}");
    }

    // Regression test for defect #543/2: binding the result of a `Future<void>`-returning
    // call (`final result = await voidCall();`) is a Dart compile error even when `result`
    // is never read — the initializer's void value is what's illegal, not an unused
    // variable. This fixture uses a plain (non-trait-bridge) void call with no
    // `test_backend` argument, isolating the void-binding defect from the stub-emission
    // defect covered above.
    #[test]
    fn snippet_does_not_bind_a_void_returning_call_to_a_variable() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "clear_validators", "description": "clear validators", "input": null
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "clear_validators".into();
        e2e_config.call.returns_void = true;

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(!body.contains("final result ="), "void call must not be bound:\n{body}");
        assert!(
            body.contains(".clearValidators();"),
            "void call must still be awaited directly without a binding:\n{body}"
        );
    }
}
