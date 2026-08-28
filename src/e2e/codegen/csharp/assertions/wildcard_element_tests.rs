//! Regression coverage for the element accessor a wildcard (`container[].field`) fixture path
//! expands to in the C# e2e generator.
//!
//! ~keep Registered from the sibling `wildcard_traversal_tests.rs` rather than from `csharp.rs`
//! or `csharp/assertions.rs`: both of those are AT their recorded ceilings in
//! `tests/file_size_baseline.txt` (1142 and 1312) and the `file-modularization` rule forbids
//! growing them — even by a two-line `mod` declaration.
//!
//! The defect: `render_wildcard_assertion` splits `records[].kind` into the container `records`
//! and the element half `kind`, then built the `Any` lambda body with `FieldResolver::accessor`,
//! which anchors a path against the call's RESULT type. `kind` is not declared on the root, so the
//! envelope rescue prefixed it back to `records[0].kind` and the lambda body came out as
//! `e….Records[0].Kind` — the container path applied a second time against a binding that is
//! already an element.
//!
//! ~keep The lambda parameter is a hash of the assertion, so the assertions below are written
//! against the container accessor's occurrence count rather than a spelled-out parameter name:
//! `.Records` belongs to the `Any` receiver and to nothing else.

use std::collections::HashMap;

use crate::e2e::codegen::csharp::assertions::render_assertion;
use crate::e2e::codegen::wildcard_element_fixture::{
    WILDCARD_FIELD, assert_container_accessor_appears_once, contains_assertion, envelope_resolver, report_resolver,
};
use crate::e2e::field_access::FieldResolver;

const CONTAINER_ACCESSOR: &str = ".Records";

fn render(resolver: &FieldResolver) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        &contains_assertion(WILDCARD_FIELD),
        "result",
        "SampleTests",
        "SampleException",
        resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        false,
    );
    out
}

#[test]
fn wildcard_lambda_body_is_relative_to_the_element_binding() {
    let rendered = render(&report_resolver("csharp"));

    assert!(
        !rendered.contains(".Records[0].Kind"),
        "lambda body must not re-apply the container path to the element binding \
         (`.Records[0]` addresses a member the element type does not declare), got: {rendered}"
    );
    assert!(
        rendered.contains(".Kind)!.Contains("),
        "lambda body must address the element binding's own field, got: {rendered}"
    );
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}

/// The container half must keep resolving against the result variable — the fix must not turn
/// into "never anchor", which would still satisfy the assertion above.
#[test]
fn wildcard_container_stays_anchored_to_the_result_variable() {
    let rendered = render(&report_resolver("csharp"));

    assert!(
        rendered.contains("result.Records?.Any("),
        "container half must quantify over the result variable's own field, got: {rendered}"
    );
}

/// The stronger container control: on an envelope root the container is reachable only THROUGH
/// the `result_fields` projection, so dropping the anchoring from `accessor` — rather than only
/// from the element half — renders `result.Records`, a member the envelope does not declare.
#[test]
fn wildcard_container_keeps_its_envelope_projection() {
    let rendered = render(&envelope_resolver("csharp"));

    assert!(
        rendered.contains("result.Results[0].Records?.Any("),
        "container half must keep the result-anchored envelope projection, got: {rendered}"
    );
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}
