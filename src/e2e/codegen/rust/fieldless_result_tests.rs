//! Regression coverage for field assertions rendered against a FIELDLESS Rust result.
//!
//! Split into its own file rather than added to `rust/assertions.rs`: that file sits at the
//! ceiling `tests/file_size_baseline.txt` records for it (see `file-modularization` in
//! CLAUDE.md), so new coverage goes into a fresh module. ~keep
//!
//! The defect: a call returning `Result<bytes::Bytes>` has no fields at all, and
//! `FieldResolver::with_result_is_byte_payload` is the flag that says so — the same fact
//! `is_valid_for_result` already refuses every path on, and the one sixteen other backends drop
//! the assertion for. Rust's skip guard was short-circuited by `result_is_simple`, so the oracle
//! was never asked; the assertion then reached `render_not_empty_assertion`, which re-derives
//! `FieldResolver::accessor(field, ..)` for an optional-shaped path instead of reusing the
//! `field_access` the simple-result arm computed, and emitted `result.payload` on a `Bytes` value
//! — `error[E0609]: no field 'payload' on type 'bytes::Bytes'`, which fails the whole generated
//! e2e crate.

use std::collections::{HashMap, HashSet};

use super::assertions::render_assertion;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

fn set(values: &[&str]) -> HashSet<String> {
    values.iter().map(ToString::to_string).collect()
}

/// The consumer-reported config shape: the fixture's field is declared in `result_fields` and
/// `fields_optional` (which is what drives `render_not_empty_assertion` onto the accessor branch),
/// and the call's declared Rust return type is a raw byte payload.
fn resolver(optional: &[&str], result_fields: &[&str], is_byte_payload: bool) -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &set(optional),
        &set(result_fields),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_result_is_byte_payload(is_byte_payload)
}

fn render(field: &str, field_resolver: &FieldResolver, result_is_simple: bool) -> String {
    let assertion = Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some(field.to_string()),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample_module",
        "sample_dep",
        false,
        &[],
        field_resolver,
        false,
        result_is_simple,
        false,
        false,
        true,
        None,
    );
    out
}

#[test]
fn should_skip_the_assertion_when_a_simple_result_type_has_no_fields() {
    let out = render("payload", &resolver(&["payload"], &["payload"], true), true);
    assert_eq!(
        out,
        format!(
            "    // skipped: {}\n",
            FieldSkip::NotAvailableWhenResultIsSimple.message("payload")
        ),
        "a fieldless (byte-payload) result must route through the recorded skip funnel"
    );
    assert!(
        !out.contains("result.payload"),
        "`result.payload` on a `bytes::Bytes` value is E0609 in the generated crate; got: {out}"
    );
}

/// Control 1 — the fix must not be "skip everything". A real field on an ordinary struct result
/// still renders the member access.
#[test]
fn should_still_render_the_field_access_for_a_real_field_on_a_struct_result() {
    let out = render("payload", &resolver(&[], &["payload"], false), false);
    assert_eq!(
        out, "    assert!(!result.payload.is_empty(), \"expected non-empty value\");\n",
        "a struct result that declares the field must keep asserting on it"
    );
}

/// Control 2 — `result_is_simple` alone must keep its existing reinterpretation. A scalar (not
/// byte-payload) result still asserts on the whole value rather than skipping, so "skip whenever
/// `result_is_simple` is set" cannot pass this file either.
#[test]
fn should_assert_on_the_whole_value_when_a_simple_result_still_has_fields() {
    let out = render("payload", &resolver(&[], &["payload"], false), true);
    assert_eq!(
        out, "    assert!(!result.is_empty(), \"expected non-empty value\");\n",
        "a simple (non-fieldless) result reinterprets the field as the whole value"
    );
}

/// The second door into the same `E0609`: `render_not_empty_assertion` is the one renderer that
/// rebuilds its own accessor instead of using the `field_access` it was handed, and it took that
/// branch for any path the resolver reports optional. A `result_is_simple` call whose asserted
/// field is declared optional therefore emitted `result.payload` even without the byte-payload
/// flag set, so this case must hold whether or not the consumer declared `result_is_bytes`.
#[test]
fn should_not_rebuild_a_member_accessor_for_an_optional_field_on_a_simple_result() {
    let out = render("payload", &resolver(&["payload"], &["payload"], false), true);
    assert_eq!(
        out, "    assert!(!result.is_empty(), \"expected non-empty value\");\n",
        "the simple-result decision must survive into the not_empty renderer"
    );
    assert!(
        !out.contains("result.payload"),
        "a simple result has no `payload` member to reach; got: {out}"
    );
}

/// The `field == result_var` sentinel names the whole return value, which a fieldless result
/// answers perfectly well — skipping it would drop coverage the consumer does have.
#[test]
fn should_keep_the_whole_result_sentinel_on_a_fieldless_result() {
    let out = render("result", &resolver(&[], &[], true), true);
    assert_eq!(
        out, "    assert!(!result.is_empty(), \"expected non-empty value\");\n",
        "`field: \"result\"` names the byte payload itself, not a member of it"
    );
}
