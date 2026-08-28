//! Regression coverage for the element accessor a wildcard (`container[].field`) fixture path
//! expands to in the TypeScript e2e generator — which emits BOTH the node and the wasm test
//! trees from this one function.
//!
//! Split into its own file rather than added to `typescript/assertions.rs`: that file sits at its
//! recorded ceiling in `tests/file_size_baseline.txt`, so new coverage goes into a fresh module
//! instead of growing it (see `file-modularization` in CLAUDE.md). ~keep
//!
//! The defect: the wildcard branch splits `records[].kind` into the container `records` and the
//! element half `kind`, then built the `.some(...)` closure body with `FieldResolver::accessor`,
//! which anchors a path against the call's RESULT type. `kind` is not declared on the root, so the
//! envelope rescue prefixed it back to `records[0].kind` and the closure body came out as
//! `e.records[0].kind` — the container path applied a second time against a binding that is
//! already an element.

use std::collections::HashMap;

use super::assertions::render_assertion;
use crate::e2e::codegen::wildcard_element_fixture::{
    WILDCARD_FIELD, assert_container_accessor_appears_once, assert_element_relative, contains_assertion,
    envelope_resolver, report_resolver,
};
use crate::e2e::field_access::FieldResolver;

const CONTAINER_ACCESSOR: &str = ".records";

fn render(resolver: &FieldResolver, lang: &str) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        &contains_assertion(WILDCARD_FIELD),
        "result",
        resolver,
        false,
        &HashMap::new(),
        lang,
        false,
        false,
        false,
    );
    out
}

#[test]
fn wildcard_closure_body_is_relative_to_the_element_binding() {
    let rendered = render(&report_resolver("typescript"), "node");

    assert_element_relative(&rendered, "String(e.kind)", "e.records[0].kind");
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}

/// The node and wasm trees are emitted by this one function, and its wildcard branch returns
/// before any `lang == "wasm"` rewrite and hardcodes `"typescript"` as the accessor language.
/// Pinning the two outputs equal is what makes "one fix covers both trees" a checked fact rather
/// than a reading of the control flow. ~keep
#[test]
fn wildcard_rendering_is_identical_for_the_node_and_wasm_trees() {
    let resolver = report_resolver("typescript");

    assert_eq!(
        render(&resolver, "node"),
        render(&resolver, "wasm"),
        "the wildcard branch must emit one expression for both output trees"
    );
}

/// The container half must keep resolving against the result variable — the fix must not turn
/// into "never anchor", which would still satisfy the assertion above.
#[test]
fn wildcard_container_stays_anchored_to_the_result_variable() {
    let rendered = render(&report_resolver("typescript"), "node");

    assert!(
        rendered.contains("(result.records ?? []).some((e) =>"),
        "container half must quantify over the result variable's own field, got: {rendered}"
    );
}

/// The stronger container control: on an envelope root the container is reachable only THROUGH
/// the `result_fields` projection, so dropping the anchoring from `accessor` — rather than only
/// from the element half — renders `result.records`, a member the envelope does not declare.
#[test]
fn wildcard_container_keeps_its_envelope_projection() {
    let rendered = render(&envelope_resolver("typescript"), "node");

    assert!(
        rendered.contains("(result.results[0].records ?? []).some((e) =>"),
        "container half must keep the result-anchored envelope projection, got: {rendered}"
    );
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}
