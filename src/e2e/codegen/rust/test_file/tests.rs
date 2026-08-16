use super::helpers::resolve_module_for_call;
use super::*;

#[test]
fn resolve_module_for_call_prefers_crate_name_override() {
    use crate::e2e::config::CallConfig;
    use std::collections::HashMap;
    let mut overrides = HashMap::new();
    overrides.insert(
        "rust".to_string(),
        crate::e2e::config::CallOverride {
            crate_name: Some("custom_crate".to_string()),
            module: Some("ignored_module".to_string()),
            ..Default::default()
        },
    );
    let call = CallConfig {
        overrides,
        ..Default::default()
    };
    let result = resolve_module_for_call(&call, "dep_name");
    assert_eq!(result, "custom_crate");
}

/// Regression test: a non-streaming fixture whose result struct has a `chunks`
/// field (registered in `fields_array`) must emit `let chunks = &result.chunks;`
/// before any assertion so the streaming-virtual-field arm's hardcoded `chunks`
/// identifier resolves.  Without the fix this generated
/// `assert!(chunks.len() >= 2 as usize, ...)` with `chunks` undeclared.
#[test]
fn fields_array_binding_emitted_before_count_min_assertion_for_non_streaming_fixture() {
    use crate::e2e::config::{CallConfig, StreamingConfig};
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::HashSet;

    let mut fields_array = HashSet::new();
    fields_array.insert("chunks".to_string());

    let call = CallConfig {
        function: "process".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        fields_array,
        returns_result: true,
        streaming: Some(StreamingConfig::Enabled(false)),
        ..Default::default()
    };

    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "chunking_test".to_string(),
        description: "Chunking produces multiple pieces".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("chunks".to_string()),
            value: Some(serde_json::Value::Number(serde_json::Number::from(2u64))),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        category: None,
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(&mut out, &fixture, &e2e_config, &test_config, &[], "my_crate", None, false);

    assert!(
        out.contains("let chunks = &result.chunks"),
        "expected `let chunks = &result.chunks` binding before assertion; got:\n{out}"
    );
    assert!(
        out.contains("chunks.len() >= 2"),
        "expected count_min assertion referencing `chunks`; got:\n{out}"
    );
    // The binding must appear before the assertion in the output.
    let binding_pos = out.find("let chunks = &result.chunks").unwrap();
    let assert_pos = out.find("chunks.len() >= 2").unwrap();
    assert!(
        binding_pos < assert_pos,
        "binding must appear before assertion; got:\n{out}"
    );
}

