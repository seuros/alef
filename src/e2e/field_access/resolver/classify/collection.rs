//! IR-derived collection-field classification.
//!
//! Split out of `classify.rs` at the concept boundary -- both methods here answer a
//! collection-shape question straight from `ir_collection`: whether a field is itself the root
//! of a `Vec`/array ([`FieldResolver::is_collection_root`]) and, when it is, what element type
//! it holds ([`FieldResolver::collection_element_type`]). `FieldResolver::is_array` (which also
//! consults `ir_collection::is_collection_path`) stays in `classify.rs` itself: it answers a
//! narrower, config-first question (`fields_array` membership before any IR fallback) that call
//! sites reach for directly, rather than composing with either method here. ~keep

use super::super::super::ir_collection::{
    element_type_at_path, has_non_string_scalar_elements_at_path, is_collection_path,
};
use super::super::super::types::FieldResolver;

impl FieldResolver {
    /// Check if a field name is the root of a collection type (i.e., the field
    /// itself returns a `Vec`/array, even though it is not in `fields_array`
    /// directly).
    ///
    /// `fields_array` tracks traversal paths like `choices[0].message.tool_calls`
    /// — the array element paths — not the bare collection accessor (`choices`).
    /// `fields_optional` may also contain paths like `data[0].url` that reveal
    /// `data` is a collection root.
    ///
    /// Returns `true` when any entry in `array_fields` or `optional_fields`
    /// starts with `{field}[`, indicating that `field` is the top-level
    /// collection getter — and, when config is silent, falls back to the
    /// IR-derived classification (`with_ir_collection_map`) the same way `is_enum` falls back
    /// to `with_ir_enum_map`. A field with no per-element path declared anywhere in the fixture
    /// suite (e.g. a recursive `List<T> Children` nothing ever indexes into) has no config
    /// signal at all, so without this fallback a caller deciding whether to serialize the field
    /// for `is_empty`/`contains` would wrongly fall through to a raw `ToString()`-style check.
    /// ~keep
    pub fn is_collection_root(&self, field: &str) -> bool {
        let prefix = format!("{field}[");
        if self.array_fields.iter().any(|af| af.starts_with(&prefix))
            || self.optional_fields.iter().any(|of| of.starts_with(&prefix))
        {
            return true;
        }
        let resolved = self.resolve(field);
        is_collection_path(&self.ir_collection_map, resolved)
    }

    /// The IR type name `field`'s elements are, when `field` is a collection reachable from the
    /// call's own anchored root — e.g. `"rows"` on a `Vec<Row>` field resolves to `"Row"`.
    ///
    /// Used to anchor validation of an `Iterate` operation's per-item field names against the
    /// LOOP ITEM's own type, rather than the call's result type: `default_operations_from_assertions`
    /// already documents why the latter is the wrong anchor for them. `None` when the IR cannot
    /// resolve the element type (no anchored root, an unrecognized field, a collection of a
    /// scalar or foreign type) — callers must treat that as "no answer, don't reject" like every
    /// other IR oracle here.
    pub fn collection_element_type(&self, field: &str) -> Option<String> {
        let resolved = self.resolve(field);
        element_type_at_path(&self.ir_collection_map, resolved)
    }

    /// Whether `field` is a collection whose elements are numeric, boolean or `char` — values
    /// with no text inside them to search.
    ///
    /// [`Self::collection_element_type`] cannot answer this. It resolves only struct-to-struct
    /// edges, so it returns `None` for `Vec<u32>`, for `Vec<String>` and for a field the IR has
    /// never heard of, all alike. A caller deciding whether a string expectation may be lowered
    /// as substring containment over the elements needs those three cases apart, and reading
    /// `None` as "scalar" would sweep the other two in with it.
    ///
    /// `false` means "no positive evidence", never "known to be textual" — the same convention
    /// every other IR oracle on this resolver uses, so a call site with no anchored root keeps
    /// exactly the behaviour it had. ~keep
    pub fn collection_element_is_non_string_scalar(&self, field: &str) -> bool {
        let resolved = self.resolve(field);
        has_non_string_scalar_elements_at_path(
            &self.ir_collection_map,
            &self.non_string_scalar_collection_fields,
            resolved,
        )
    }
}
