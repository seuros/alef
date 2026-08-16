//! Dart-specific e2e generator tests.

use super::stubs::emit_test_backend;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
use crate::e2e::fixture::Fixture;

fn make_trait_bridge(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        ..Default::default()
    }
}

fn make_method(name: &str, required: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::Bool),
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
        has_default_impl: !required,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
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
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Verify that no sample_core-domain names leak into the generated output when
/// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
#[test]
fn dart_stub_contains_no_sample_crate_domain_names() {
    let bridge = make_trait_bridge("TestTrait");
    let required_method = make_method("doWork", true);
    let methods = [&required_method];
    let fixture = make_fixture("my_test_fixture");

    let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);

    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        !output.contains("SampleCrate"),
        "must not contain literal 'SampleCrate', got:\n{output}"
    );
    assert!(
        !output.contains("sample_crate::"),
        "must not contain 'sample_crate::', got:\n{output}"
    );
    assert!(
        !output.contains("SampleCrateBridge"),
        "must not contain 'SampleCrateBridge', got:\n{output}"
    );
    assert!(
        output.contains("TestStubMyTestFixture"),
        "class name must be derived from fixture id, got:\n{output}"
    );
    assert!(
        output.contains("extends TestTrait"),
        "class must extend the configured trait class, got:\n{output}"
    );
    assert!(
        output.contains("doWork"),
        "required method must be emitted, got:\n{output}"
    );
}

fn make_param(name: &str, ty: TypeRef) -> crate::core::ir::ParamDef {
    crate::core::ir::ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

fn make_method_with_params(name: &str, required: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![
            make_param("content", TypeRef::Bytes),
            make_param("mime_type", TypeRef::String),
        ],
        return_type: TypeRef::Named("SampleResult".to_string()),
        is_async: true,
        is_static: false,
        error_type: Some("anyhow::Error".to_string()),
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: !required,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// Verify params use concrete Dart types (not `dynamic`) and no @override annotation.
#[test]
fn dart_stub_uses_typed_params_not_dynamic() {
    let bridge = make_trait_bridge("TestTrait");
    let required_method = make_method_with_params("extract", true);
    let methods = [&required_method];
    let fixture = make_fixture("my_test_fixture");

    let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);
    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        !output.contains("dynamic content"),
        "param must not use `dynamic`, got:\n{output}"
    );
    assert!(
        output.contains("Uint8List content"),
        "bytes param must map to Uint8List, got:\n{output}"
    );
    assert!(
        output.contains("String mimeType"),
        "string param must map to String, got:\n{output}"
    );
    assert!(
        output.contains("Future<SampleResult>"),
        "return type must be concrete not dynamic, got:\n{output}"
    );
    assert!(
        !output.contains("@override"),
        "local class members must not use @override annotation, got:\n{output}"
    );
}

/// Verify that `fixture.input["name"]` is used as the plugin name when present.
#[test]
fn dart_stub_uses_fixture_input_name_for_plugin_name() {
    let bridge = make_trait_bridge("TestTrait");
    let required_method = make_method("doWork", true);
    let methods = [&required_method];
    let mut fixture = make_fixture("my_fixture_id");
    fixture.input = serde_json::json!({ "name": "my-backend-name" });

    let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);
    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        output.contains("'my-backend-name'"),
        "plugin name must come from fixture.input.name, got:\n{output}"
    );
    assert!(
        !output.contains("my_fixture_id"),
        "fixture id must not appear as plugin name when input.name is set, got:\n{output}"
    );
}

/// Verify that _setEnv helper forces overwrite=1 and checks return code.
/// Regression test for the bug where setenv(..., 0) silently no-ops when the
/// env var is already set, causing SAMPLE_ALLOW_PRIVATE_NETWORK to be
/// invisible to Rust FFI dylib in dart e2e tests.
#[test]
fn dart_emit_setenv_forces_overwrite_and_checks_return_code() {
    use crate::e2e::config::E2eConfig;
    use std::collections::HashMap;

    // Create a minimal E2eConfig with an env var to trigger _setEnv emission.
    let mut env = HashMap::new();
    env.insert("SAMPLE_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());

    let e2e_config = E2eConfig {
        env,
        ..Default::default()
    };

    // Build a minimal test file just to check the _setEnv helper.
    // We'll use a dummy fixture and configuration.
    let fixture = make_fixture("test_fixture");
    let _bridge = make_trait_bridge("TestTrait");
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs = [];
    let enums = [];
    let adapters = [];
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &adapters,
        &config,
        &type_defs,
        &enums,
    );

    // Verify that the generated setenv call uses overwrite=1 (third argument).
    assert!(
        output.contains("setenv(keyPtr, valuePtr, 1)"),
        "setenv must use overwrite=1, got:\n{output}"
    );

    // Verify that the old buggy pattern is NOT in the output.
    assert!(
        !output.contains("setenv(keyPtr, valuePtr, 0)"),
        "setenv must NOT use overwrite=0, got:\n{output}"
    );

    // Verify that return code is captured and checked.
    assert!(
        output.contains("final result = setenv(keyPtr, valuePtr, 1)"),
        "setenv result must be captured, got:\n{output}"
    );

    assert!(
        output.contains("if (result != 0)"),
        "return code must be checked with 'if (result != 0)', got:\n{output}"
    );

    assert!(
        output.contains("throw StateError"),
        "must throw StateError on non-zero return code, got:\n{output}"
    );
}

