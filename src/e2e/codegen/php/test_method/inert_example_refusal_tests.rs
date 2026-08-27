use super::render_test_method;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::inert_example::take_inert_examples;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

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
        description: "php refusal fixture".to_string(),
        assertions,
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "process".into();
    e2e_config.call.result_var = "result".into();
    e2e_config.call.result_fields = HashSet::from(["content".to_string()]);
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    let mut trait_bridge_imports: Vec<String> = Vec::new();
    render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "php",
        "Sample",
        "SampleClient",
        &[],
        &[],
        &[],
        &super::super::enum_variant_access::PhpEnumLowering::from_enums(&[]),
        &HashMap::new(),
        false,
        None,
        "",
        &[],
        "",
        &config,
        &[],
        &mut trait_bridge_imports,
    );
    out
}

/// CONTROL, asserted first: a field the oracle resolves still renders its real check and the
/// vacuous fallback is not appended beside it. An over-broad refusal here would silently
/// delete coverage that runs today — the same defect pointing the other way. ~keep
#[test]
fn a_resolvable_assertion_is_published_unchanged() {
    let _ = take_inert_examples();

    let out = render("php_control", vec![assertion("content")]);

    assert!(
        // The renderer emits a double-quoted PHP string literal. ~keep
        out.contains("$this->assertEquals(\"x\","),
        "the renderable assertion must still be emitted, got:\n{out}"
    );
    assert!(
        !out.contains("markTestSkipped('alef") && !out.contains("$this->fail('alef"),
        "a live example must not be refused, got:\n{out}"
    );
    assert!(
        take_inert_examples().is_empty(),
        "nothing may be recorded for a live example"
    );
}

/// A field the availability oracle rejects is the consumer's to fix, so the disarmed run that
/// still emits it gets an assertion that FAILS and names the fixture — never `markTestSkipped`,
/// which would let a fixable authoring gap sit quietly in the skipped column forever.
#[test]
fn an_unresolved_field_path_is_refused_with_a_failing_check() {
    let _ = take_inert_examples();

    let out = render(
        "php_unresolved",
        vec![assertion("nonexistent_field"), assertion("another_missing_field")],
    );

    assert!(
        out.contains("$this->fail('alef resolved no assertion for fixture `php_unresolved`"),
        "a consumer-fixable gap must be refused with a FAILING check, got:\n{out}"
    );
    assert!(
        out.contains("// skipped:") && out.contains("nonexistent_field") && out.contains("another_missing_field"),
        "the skip markers must be carried into the refusal, not replaced by silence, got:\n{out}"
    );
    assert!(
        !out.contains("$this->assertNotNull($result);"),
        "the vacuous fallback must not stand in for the refused assertions, got:\n{out}"
    );
    let refusals = take_inert_examples();
    assert_eq!(refusals.len(), 1, "the refusal must be recorded once for the summary");
    assert_eq!(refusals[0].fixture_id, "php_unresolved");
}

/// CONTROL: alef's own acknowledged debt keeps the non-streaming `assertNotNull($result)`
/// fallback. That check CAN fail — a binding returning null really does trip it — so refusing
/// it would delete the "the call worked" coverage it carries. ~keep
#[test]
fn acknowledged_debt_on_a_non_streaming_call_keeps_its_failable_fallback() {
    let _ = take_inert_examples();

    let out = render(
        "php_generator_debt",
        vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::json!("x")),
            skip: Some(crate::e2e::fixture::AssertionSkip::All(true)),
            ..Default::default()
        }],
    );

    assert!(
        out.contains("$this->assertNotNull($result);"),
        "the failable fallback must survive, got:\n{out}"
    );
    assert!(
        out.contains("// skipped:"),
        "the marker must survive beside the fallback, got:\n{out}"
    );
    assert!(
        take_inert_examples().is_empty(),
        "an example that still asserts something is not a refusal"
    );
}

/// CONTROL: a fixture that declares NO assertions is the deliberate "just call it" smoke
/// contract and must be published exactly as before. ~keep
#[test]
fn a_fixture_with_no_declared_assertions_keeps_its_smoke_test_shape() {
    let _ = take_inert_examples();

    let out = render("php_smoke_only", Vec::new());

    assert!(
        out.contains("$this->assertNotNull($result);"),
        "the pre-existing smoke fallback must be untouched, got:\n{out}"
    );
    assert!(
        !out.contains("markTestSkipped('alef") && !out.contains("$this->fail('alef"),
        "a fixture with no assertions must never be refused, got:\n{out}"
    );
    assert!(take_inert_examples().is_empty());
}
