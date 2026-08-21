//! Regression coverage for the void `not_error` vacuous-test defect.
//!
//! ~keep A fixture whose only assertion is `{"type": "not_error"}` on a `returns_void` call
//! bound no `result` — `render_test_case` emits a bare `await receiver.function(args);` for
//! every `returns_void` call, and its own vacuous-assertion fallback (`expect(result,
//! isNotNull)`) explicitly excludes `returns_void` because `result` was never bound there. So
//! the generated test body for a void `not_error`-only fixture was a bare `await` statement
//! with no assertion at all, relying only on the implicit "an uncaught rejection fails the
//! test" behavior `package:test` gives every `Future`. The fix wraps the call in
//! `expectLater(..., completes)` — `completes` is a real `package:test` matcher that fails if
//! the `Future` rejects — via `render_test_case`'s `void_not_error` flag, matching the Kotlin
//! `not_error` fix (`kotlin/not_error.rs`) closed for the non-void case.
//!
//! Lives in its own file rather than growing `test_case.rs`: that file is already over the
//! repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), matching the precedent set
//! by `local_naming_tests.rs` and `enum_field_classification_tests.rs`.

use super::test_case::{DartTestCaseContext, render_test_case};
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture};

fn render_void_call(assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        id: "prefetch_languages".to_string(),
        description: "Prefetch languages".to_string(),
        assertions,
        ..Fixture::default()
    };

    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "prefetch_languages".to_string();
    e2e_config.call.returns_void = true;

    let config = ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let functions: Vec<crate::core::ir::FunctionDef> = Vec::new();
    let first_class_map = super::values::build_dart_first_class_map(&type_defs, &enums, &e2e_config);

    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        DartTestCaseContext {
            e2e_config: &e2e_config,
            lang: "dart",
            bridge_class: &config.dart_bridge_class_name(),
            dart_first_class_map: &first_class_map,
            adapters: &config.adapters,
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            functions: &functions,
            errors: &[],
            native_typed_dtos: true,
            is_snippet: false,
        },
    );
    out
}

fn not_error_assertion() -> Assertion {
    Assertion {
        assertion_type: "not_error".to_string(),
        ..Assertion::default()
    }
}

/// The regression this file exists for: before the fix, a void `not_error`-only fixture
/// rendered a bare `await bridge.prefetchLanguages();` with no assertion anywhere in the
/// test body.
#[test]
fn void_not_error_wraps_the_call_in_expect_later_completes() {
    let out = render_void_call(vec![not_error_assertion()]);

    assert!(
        out.contains("await expectLater(") && out.contains("prefetchLanguages"),
        "expected the void call wrapped in expectLater(..., completes), got:\n{out}"
    );
    assert!(
        out.contains("completes"),
        "expected the `completes` matcher, got:\n{out}"
    );
    assert!(
        !out.contains("await bridge.prefetchLanguages();\n") && !out.contains("await PrefetchLanguages();\n"),
        "the call must not also appear as a bare, unasserted statement, got:\n{out}"
    );
}

/// A void fixture with no `not_error` assertion at all must keep its prior behavior — a bare
/// `await` call — so wrapping every void call regardless of what it asserts would be a
/// different, unrequested behavior change.
#[test]
fn void_call_without_not_error_stays_a_bare_await_statement() {
    let out = render_void_call(vec![]);

    assert!(
        out.contains("prefetchLanguages"),
        "expected the call to still be emitted, got:\n{out}"
    );
    assert!(
        !out.contains("expectLater"),
        "must not wrap the call when no not_error assertion is present, got:\n{out}"
    );
}
