//! Regression coverage for the element accessor a wildcard (`container[].field`) fixture path
//! expands to in the Java e2e generator.
//!
//! Split into its own file rather than added to `java/assertions.rs`: that file sits at its
//! recorded ceiling in `tests/file_size_baseline.txt`, so new coverage goes into a fresh module
//! instead of growing it (see `file-modularization` in CLAUDE.md). ~keep
//!
//! The defect: `assertion_wildcard::render_wildcard_assertion` splits `records[].kind` into the
//! container `records` and the element half `kind`, then built the `anyMatch` lambda body with
//! `FieldResolver::accessor`, which anchors a path against the call's RESULT type. `kind` is not
//! declared on the root, so the envelope rescue prefixed it back to `records[0].kind` and the
//! lambda body came out as `e….records().get(0).kind()` — the container path applied a second
//! time against a binding that is already an element.
//!
//! ~keep The lambda parameter is a hash of the assertion, so the assertions below are written
//! against the container accessor's occurrence count rather than a spelled-out parameter name:
//! `.records()` belongs to the stream expression and to nothing else.

use std::collections::{HashMap, HashSet};

use super::assertions::render_assertion;
use crate::e2e::codegen::wildcard_element_fixture::{
    WILDCARD_FIELD, assert_container_accessor_appears_once, contains_assertion, envelope_resolver, report_resolver,
};
use crate::e2e::field_access::FieldResolver;

const CONTAINER_ACCESSOR: &str = ".records()";

fn render(resolver: &FieldResolver) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        &contains_assertion(WILDCARD_FIELD),
        "result",
        "SampleTest",
        resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        false,
    );
    out
}

#[test]
fn wildcard_lambda_body_is_relative_to_the_element_binding() {
    let rendered = render(&report_resolver("java"));

    assert!(
        !rendered.contains(".records().get(0).kind()"),
        "lambda body must not re-apply the container path to the element binding \
         (`.records().get(0)` addresses a member the element type does not declare), got: {rendered}"
    );
    assert!(
        rendered.contains(".kind()).contains("),
        "lambda body must address the element binding's own field, got: {rendered}"
    );
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}

/// The container half must keep resolving against the result variable — the fix must not turn
/// into "never anchor", which would still satisfy the assertion above.
#[test]
fn wildcard_container_stays_anchored_to_the_result_variable() {
    let rendered = render(&report_resolver("java"));

    assert!(
        rendered.contains("result.records().stream().anyMatch("),
        "container half must stream the result variable's own field, got: {rendered}"
    );
}

/// The stronger container control: on an envelope root the container is reachable only THROUGH
/// the `result_fields` projection, so dropping the anchoring from `accessor` — rather than only
/// from the element half — renders `result.records()`, a member the envelope does not declare.
#[test]
fn wildcard_container_keeps_its_envelope_projection() {
    let rendered = render(&envelope_resolver("java"));

    assert!(
        rendered.contains("result.results().get(0).records().stream().anyMatch("),
        "container half must keep the result-anchored envelope projection, got: {rendered}"
    );
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}
