//! Regression coverage for one decision every e2e backend that distinguishes streaming from
//! non-streaming calls must make the same way: a streaming fixture's function-scoped result
//! local (Rust's `result`, Go's `result`/`value`, ...) is never bound at all -- only the
//! drained-stream locals (`stream`/`chunks`) are -- so any codegen path that unconditionally
//! emits an accessor off the non-streaming binding for a streaming fixture references an
//! undeclared variable.
//!
//! Two backends implemented this decision independently and both dropped the same guard: the
//! Rust generator's "emit Option field unwrap bindings" loop
//! (`rust/test_file/test_function.rs`) checked `!result_is_vec` but not `!is_streaming`, and the
//! Go generator's "optional locals" loop (`go/test_function.rs`) checked
//! `!result_is_simple && !field_resolver.is_valid_for_result(f)` but likewise never checked
//! `is_streaming`. Both surfaced downstream as a compile error (Rust `E0425 cannot find value
//! 'result'`; Go `undefined: result`) the moment a streaming fixture asserted a string-typed
//! optional field (e.g. `finish_reason`) that also happened to be declared optional.
//!
//! This pins the decision, independent of per-language syntax, through each backend's own
//! public [`super::E2eCodegen::generate`] entry point -- mirroring
//! `working_directory_guard_tests.rs` -- so a regression in either backend, or a new backend
//! that reintroduces an unguarded non-streaming accessor, is caught here.

use super::E2eCodegen;
use super::go::GoCodegen;
use super::rust::RustE2eCodegen;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{CallConfig, E2eConfig, StreamingConfig};
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};
use std::collections::HashSet;

fn generated_file_ending_in<'a>(files: &'a [GeneratedFile], suffix: &str) -> &'a GeneratedFile {
    files.iter().find(|f| f.path.ends_with(suffix)).unwrap_or_else(|| {
        panic!(
            "expected a generated file ending in `{suffix}`, got: {:?}",
            files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
        )
    })
}

/// A streaming fixture that asserts a string-typed optional field (`finish_reason`) declared
/// optional -- the exact shape that reaches the buggy loop in each backend.
fn streaming_tool_calls_fixture() -> Fixture {
    Fixture {
        id: "stream_with_tool_calls".to_string(),
        description: "streams tool-call chunks and asserts the finish reason".to_string(),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("finish_reason".to_string()),
            value: Some(serde_json::json!("tool_calls")),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn streaming_call_config() -> CallConfig {
    CallConfig {
        function: "process".to_string(),
        fields_optional: HashSet::from(["finish_reason".to_string()]),
        streaming: Some(StreamingConfig::Enabled(true)),
        // Both backends' streaming binding path only activates on the `Result`-returning
        // branch; `CallConfig::default()`'s `returns_result: false` would route Go's
        // codegen down the `err := call()` fire-and-forget path instead, never reaching
        // the buggy loop this test exists to guard. ~keep
        returns_result: true,
        ..Default::default()
    }
}

fn streaming_group() -> FixtureGroup {
    FixtureGroup {
        category: "streaming".to_string(),
        fixtures: vec![streaming_tool_calls_fixture()],
    }
}

fn streaming_e2e_config() -> E2eConfig {
    E2eConfig {
        call: streaming_call_config(),
        ..Default::default()
    }
}

#[test]
fn rust_streaming_test_never_references_the_non_streaming_result_binding() {
    let e2e_config = streaming_e2e_config();
    let files = RustE2eCodegen
        .generate(
            &[streaming_group()],
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("rust e2e generation succeeds for a streaming fixture");
    let test_file = generated_file_ending_in(&files, "streaming_test.rs");
    assert!(
        !test_file.content.contains("result.finish_reason"),
        "rust streaming test must not reference the non-streaming `result` binding, got:\n{}",
        test_file.content
    );
    assert!(
        test_file.content.contains("let stream ="),
        "rust streaming test must bind the stream, got:\n{}",
        test_file.content
    );
}

#[test]
fn go_streaming_test_never_references_the_non_streaming_result_binding() {
    let e2e_config = streaming_e2e_config();
    let files = GoCodegen
        .generate(
            &[streaming_group()],
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("go e2e generation succeeds for a streaming fixture");
    let test_file = generated_file_ending_in(&files, "streaming_test.go");
    assert!(
        !test_file.content.contains(":= result.") && !test_file.content.contains("result.FinishReason"),
        "go streaming test must not reference the non-streaming `result` binding, got:\n{}",
        test_file.content
    );
    assert!(
        test_file.content.contains("stream, err :="),
        "go streaming test must bind the stream, got:\n{}",
        test_file.content
    );
}
