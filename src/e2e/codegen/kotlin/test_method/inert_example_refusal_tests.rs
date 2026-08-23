//! Refusal behaviour for fixtures whose declared assertions cannot be resolved.

use super::render_test_method;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::inert_example::take_inert_examples;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "equals".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::json!("x")),
        ..Default::default()
    }
}

/// A non-empty `result_fields` set is what arms the availability oracle: with it empty the
/// resolver is deliberately permissive and no field is ever rejected. ~keep
fn render(fixture_id: &str, assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        id: fixture_id.to_string(),
        description: "kotlin refusal fixture".to_string(),
        assertions,
        ..Fixture::default()
    };
    let e2e_config = E2eConfig {
        result_fields: std::collections::HashSet::from(["content".to_string()]),
        call: CallConfig {
            function: "process".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let mut out = String::new();
    render_test_method(
        &mut out,
        &fixture,
        "Facade",
        "",
        "",
        &[],
        None,
        false,
        &e2e_config,
        &std::collections::HashMap::new(),
        false,
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("render_test_method succeeds");
    out
}

/// CONTROL, asserted first: a field the oracle resolves still renders its real check and
/// records no refusal. An over-broad refusal here would silently delete coverage that runs
/// today — the same defect pointing the other way. ~keep
#[test]
fn a_resolvable_assertion_is_published_unchanged() {
    let _ = take_inert_examples();

    let out = render("kotlin_control", vec![assertion("content")]);

    assert!(
        out.contains("assertEquals("),
        "the renderable assertion must still be emitted, got:\n{out}"
    );
    assert!(
        !out.contains("assumeTrue(false"),
        "a live example must not be refused, got:\n{out}"
    );
    assert!(
        take_inert_examples().is_empty(),
        "nothing may be recorded for a live example"
    );
}

/// The blocker: every declared assertion funnels into a skip marker, so the method called the
/// binding and then checked nothing. Kotlin has no formatter that objects to a body ending on
/// a lone comment, so this shipped as a permanent green.
#[test]
fn an_unresolved_field_path_is_refused_with_a_failing_check() {
    let _ = take_inert_examples();

    let out = render(
        "kotlin_unresolved",
        vec![assertion("nonexistent_field"), assertion("another_missing_field")],
    );

    assert!(
        out.contains("kotlin.test.assertTrue(false, \"alef resolved no assertion for fixture `kotlin_unresolved`"),
        "a consumer-fixable gap must be refused with a FAILING check, got:\n{out}"
    );
    assert!(
        out.contains("nonexistent_field") && out.contains("another_missing_field"),
        "the markers must be carried into the refusal, got:\n{out}"
    );
    assert!(
        out.contains("// skipped:"),
        "the skip markers must still be emitted, not replaced by silence, got:\n{out}"
    );
    assert!(
        !out.contains("assumeTrue(false"),
        "a consumer-fixable gap must not be parked as skipped, got:\n{out}"
    );
    let refusals = take_inert_examples();
    assert_eq!(refusals.len(), 1, "the refusal must be recorded once for the summary");
    assert_eq!(refusals[0].fixture_id, "kotlin_unresolved");
}

/// alef's own debt is not the consumer's to fix, so it gets JUnit's assumption-based skip
/// rather than a failure: failing a consumer's build for a gap no fixture edit clears is what
/// forces the blanket opt-out this mechanism exists to avoid.
#[test]
fn acknowledged_generator_debt_is_refused_as_a_skip() {
    let _ = take_inert_examples();

    let out = render(
        "kotlin_generator_debt",
        vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::json!("x")),
            skip: Some(crate::e2e::fixture::AssertionSkip::All(true)),
            ..Default::default()
        }],
    );

    assert!(
        out.contains(
            "org.junit.jupiter.api.Assumptions.assumeTrue(false, \"alef rendered no runnable expectation for \
             fixture `kotlin_generator_debt`"
        ),
        "acknowledged debt must be parked as skipped, got:\n{out}"
    );
    assert!(
        !out.contains("assertTrue(false"),
        "alef's own debt must not fail a consumer's suite, got:\n{out}"
    );
    assert_eq!(take_inert_examples().len(), 1);
}

/// CONTROL: a fixture that declares NO assertions is the deliberate "just call it" smoke
/// contract and must be published exactly as before. ~keep
#[test]
fn a_fixture_with_no_declared_assertions_keeps_its_smoke_test_shape() {
    let _ = take_inert_examples();

    let out = render("kotlin_smoke_only", Vec::new());

    assert!(
        out.contains("Facade.process("),
        "the call must still be emitted, got:\n{out}"
    );
    assert!(
        !out.contains("assumeTrue(false") && !out.contains("assertTrue(false"),
        "a fixture with no assertions must never be refused, got:\n{out}"
    );
    assert!(take_inert_examples().is_empty());
}
