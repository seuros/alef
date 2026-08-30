//! The single wire-value-to-Rust-identifier oracle for Rust e2e assertions.
//!
//! Every Rust assertion that inspects an enum-typed field renders it through `Debug` — the only
//! trait an arbitrary IR enum is guaranteed to implement — and `Debug` prints the RUST
//! identifier. A fixture's expectation records the SERDE WIRE value. The two spellings coincide
//! only while no `#[serde(rename)]` / `#[serde(rename_all)]` moves the variant off its
//! identifier. `equals`, the containment operators (`contains`, `contains_all`, `not_contains`,
//! `contains_any`) and the wildcard element renderer therefore have the identical surface
//! mismatch and must be reconciled by the identical lookup, so that one enum cannot be
//! translated one way by `assert_eq!` and another way by `assert!(..contains(..))`.
//!
//! ~keep The containment predicate case-folds BOTH sides (`format!("{:?}", f).to_lowercase()
//! .contains(&EXPECTED.to_lowercase())`), which changes what translation buys but never makes it
//! unsafe:
//!
//! * A rename that differs from the identifier only by case (`Plain` -> `"plain"`) is recorded
//!   by the map — the strings differ — yet translating it cannot change the predicate's truth
//!   value, because both spellings fold to the same lowercase needle. Such a rename is a no-op
//!   here and a real fix for `equals`, which does not case-fold.
//! * Case folding can make two spellings collide that the map's exact-string exclusions let
//!   through (variant `Foo` renamed to `"bar"` alongside a variant `BAR`). Untranslated, that
//!   fixture matched `BAR` and missed `Foo`, i.e. exactly the wrong variant; translated, it
//!   matches `Foo`. Folding therefore never turns a recorded rename into a worse answer.
//!
//! No extra case-insensitive exclusion is applied on top of the map, deliberately: the map is
//! the one oracle, and adding a containment-only rule would reintroduce the two-generators
//! disagreement this module exists to remove.

use crate::e2e::escape::rust_raw_string;
use crate::e2e::field_access::FieldResolver;

use super::assertion_synthetic::value_to_rust_string;

/// Rewrite an enum field's fixture expectation from the serde WIRE value to the Rust variant
/// identifier, so it compares like-for-like against the `format!("{:?}", ..)` expression the
/// enum branches of the assertion renderers emit.
///
/// Returns `None` — leaving the fixture literal exactly as authored — whenever the IR cannot
/// resolve the field to a concrete enum, the expectation is not a string, or no serde rename
/// separates the wire spelling from the identifier. That last case is the idiomatic one and is
/// already correct untranslated, which is what makes this purely additive: it can only change
/// output for a field the IR positively resolves to an enum with a renamed variant matching the
/// fixture value.
pub(super) fn renamed_variant_expected(
    field: Option<&str>,
    value: &serde_json::Value,
    field_resolver: &FieldResolver,
) -> Option<String> {
    let wire = value.as_str()?;
    let variant = field_resolver.enum_variant_for_wire_value(field?, wire)?;
    Some(rust_raw_string(variant))
}

/// The Rust literal a containment operator should search for, given a fixture value.
///
/// `enum_field` is `Some(path)` only when the assertion's field is enum-typed; callers pass
/// `None` for every other field kind so a collection or string field can never be routed
/// through the enum rename lookup. On any miss this is exactly [`value_to_rust_string`], the
/// pre-existing behaviour, so a fixture value that names no renamed variant still produces the
/// assertion the fixture author wrote — including one that legitimately fails.
///
/// ~keep The wildcard caller (`structure[].kind`) passes the WHOLE fixture path, never the
/// element half on its own, even though `wildcard_elem_is_enum` accepts either spelling for the
/// coarser "is this an enum at all" question. The bare leaf name is resolved against the call's
/// ROOT type, where it either misses or — worse — hits a same-named field on an unrelated owner
/// and answers with that other enum's rename table; the full path is what the IR walk is built
/// to traverse. Narrowing a rename to the wrong enum is precisely the silent retarget the map's
/// collision exclusion exists to prevent, so it must not be reintroduced by the path selection.
pub(super) fn containment_expected(
    value: &serde_json::Value,
    enum_field: Option<&str>,
    field_resolver: &FieldResolver,
) -> String {
    renamed_variant_expected(enum_field, value, field_resolver).unwrap_or_else(|| value_to_rust_string(value))
}
