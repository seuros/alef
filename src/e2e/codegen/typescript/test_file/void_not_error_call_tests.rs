//! Regression coverage for `void_not_error` call-wrapper shape selection (sync vs. async).
//!
//! ~keep `test_function.jinja`'s `void_not_error` branch unconditionally emitted
//! `await expect(call_expr).resolves.not.toThrow()`, regardless of whether `call_expr` actually
//! returns a Promise. NAPI's synchronous bindings (e.g. `cleanCache()`, `configure()`, `init()`,
//! `prefetch()`) return `undefined` directly, so `.resolves` — which requires a Promise — threw
//! `TypeError: You must provide a Promise to expect() when using .resolves, not 'undefined'` at
//! test run time in a consumer repo. The fix threads the already-computed `call_is_async` into
//! the template so a sync void call gets `expect(() => call_expr).not.toThrow()` instead, which
//! needs no Promise. This table covers both shapes so neither regresses into the other.
//!
//! Lives in its own file rather than growing `assertions.rs`: that file is already over the
//! repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), matching the precedent set by
//! `is_true_tests.rs` and this backend's `node_enum_import_tests.rs`.

use super::*;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::CallConfig;
use crate::e2e::fixture::Assertion;

fn void_fixture(assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        id: "prefetch_languages".to_string(),
        description: "Prefetch languages".to_string(),
        assertions,
        ..Fixture::default()
    }
}

fn not_error_assertion() -> Assertion {
    Assertion {
        assertion_type: "not_error".to_string(),
        ..Assertion::default()
    }
}

fn render_void_call(is_async: bool, assertions: Vec<Assertion>) -> String {
    let fixture = void_fixture(assertions);
    let call = CallConfig {
        function: "prefetchLanguages".to_string(),
        module: "myLib".to_string(),
        result_var: "result".to_string(),
        returns_void: true,
        r#async: is_async,
        ..CallConfig::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig::default();
    let type_defs: Vec<TypeDef> = Vec::new();
    let enums: Vec<EnumDef> = Vec::new();
    let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();
    let mut referenced_enums = std::collections::BTreeSet::new();

    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        None,
        None,
        &e2e_config,
        "node",
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &type_defs,
        &enums,
        &[],
        "",
        &config,
        &mut referenced_enums,
        &errors,
        &[],
    );
    out
}

/// Sync shape: a synchronous void call resolves no Promise, so `.resolves` is a hard runtime
/// `TypeError`. The wrapper must be a plain `expect(() => ...)` instead.
#[test]
fn sync_void_not_error_wraps_the_call_in_expect_fn_not_to_throw() {
    let out = render_void_call(false, vec![not_error_assertion()]);

    assert!(
        out.contains("expect(() => prefetchLanguages()).not.toThrow();"),
        "expected the sync void call wrapped in expect(() => ...).not.toThrow(), got:\n{out}"
    );
    assert!(
        !out.contains(".resolves"),
        "a sync call has no Promise for `.resolves` to unwrap, got:\n{out}"
    );
}

/// Async shape: an async void call resolves a Promise, so `.resolves.not.toThrow()` is the
/// correct, awaitable wrapper.
#[test]
fn async_void_not_error_wraps_the_call_in_resolves_not_to_throw() {
    let out = render_void_call(true, vec![not_error_assertion()]);

    assert!(
        out.contains("await expect(prefetchLanguages()).resolves.not.toThrow();"),
        "expected the async void call wrapped in expect(...).resolves.not.toThrow(), got:\n{out}"
    );
    assert!(
        !out.contains("expect(() =>"),
        "an async call must not use the sync `expect(() => ...)` wrapper, got:\n{out}"
    );
}
