//! Regression coverage for the collection `is_empty` defect (B3): `field_needs_json_serialize`
//! only checked `field_resolver.is_array(f)`, which misses a collection field whose entries are
//! tracked only via their element paths in `fields_array` (e.g. `children[0]` for a recursive
//! `List<DataNode> Children`, never a bare `children` entry). Without `is_collection_root`,
//! `is_empty`'s template branch (`csharp/assertion.jinja`) fell through to
//! `Assert.True(string.IsNullOrEmpty(field.ToString()))` — `List<T>.ToString()` returns the
//! type name (a non-empty string), so the assertion could never pass. `Assert.Empty` is the
//! correct existing branch, gated on the wrong predicate.
//!
//! Lives in its own file rather than growing `csharp/assertions.rs`: that file is already over
//! the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md).

use super::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::HashSet;

/// `array_fields` carries only the element path (`children[0]`), matching how a real
/// consumer's `alef.toml` records a recursive-tree collection field — never a bare `children`
/// entry, which is exactly the shape `is_array` alone misses.
fn resolver_with_collection_root(field: &str) -> FieldResolver {
    let array_fields: HashSet<String> = [format!("{field}[0]")].into_iter().collect();
    FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &array_fields,
        &std::collections::HashSet::new(),
    )
}

fn is_empty_on_field(field: &str) -> Assertion {
    Assertion {
        assertion_type: "is_empty".to_string(),
        field: Some(field.to_string()),
        ..Default::default()
    }
}

/// The regression this file exists for: `is_empty` on a `List<DataNode> Children` field
/// (a collection-root, not a plain `fields_array` entry) must render `Assert.Empty`, not the
/// `ToString()`-based check that can never pass on a non-string collection.
#[test]
fn is_empty_on_collection_root_field_uses_assert_empty_not_tostring() {
    let resolver = resolver_with_collection_root("children");
    let assertion = is_empty_on_field("children");
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        "SampleException",
        &resolver,
        false,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        false,
    );
    assert!(
        out.contains("Assert.Empty("),
        "expected Assert.Empty for a collection-root field; got: {out}"
    );
    assert!(
        !out.contains("ToString()"),
        "collection field must not fall back to the ToString()-based empty check; got: {out}"
    );
}
