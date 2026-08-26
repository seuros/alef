//! Whether an `Iterate` operation's per-item field is safe to format with Rust's `{}` (`Display`)
//! rather than `{:?}` (`Debug`).
//!
//! Split out of `classify.rs` (which was already at the file-modularization cap) rather than
//! grown there. [`FieldResolver::is_display_unsafe`] answers the whole-operation question for a
//! `Show` or a `fields`-less `Iterate`, anchored at the call's own declared result type; it has no
//! answer for an `Iterate`'s per-item `fields`, which are rooted at the LOOP ITEM type instead (see
//! `presentation::downgrade_display_unsafe_operations`'s doc comment). This is that missing
//! answer, keyed by the element type `FieldResolver::collection_element_type` already resolves.

use super::super::types::FieldResolver;

impl FieldResolver {
    /// Whether `field_name` is a member of `type_name` whose declared Rust type alef can
    /// positively vouch for as implementing `Display` — a bare `String`, `char`, or numeric/`bool`
    /// primitive, per
    /// [`ir_result_fields::type_ref_is_display_safe`](super::super::ir_result_fields::type_ref_is_display_safe).
    ///
    /// An ALLOWLIST answer, not an existence oracle: `false` covers both "not declared at all" and
    /// "declared, but with a type this allowlist does not vouch for" (a collection, `Option<_>`, a
    /// `Named` struct/enum, or any other opaque/wrapped shape) without distinguishing them, because
    /// a per-item field formatter that guesses `{}` wrong is a snippet that fails to compile —
    /// unlike every existence-only oracle in `classify.rs`, "no answer" here means "not safe", not
    /// "don't reject".
    pub fn is_declared_field_display_safe(&self, type_name: &str, field_name: &str) -> bool {
        self.ir_result_field_map
            .display_safe_fields
            .get(type_name)
            .is_some_and(|fields| fields.contains(field_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};
    use std::collections::{HashMap, HashSet};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn resolver_for(type_defs: &[TypeDef]) -> FieldResolver {
        let map = FieldResolver::ir_result_field_facts(type_defs, "rust");
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_result_fields(map, None)
    }

    fn table_type_defs() -> Vec<TypeDef> {
        vec![TypeDef {
            name: "Table".to_string(),
            fields: vec![
                field("name", TypeRef::String),
                field("row_count", TypeRef::Primitive(PrimitiveType::U32)),
                field("cells", TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))))),
                field("owner", TypeRef::Named("Person".to_string())),
            ],
            ..TypeDef::default()
        }]
    }

    /// The control: a bare scalar field is positively vouched for.
    #[test]
    fn a_string_field_is_display_safe() {
        let resolver = resolver_for(&table_type_defs());
        assert!(resolver.is_declared_field_display_safe("Table", "name"));
        assert!(resolver.is_declared_field_display_safe("Table", "row_count"));
    }

    /// The fix: a `Vec<Vec<String>>` field is refused even though it never appears in
    /// `field_types` (nothing peels `Vec` when checking `Named`-ness) and would therefore have
    /// read as "safe" under the whole-operation oracle.
    #[test]
    fn a_nested_vec_field_is_not_display_safe() {
        let resolver = resolver_for(&table_type_defs());
        assert!(!resolver.is_declared_field_display_safe("Table", "cells"));
    }

    /// A `Named` struct field is refused too, the same as the whole-operation oracle already
    /// refuses it, just answered by the allowlist instead of the `field_types` presence check.
    #[test]
    fn a_named_struct_field_is_not_display_safe() {
        let resolver = resolver_for(&table_type_defs());
        assert!(!resolver.is_declared_field_display_safe("Table", "owner"));
    }

    /// No answer (an undeclared field, or a type this resolver never saw) must default to unsafe,
    /// the opposite direction from every existence-only oracle in `classify.rs` — guessing "safe"
    /// wrong here is a compile failure, so silence must not read as permission.
    #[test]
    fn an_unknown_field_or_type_is_not_display_safe() {
        let resolver = resolver_for(&table_type_defs());
        assert!(!resolver.is_declared_field_display_safe("Table", "nonexistent"));
        assert!(!resolver.is_declared_field_display_safe("UnknownType", "name"));
    }
}