/// An `error` assertion with a declared `value` must produce a `throwsA`
/// predicate matcher that checks both the caught error's `toString()` and its
/// `runtimeType.toString()`, since fixture authors use either a message-only
/// field name or a type-name prefix.
#[test]
fn dart_error_assertion_with_declared_value_checks_message_and_type() {
    use crate::e2e::fixture::Assertion;

    let mut fixture = make_fixture("invalid_thing");
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        value: Some(serde_json::json!("ThingNotFound")),
        ..Default::default()
    });

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let config = crate::core::config::ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
    );

    assert!(
        output.contains(
            "throwsA(predicate((e) => e.toString().contains('ThingNotFound') || e.runtimeType.toString().contains('ThingNotFound')))"
        ),
        "expected a disjunctive message-or-type predicate matcher against the declared value, got:\n{output}"
    );
    assert!(
        !output.contains("throwsA(anything)"),
        "declared value must replace the anything-matcher, got:\n{output}"
    );
}

/// With no declared `value` on the `error` assertion, output must be
/// byte-identical to the pre-existing `throwsA(anything)` behavior.
#[test]
fn dart_error_assertion_without_declared_value_is_byte_identical() {
    use crate::e2e::fixture::Assertion;

    let mut fixture = make_fixture("invalid_thing");
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let config = crate::core::config::ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
    );

    assert!(output.contains("throwsA(anything)"));
    assert!(!output.contains("predicate((e)"));
}

/// Declared error values containing Dart string-interpolation and escape
/// characters (`'`, `\`, `$`) must be escaped via the shared `escape_dart`
/// helper, not hand-rolled, so the emitted literal stays a valid single-quoted
/// Dart string.
#[test]
fn dart_error_assertion_escapes_declared_value_for_dart_string_literal() {
    use crate::e2e::fixture::Assertion;

    let mut fixture = make_fixture("invalid_thing");
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        value: Some(serde_json::json!("bad 'field' \\ $value")),
        ..Default::default()
    });

    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let config = crate::core::config::ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
    );

    let expected_escaped = super::values::escape_dart("bad 'field' \\ $value");
    let expected_snippet = format!("e.toString().contains('{expected_escaped}')");
    assert!(
        output.contains(&expected_snippet),
        "expected escaped literal snippet `{expected_snippet}` in:\n{output}"
    );
}

#[test]
fn dart_test_file_emits_wrapper_for_call_config_trait_argument() {
    let fixture = make_fixture("register_backend");
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "registerBackend".into();
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
    let mut config = crate::core::config::ResolvedCrateConfig::default();
    config.trait_bridges.push(make_trait_bridge("TestTrait"));
    let type_defs = [crate::core::ir::TypeDef {
        name: "TestTrait".into(),
        methods: vec![make_method("doWork", true)],
        ..Default::default()
    }];
    let output = super::test_file::render_test_file(
        "plugins",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &crate::e2e::field_access::DartFirstClassMap::default(),
        &[],
        &config,
        &type_defs,
        &[],
    );
    assert!(output.contains("Future<TestTraitDartImpl> _createTestStubRegisterBackendWrapper()"));
    assert!(output.contains("await _createTestStubRegisterBackendWrapper()"));
}

#[test]
fn dart_trait_stub_wrapper_compiles() {
    if std::process::Command::new("dart").arg("--version").output().is_err() {
        return;
    }
    let method = make_method("doWork", true);
    let emission = emit_test_backend(
        &make_trait_bridge("TestTrait"),
        &[&method],
        &make_fixture("register_backend"),
        &[],
    );
    let source = format!(
        "abstract class TestTrait {{ Future<bool> doWork(); }}\nclass TestTraitDartImpl {{}}\nFuture<TestTraitDartImpl> createTestTraitDartImpl({{required String pluginName, required String pluginVersion, required Future<bool> Function() doWork}}) async => TestTraitDartImpl();\n{}\nFuture<void> main() async {{ await _createTestStubRegisterBackendWrapper(); }}\n",
        emission.setup_block
    );
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("stub.dart");
    std::fs::write(&source_path, &source).expect("write Dart source");
    // Pin the child's working directory. Other tests in this binary mutate the
    // process-global cwd via `set_current_dir` into tempdirs that are then dropped, so
    // an inherited cwd can already be deleted by the time this runs -- the Dart VM then
    // fails startup with "Error determining current directory" rather than any analysis
    // result. ~keep
    let output = std::process::Command::new("dart")
        .args(["analyze", "--fatal-infos"])
        .arg(&source_path)
        .current_dir(directory.path())
        .output()
        .expect("run Dart analyzer");
    assert!(
        output.status.success(),
        "dart analyze failed ({})\n--- stdout ---\n{}\n--- stderr ---\n{}\n--- source ---\n{source}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
