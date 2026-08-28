//! Regression coverage for the element accessor a wildcard (`container[].field`) fixture path
//! expands to in the PHP e2e generator.
//!
//! ~keep PHP was reported CLEAN by a survey of one consumer corpus's generated output. It is not
//! structurally clean: its wildcard branch routed the element half through the same
//! result-anchored `FieldResolver::accessor` the six confirmed-defective backends used, against a
//! resolver its `call_field_resolver` anchors at the call's declared result type exactly as they
//! do. That corpus simply produced no `result_fields` envelope rescue for this backend. The
//! fixture below supplies one, and the assertions fail against the pre-fix generator — an
//! accident, not an immunity.
//!
//! Pre-fix the closure body was `$e->records[0]->kind`: the container path applied a second time
//! against a binding that is already an element.

use std::collections::BTreeMap;

use super::assertions::render_assertion;
use super::enum_variant_access::PhpVariantAccess;
use crate::e2e::codegen::wildcard_element_fixture::{
    WILDCARD_FIELD, assert_container_accessor_appears_once, assert_element_relative, contains_assertion,
    envelope_resolver, report_resolver,
};
use crate::e2e::field_access::FieldResolver;

const CONTAINER_ACCESSOR: &str = "->records";

fn render(resolver: &FieldResolver) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        &contains_assertion(WILDCARD_FIELD),
        "result",
        resolver,
        false,
        false,
        &BTreeMap::new(),
        false,
        &PhpVariantAccess::none(),
    );
    out
}

#[test]
fn wildcard_closure_body_is_relative_to_the_element_binding() {
    let rendered = render(&report_resolver("php"));

    assert_element_relative(&rendered, "(string)$e->kind", "$e->records[0]->kind");
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}

/// The container half must keep resolving against the result variable — the fix must not turn
/// into "never anchor", which would still satisfy the assertion above.
#[test]
fn wildcard_container_stays_anchored_to_the_result_variable() {
    let rendered = render(&report_resolver("php"));

    assert!(
        rendered.contains("array_filter($result->records, fn($e) =>"),
        "container half must filter the result variable's own field, got: {rendered}"
    );
}

/// The stronger container control: on an envelope root the container is reachable only THROUGH
/// the `result_fields` projection, so dropping the anchoring from `accessor` — rather than only
/// from the element half — renders `$result->records`, a member the envelope does not declare.
#[test]
fn wildcard_container_keeps_its_envelope_projection() {
    let rendered = render(&envelope_resolver("php"));

    assert!(
        rendered.contains("array_filter($result->results[0]->records, fn($e) =>"),
        "container half must keep the result-anchored envelope projection, got: {rendered}"
    );
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}
