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
//! `sync_trait_bridge_void_not_error_wraps_in_expect_fn_not_to_throw` and
//! `async_trait_bridge_void_not_error_wraps_in_resolves_not_to_throw` below cover a second, later
//! regression: `render_test_case` unconditionally forced `call_is_async = call_is_async ||
//! has_trait_bridge`, so ANY call taking a `test_backend` (trait-bridge) argument was treated as
//! async for wrapper-selection purposes even when the call itself is synchronous. That made a
//! synchronous trait method's `not_error` assertion emit `.resolves.not.toThrow()` on a
//! non-Promise — a runtime TypeError, and a type error under strict TypeScript, since jest's
//! `.resolves` is typed to require `Promise<T>`. The fix keeps `call_is_async` (the IR/config
//! authority) unforced for the template's wrapper-shape decision, and only forces the separate
//! `await_kw` local (safe, since `await` on a non-Promise is a legal no-op). ~keep
//!
//! Lives in its own file rather than growing `assertions.rs`: that file is already over the
//! repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), matching the precedent set by
//! `is_true_tests.rs` and this backend's `node_enum_import_tests.rs`.

use super::*;
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::e2e::config::{ArgMapping, CallConfig};
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

/// A call taking a `test_backend` (trait-bridge) argument, so `render_test_case` sees
/// `has_trait_bridge == true` in addition to `is_async`. Only `render_void_call`'s `is_async`
/// flag decides the true async/sync signal here (`call_config.r#async`); trait-bridge presence
/// must never override it for the `void_not_error` wrapper choice.
fn render_trait_bridge_void_call(is_async: bool, assertions: Vec<Assertion>) -> String {
    let mut fixture = void_fixture(assertions);
    fixture.args = vec![ArgMapping {
        name: "backend".to_string(),
        field: "backend".to_string(),
        arg_type: "test_backend".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some("SyncTraitStub".to_string()),
    }];
    let call = CallConfig {
        function: "runWithBackend".to_string(),
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
    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "SyncTraitStub".to_string(),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };
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
    );
    out
}

/// Sync control: a synchronous trait-bridge call (`r#async: false`) must still get the
/// throw-based sync wrapper, not `.resolves` — `.resolves` requires a real Promise both at
/// runtime and, under strict TypeScript, at the type level (jest types `.resolves` as needing
/// `Promise<T>`). Trait-bridge presence alone must not force the async wrapper.
#[test]
fn sync_trait_bridge_void_not_error_wraps_in_expect_fn_not_to_throw() {
    let out = render_trait_bridge_void_call(false, vec![not_error_assertion()]);

    assert!(
        out.contains("expect(() => runWithBackend("),
        "expected the sync trait-bridge call wrapped in expect(() => ...).not.toThrow(), got:\n{out}"
    );
    assert!(
        out.contains(").not.toThrow();"),
        "expected a plain `.not.toThrow()` (no `.resolves`), got:\n{out}"
    );
    assert!(
        !out.contains(".resolves"),
        "a sync trait-bridge call has no Promise for `.resolves` to unwrap; trait-bridge \
         presence must not force the async wrapper, got:\n{out}"
    );
}

/// Async control: an async trait-bridge call (`r#async: true`) keeps the `.resolves` wrapper,
/// so the sync fix above does not regress the already-correct async shape.
#[test]
fn async_trait_bridge_void_not_error_wraps_in_resolves_not_to_throw() {
    let out = render_trait_bridge_void_call(true, vec![not_error_assertion()]);

    assert!(
        out.contains("await expect(runWithBackend("),
        "expected the async trait-bridge call wrapped in expect(...).resolves.not.toThrow(), got:\n{out}"
    );
    assert!(
        out.contains(").resolves.not.toThrow();"),
        "expected `.resolves.not.toThrow()`, got:\n{out}"
    );
    assert!(
        !out.contains("expect(() =>"),
        "an async trait-bridge call must not use the sync `expect(() => ...)` wrapper, got:\n{out}"
    );
}
