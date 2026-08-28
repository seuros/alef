//! Regression coverage for the element accessor a wildcard (`container[].field`) fixture path
//! expands to in the Python e2e generator.
//!
//! ~keep Registered from the sibling `wildcard_tests.rs` rather than from `python/mod.rs`,
//! because that file is AT its recorded ceiling in `tests/file_size_baseline.txt` (1373) and the
//! `file-modularization` rule forbids growing it — even by a two-line `mod` declaration. The same
//! constraint is why this is a new file instead of an addition to `python/assertions.rs`.
//!
//! The defect: `render_python_wildcard_assertion` splits `records[].kind` into the container
//! `records` and the element half `kind`, then built the comprehension body with
//! `FieldResolver::accessor`, which anchors a path against the call's RESULT type. `kind` is not
//! declared on the root, so the envelope rescue prefixed it back to `records[0].kind` and the
//! comprehension body came out as `_e.records[0].kind` — the container path applied a second time
//! against a binding that is already an element.

use std::collections::{HashMap, HashSet};

use crate::e2e::codegen::python::assertions::render_assertion;
use crate::e2e::codegen::wildcard_element_fixture::{
    WILDCARD_FIELD, assert_container_accessor_appears_once, assert_element_relative, contains_assertion,
    envelope_resolver, report_resolver,
};
use crate::e2e::field_access::FieldResolver;

const CONTAINER_ACCESSOR: &str = ".records";

fn render(resolver: &FieldResolver) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        &contains_assertion(WILDCARD_FIELD),
        "result",
        resolver,
        &HashSet::new(),
        &HashMap::new(),
        false,
    );
    out
}

#[test]
fn wildcard_comprehension_body_is_relative_to_the_element_binding() {
    let rendered = render(&report_resolver("python"));

    assert_element_relative(&rendered, "str(_e.kind)", "_e.records[0].kind");
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}

/// The container half must keep resolving against the result variable — the fix must not turn
/// into "never anchor", which would still satisfy the assertion above.
#[test]
fn wildcard_container_stays_anchored_to_the_result_variable() {
    let rendered = render(&report_resolver("python"));

    assert!(
        rendered.contains("for _e in (result.records or [])"),
        "container half must iterate the result variable's own field, got: {rendered}"
    );
}

/// The stronger container control: on an envelope root the container is reachable only THROUGH
/// the `result_fields` projection, so dropping the anchoring from `accessor` — rather than only
/// from the element half — renders `result.records`, a member the envelope does not declare.
#[test]
fn wildcard_container_keeps_its_envelope_projection() {
    let rendered = render(&envelope_resolver("python"));

    assert!(
        rendered.contains("for _e in (result.results[0].records or [])"),
        "container half must keep the result-anchored envelope projection, got: {rendered}"
    );
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}
