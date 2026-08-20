//! Regression coverage for the void `not_error` vacuous-test defect.
//!
//! ~keep A fixture whose only assertion is `{"type": "not_error"}` on a `returns_void` call
//! bound no `result` to assert against (`test_method.rs`'s assertion loop `continue`s past a
//! void call before `render_assertion` is ever reached), so `body_buffer` held only the "no
//! result to assert on" skip comment. That gave `inert_example::inert_verdict` no executable
//! line to find, and it substituted an unconditional `try XCTSkipIf(true, ...)` for the whole
//! body — the method ran, but its declared check never did; it silently skipped itself every
//! time. The fix wraps the call itself in `XCTAssertNoThrow` (sync) or a do/catch that fails
//! the test on a caught error (async), via `test_method.rs`'s `void_not_error` flag, matching
//! the Kotlin `not_error` fix (`kotlin/not_error.rs`) closed for the non-void case.
//!
//! Lives in its own file rather than growing `test_method.rs`: that file is close to the
//! repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), matching the precedent set
//! by `is_true_tests.rs` and `enum_field_classification_tests.rs`.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::SwiftFirstClassMap;
use crate::e2e::fixture::{Assertion, Fixture};

fn void_fixture(assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        id: "prefetch_languages".to_string(),
        description: "Prefetch languages".to_string(),
        call: Some("prefetch_languages".to_string()),
        assertions,
        ..Fixture::default()
    }
}

fn render_void_call(is_async: bool, assertions: Vec<Assertion>) -> String {
    let fixture = void_fixture(assertions);
    let call_config = CallConfig {
        function: "prefetch_languages".to_string(),
        returns_void: true,
        r#async: is_async,
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig {
        call: call_config.clone(),
        ..E2eConfig::default()
    };
    e2e_config.calls.insert("prefetch_languages".to_string(), call_config);
    let config = ResolvedCrateConfig {
        name: "sample".to_string(),
        ..ResolvedCrateConfig::default()
    };
    let map = SwiftFirstClassMap::default();

    let mut out = String::new();
    super::test_method::render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "",
        "",
        &[],
        false,
        None,
        &map,
        "Sample",
        &config,
        &[],
        &[],
        &[],
        &[],
    );
    out
}

fn not_error_assertion() -> Assertion {
    Assertion {
        assertion_type: "not_error".to_string(),
        ..Assertion::default()
    }
}

/// The regression this file exists for, synchronous case: before the fix, a void
/// `not_error`-only fixture rendered `try XCTSkipIf(true, ...)` — the test skipped itself
/// instead of running the check it declared. `CallOverride` is unused here on purpose: the
/// base `CallConfig` already carries `returns_void`/`async`.
#[test]
fn void_not_error_sync_wraps_the_call_in_xctassert_no_throw() {
    let out = render_void_call(false, vec![not_error_assertion()]);

    assert!(
        out.contains("XCTAssertNoThrow(try Sample.prefetchLanguages())"),
        "expected the void call wrapped in XCTAssertNoThrow, got:\n{out}"
    );
    assert!(
        !out.contains("XCTSkipIf"),
        "must not fall back to an unconditional skip, got:\n{out}"
    );
    assert!(
        !out.contains("        try Sample.prefetchLanguages()\n"),
        "the call must not also appear as a bare, unasserted statement, got:\n{out}"
    );
}

/// Async case: `XCTAssertNoThrow` has no async-aware overload in this codebase's established
/// pattern (see the do/catch this file already uses for `expects_error`'s async branch), so
/// the async call gets a do/catch that fails the test via `XCTFail` on a caught error instead.
#[test]
fn void_not_error_async_wraps_the_call_in_a_do_catch_that_fails_on_error() {
    let out = render_void_call(true, vec![not_error_assertion()]);

    assert!(
        out.contains("do {"),
        "expected a do/catch wrapping the async call, got:\n{out}"
    );
    assert!(
        out.contains("try await Sample.prefetchLanguages()"),
        "expected the async call inside the do block, got:\n{out}"
    );
    assert!(
        out.contains("XCTFail("),
        "expected the catch block to fail the test, got:\n{out}"
    );
    assert!(
        !out.contains("XCTSkipIf"),
        "must not fall back to an unconditional skip, got:\n{out}"
    );
}

/// A void fixture with no `not_error` assertion at all must keep its prior behavior — a bare
/// call with the "no result to assert on" skip comment — so wrapping every void call
/// regardless of what it asserts would be a different, unrequested behavior change.
#[test]
fn void_call_without_not_error_stays_unchanged() {
    let out = render_void_call(false, vec![]);

    assert!(
        out.contains("try Sample.prefetchLanguages()"),
        "expected a bare call statement, got:\n{out}"
    );
    assert!(
        !out.contains("XCTAssertNoThrow"),
        "must not wrap the call when no not_error assertion is present, got:\n{out}"
    );
}