/// Regression test for alef task #81: rust had no fallback at all for a dropped
/// field assertion — `is_valid_for_result` rejects the field, `render_assertion`
/// emits a skip comment, and (until this fix) nothing else ever consulted it. This
/// pins that `finalize_test_body` sees the skip comment with the exact marker text
/// the shared `fail_on_unavailable_field_markers` mechanism (src/e2e/codegen/mod.rs)
/// matches on. `body_buf` is a fresh buffer per `render_test_function` call (see
/// `let mut body_buf = String::new();` at the top of this file), so per-fixture
/// attribution is correct by construction — no offset bookkeeping needed here,
/// unlike Go's shared-buffer caller.
#[test]
fn dropped_field_assertion_carries_the_marker_that_arms_the_strict_mode() {
    use crate::e2e::config::CallConfig;
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::HashSet;

    let call = CallConfig {
        function: "process".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        result_fields: HashSet::from(["content".to_string()]),
        returns_result: true,
        ..Default::default()
    };
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "process_smoke".to_string(),
        description: "Process produces a result".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::Value::String("x".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        category: None,
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(&mut out, &fixture, &e2e_config, &test_config, &[], "my_crate", None, false);

    assert!(
        out.contains("field 'nonexistent_field' not available on result type"),
        "got:\n{out}"
    );
}

/// Regression test for alef task #81: a `test_backend` arg whose declared trait has
/// no matching `[[crates.trait_bridges]]` entry used to fall through — with only a
/// `tracing::warn!` — to rendering the arg's raw (null) fixture value as an ordinary
/// argument, deferring the real failure to a later `cargo build` far from the
/// fixture that caused it. It must now fail at generation time, naming the fixture,
/// the arg, and the missing trait — mirroring `c/assertions.rs`'s
/// `build_args_string_c`, which already fails loudly for the identical scenario.
#[test]
#[should_panic(expected = "fixture `register_sample_backend` requires trait `SampleBackend`")]
fn unregistered_test_backend_trait_fails_loudly_instead_of_falling_back() {
    use crate::e2e::config::{ArgMapping, CallConfig};
    use crate::e2e::fixture::Fixture;

    let call = CallConfig {
        function: "register_backend".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        returns_result: false,
        ..Default::default()
    };
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        id: "register_sample_backend".to_string(),
        description: "Register a sample backend".to_string(),
        args: vec![ArgMapping {
            name: "backend".to_string(),
            field: "backend".to_string(),
            arg_type: "test_backend".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some("SampleBackend".to_string()),
        }],
        ..Fixture::default()
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    // No `[[crates.trait_bridges]]` entries declared — `SampleBackend` is unregistered.
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(&mut out, &fixture, &e2e_config, &test_config, &[], "my_crate", None, false);
}

/// Regression test: a `result_is_simple` call with a `count_equals` assertion whose
/// `field` is NOT a real field on the (plain Vec) result type must still bind the
/// call to the result variable.  The assertion renderer emits `result.len()` for
/// `result_is_simple` calls regardless of the field, so binding to `_` would leave
/// `result` undefined.
#[test]
fn result_is_simple_count_assertion_binds_to_result_variable() {
    use crate::e2e::config::{CallConfig, StreamingConfig};
    use crate::e2e::fixture::{Assertion, Fixture};

    let call = CallConfig {
        function: "embed_texts".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        result_is_simple: true,
        returns_result: true,
        streaming: Some(StreamingConfig::Enabled(false)),
        ..Default::default()
    };

    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "embed_empty".to_string(),
        description: "embed_texts: empty input".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![
            Assertion {
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            },
            Assertion {
                assertion_type: "count_equals".to_string(),
                field: Some("embeddings".to_string()),
                value: Some(serde_json::Value::Number(serde_json::Number::from(0u64))),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            },
        ],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        category: None,
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(&mut out, &fixture, &e2e_config, &test_config, &[], "my_crate", None, false);

    assert!(
        out.contains("let result = embed_texts"),
        "expected the call to bind to `result`, not `_`; got:\n{out}"
    );
    assert!(
        out.contains("assert_eq!(result.len(), 0"),
        "expected `count_equals` assertion to render `result.len()`; got:\n{out}"
    );
    assert!(
        !out.contains("let _ = embed_texts"),
        "call must not bind to `_` when an assertion references the result; got:\n{out}"
    );
}

#[test]
fn handle_config_import_uses_resolved_options_type() {
    use crate::e2e::config::{ArgMapping, CallConfig, CallOverride};
    use crate::e2e::fixture::Fixture;
    use std::collections::HashMap;

    let mut overrides = HashMap::new();
    overrides.insert(
        "rust".to_string(),
        CallOverride {
            options_type: Some("SessionConfig".to_string()),
            ..Default::default()
        },
    );
    let call = CallConfig {
        function: "run_session".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        returns_result: false,
        args: vec![ArgMapping {
            name: "session".to_string(),
            field: "input.config".to_string(),
            arg_type: "handle".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }],
        overrides,
        ..Default::default()
    };
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "session_fixture".to_string(),
        description: "session fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "config": { "limit": 3 } }),
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        category: Some("sessions".to_string()),
    };
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    let out = render_test_file(
        "sessions",
        &[&fixture],
        &e2e_config,
        &test_config,
        &[],
        "my_crate",
        false,
        false,
    );

    assert!(
        out.contains("use my_crate::SessionConfig;"),
        "expected SessionConfig import, got:\n{out}"
    );
    assert!(out.contains("let session_config: SessionConfig = serde_json::from_str"));
    assert!(!out.contains("CrawlConfig"));
}
