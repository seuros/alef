//! Regression tests for a JSON key serde omits entirely (`#[serde(skip_serializing_if = "...")]`)
//! on a field that is NOT itself `Option<T>` -- e.g. a required `Vec<T>` skipped via
//! `Vec::is_empty`. `alef`'s IR previously had no concept of "required but conditionally
//! absent from the wire", so the Zig e2e generator's JSON-tree accessor chain always emitted
//! `.object.get("key").?`, which panics at runtime for exactly the values that legitimately
//! triggered the skip.
//!
//! Split into its own file rather than added to `zig/assertions.rs`: that file is already
//! over the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), so new test
//! coverage goes into a fresh module instead of growing it. ~keep

use super::assertions::render_json_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn resolver_with_wire_optional(field: &str) -> FieldResolver {
    let wire_optional: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_wire_optional_fields(wire_optional)
}

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn is_empty_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "is_empty".to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

/// The exact shape that crashed for tslp's `data_extraction_json_empty_object` fixture:
/// `DataNode.children: Vec<DataNode>` carries
/// `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so `process("{}", ...)` omits
/// the `"children"` key from the emitted JSON entirely -- not `null`, absent. Before the fix,
/// the generated accessor chain was `result.object.get("data").?.object.get("children").?`,
/// which panics with "attempt to use null value" on `.get("children").?` because there is no
/// entry for that key at all. The fix must render a guarded lookup for the wire-optional
/// segment instead. ~keep
#[test]
fn wire_optional_leaf_key_is_not_force_unwrapped() {
    let resolver = resolver_with_wire_optional("children");
    let assertion = is_empty_assertion("data.children");
    let mut out = String::new();

    render_json_assertion(&mut out, &assertion, "result", &resolver, false);

    assert!(
        !out.contains(".object.get(\"children\").?"),
        "a wire-optional key must not be force-unwrapped with `.?`, which panics when serde \
         omitted the key entirely. Rendered:\n{out}"
    );
    assert!(
        out.contains("(result.object.get(\"data\").object.get(\"children\") orelse .null)")
            || out.contains(".object.get(\"children\") orelse .null"),
        "a wire-optional key must be guarded with `orelse .null` so a missing key renders the \
         same as a present `null` value. Rendered:\n{out}"
    );
}

/// Companion negative case: a field the resolver does NOT know is wire-optional must keep the
/// existing `.?` force-unwrap -- the fix must not blanket-guard every JSON key, only ones the
/// IR actually marked `serde_skip_serializing_if`.
#[test]
fn non_wire_optional_leaf_key_keeps_force_unwrap() {
    let resolver = empty_resolver();
    let assertion = is_empty_assertion("data.children");
    let mut out = String::new();

    render_json_assertion(&mut out, &assertion, "result", &resolver, false);

    assert!(
        out.contains(".object.get(\"children\").?"),
        "a field not marked wire-optional must keep the plain `.?` accessor. Rendered:\n{out}"
    );
    assert!(
        !out.contains("orelse .null"),
        "a field not marked wire-optional must not be guarded. Rendered:\n{out}"
    );
}
