//! Regression coverage for the void `not_error` vacuous-test defect.
//!
//! ~keep A fixture whose only assertion is `{"type": "not_error"}` on a `returns_void` call
//! gated the entire assertion loop in `csharp.rs`'s `render_test_method` behind `!returns_void`
//! (`assertions_body` stays empty for every void fixture), and `has_usable_assertion` was
//! `false` for the same reason — so `csharp/test_method.jinja` fell into its bare-call branch,
//! relying only on the uncaught-exception-fails-the-[Fact] behavior xUnit gives every test
//! method. xUnit has no `Assert.DoesNotThrow`; the fix wraps the call in
//! `Record.Exception`/`Record.ExceptionAsync` and asserts the caught exception is `null`, via
//! `render_test_method`'s `void_not_error` flag — a real, visible check instead of an implicit
//! one, matching the Kotlin `not_error` fix (`kotlin/not_error.rs`) closed for the non-void case.
//!
//! Lives in its own file rather than growing `csharp.rs` or `csharp/tests.rs`: both are already
//! over the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md).

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::HashMap;

fn void_fixture(assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        id: "prefetch_languages".to_string(),
        description: "Prefetch languages".to_string(),
        assertions,
        ..Fixture::default()
    }
}

fn render_void_call(is_async: bool, assertions: Vec<Assertion>) -> String {
    let fixture = void_fixture(assertions);
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "prefetch_languages".to_string();
    e2e_config.call.returns_void = true;
    e2e_config.call.r#async = is_async;

    let field_resolver = FieldResolver::new(
        &HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let config = ResolvedCrateConfig {
        name: "sample".to_string(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    let mut visitor_class_decls: Vec<String> = Vec::new();
    super::render_test_method(
        &mut out,
        &mut visitor_class_decls,
        &fixture,
        "SampleClass",
        "PrefetchLanguages",
        "SampleException",
        "result",
        &[],
        &field_resolver,
        false,
        is_async,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &[],
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
/// `not_error`-only fixture rendered a bare `SampleClass.PrefetchLanguages();` with no
/// assertion anywhere in the method body.
#[test]
fn void_not_error_sync_wraps_the_call_in_record_exception() {
    let out = render_void_call(false, vec![not_error_assertion()]);

    assert!(
        out.contains("Record.Exception(() => SampleClass.PrefetchLanguages())"),
        "expected the void call wrapped in Record.Exception, got:\n{out}"
    );
    assert!(
        out.contains("Assert.Null(exception)"),
        "expected the caught exception to be asserted null, got:\n{out}"
    );
}

/// Async case: uses `Record.ExceptionAsync` instead, mirroring the async/sync split this
/// template already applies to the `expects_error` branch's `Assert.ThrowsAny`/
/// `Assert.ThrowsAnyAsync` pair.
#[test]
fn void_not_error_async_wraps_the_call_in_record_exception_async() {
    let out = render_void_call(true, vec![not_error_assertion()]);

    assert!(
        out.contains("await Record.ExceptionAsync(async () => await SampleClass.PrefetchLanguagesAsync())"),
        "expected the async void call wrapped in Record.ExceptionAsync, got:\n{out}"
    );
    assert!(
        out.contains("Assert.Null(exception)"),
        "expected the caught exception to be asserted null, got:\n{out}"
    );
}

/// A void fixture with no `not_error` assertion at all must keep its prior behavior — a bare
/// call statement — so wrapping every void call regardless of what it asserts would be a
/// different, unrequested behavior change.
#[test]
fn void_call_without_not_error_stays_a_bare_statement() {
    let out = render_void_call(false, vec![]);

    assert!(
        out.contains("SampleClass.PrefetchLanguages();"),
        "expected a bare call statement, got:\n{out}"
    );
    assert!(
        !out.contains("Record.Exception"),
        "must not wrap the call when no not_error assertion is present, got:\n{out}"
    );
}
