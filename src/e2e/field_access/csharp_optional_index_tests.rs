//! Null-forgiving placement must not depend on which shape follows the optional collection.
//!
//! ~keep `render_csharp_with_optionals` consulted `optional_fields` in its `Field` arm but not in
//! its `ArrayField` arm, so one emitted snippet could disagree with itself: a consumer's
//! `metadata-headings` snippet rendered `result.Metadata.Headings!.Count` on one line and
//! `result.Metadata.Headings[0].Level` on the next, and only the second is a `CS8602`. Indexing a
//! nullable collection dereferences it exactly as reading `.Count` does, so both arms have to ask
//! the same question of the same tracked key.

use super::FieldResolver;
use std::collections::{HashMap, HashSet};

fn resolver_with_optional(field: &str) -> FieldResolver {
    let optional: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

#[test]
fn indexing_an_optional_collection_emits_the_same_null_forgiving_operator_as_reading_its_count() {
    let resolver = resolver_with_optional("metadata.headings");

    let count = resolver.accessor("metadata.headings.length", "csharp", "result");
    let indexed = resolver.accessor("metadata.headings[0].level", "csharp", "result");

    assert_eq!(count, "result.Metadata.Headings!.Count");
    assert_eq!(
        indexed, "result.Metadata.Headings![0].Level",
        "indexing dereferences the nullable collection just as `.Count` does"
    );
}

/// The negative control: a collection nothing declares optional must not gain a `!`, or every
/// non-nullable access would carry a redundant operator (a `CS8600`-adjacent warning in strict
/// projects and pure noise in a documentation snippet). ~keep
#[test]
fn indexing_a_non_optional_collection_emits_no_null_forgiving_operator() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert_eq!(
        resolver.accessor("metadata.headings[0].level", "csharp", "result"),
        "result.Metadata.Headings[0].Level"
    );
}
