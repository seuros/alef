//! The `fields_method_calls`-declared tagged-union crossing walk `FieldResolver::classify` (via
//! [`FieldResolver::tagged_union_split`]) and `FieldResolver::result_field_oracle_knows` rely on,
//! split out of `classify.rs` because it earns its own concern: a single crossing's own payload
//! type can declare ANOTHER enum-typed field that is, in turn, its own declared crossing (a union
//! nested inside another union's payload -- `metadata.format.excel.sheet.kind` crossing `format`
//! and then `sheet`'s own union in turn), and resolving that chain is one recursive walk, not a
//! sequence of depth-specific special cases.
//!
//! ~keep Before this module existed, `tagged_union_method_call_declares` handled exactly ONE
//! crossing: it split the path at the first `fields_method_calls` entry, resolved the variant's
//! payload type, and handed the remaining suffix to a STRUCT-ONLY walk
//! (`ir_result_fields::type_declares_path`), which has no way to advance through a second
//! enum-typed field -- `field_types` only records struct-typed edges, so hitting a second union
//! mid-suffix made that walk abstain (`None`) even when the consumer's own `fields_method_calls`
//! named exactly how to cross it too. `None` there falls through to `path_crosses_unwalkable_field`
//! (`Some(false)`), so a path reaching a leaf through TWO declared crossings was refused and
//! dropped from generated doc snippets, and `validate_field_classifications` reported it
//! "unverified" -- the identical symptom `8c8718ed3` fixed for a single crossing, one union level
//! deeper.
use heck::ToUpperCamelCase;

use super::super::ir_collection::is_collection_path_from;
use super::super::ir_enum::enum_type_at_path_from;
use super::super::types::FieldResolver;

impl FieldResolver {
    /// Whether a field reached after narrowing `union_field` to `variant` is collection-typed.
    /// The enum map resolves the concrete payload owner; the collection map then walks the
    /// remaining payload-relative path, so no backend or consumer field name participates. ~keep
    pub fn union_variant_field_is_collection(&self, union_field: &str, variant: &str, field: &str) -> bool {
        let Some(union_type) = self.ir_enum_type_name(union_field) else {
            return false;
        };
        let variant = variant.to_upper_camel_case();
        let Some((_, payload_type)) = self.union_variant_payload(&union_type, &variant) else {
            return false;
        };
        is_collection_path_from(&self.ir_collection_map, payload_type, field)
    }

    /// The left-to-right scan [`Self::tagged_union_split`] does, generalized with `absolute_prefix`
    /// -- the fixture-root path already consumed by an earlier crossing, or `""` for the first one
    /// -- so [`Self::crossing_declares`] can reuse the exact same scan to find a SECOND (or third,
    /// ...) declared crossing further down a path, not just the first.
    ///
    /// `fields_method_calls` entries are always spelled relative to the call's own result root,
    /// never relative to an intermediate crossing's own payload type, which is why membership is
    /// checked against `absolute_prefix` joined with the scan's own progress rather than against
    /// `path`'s cumulative segments alone -- otherwise a second crossing nested inside the first
    /// one's payload could never match its own (root-relative) config entry. ~keep
    pub(super) fn find_crossing(&self, absolute_prefix: &str, path: &str) -> Option<(String, String, String)> {
        let segments: Vec<&str> = path.split('.').collect();
        let mut relative_so_far = String::new();
        for (i, seg) in segments.iter().enumerate() {
            if !relative_so_far.is_empty() {
                relative_so_far.push('.');
            }
            relative_so_far.push_str(seg);
            let absolute_so_far = if absolute_prefix.is_empty() {
                relative_so_far.clone()
            } else {
                format!("{absolute_prefix}.{relative_so_far}")
            };
            if self.method_calls.contains(&absolute_so_far) {
                let relative_prefix = segments[..i].join(".");
                let variant = (*seg).to_string();
                let relative_suffix = segments[i + 1..].join(".");
                return Some((relative_prefix, variant, relative_suffix));
            }
        }
        None
    }

    /// Whether the IR confirms a `fields_method_calls`-covered tagged-union crossing in
    /// `fixture_field`, and if so, whether the path segment(s) past the crossing are declared on
    /// the variant's own payload type -- walking through as many CHAINED crossings as the path and
    /// config declare, not just the first.
    ///
    /// `None` when `fixture_field` does not cross a method-call-covered union at all
    /// ([`Self::tagged_union_split`] found no covering entry), or when the IR cannot resolve the
    /// union type at the first crossing's prefix or its variant's single-field payload type --
    /// callers must fall back to their pre-existing unwalkable-field refusal in that case, exactly
    /// as if this method did not exist.
    pub(super) fn tagged_union_method_call_declares(&self, fixture_field: &str) -> Option<bool> {
        let (prefix, variant, suffix) = self.tagged_union_split(fixture_field)?;
        let root = self.ir_enum_map.root_type.as_deref()?;
        self.crossing_declares(root, "", &prefix, &variant, &suffix)
    }

    /// Resolve one validated `fields_method_calls` crossing -- `owner` reached via
    /// `relative_prefix` (ending at the enum-typed field itself), crossing into `variant`'s
    /// payload type -- and keep walking `relative_suffix` from there.
    ///
    /// Recurses into itself when the remaining suffix ALSO starts with a declared crossing: a
    /// union nested inside another union's own payload type is not a deeper case this needs a
    /// second code path for -- it is the exact same one-crossing step, applied again to whatever
    /// the previous crossing's payload type and remaining suffix are. Falls back to a plain
    /// struct-field walk ([`super::super::ir_result_fields::type_declares_path`]) once no further
    /// crossing is declared, exactly as the single-crossing case always has.
    ///
    /// `absolute_prefix` is the fixture-root path already consumed by an earlier crossing (or `""`
    /// for the first one) -- see [`Self::find_crossing`] for why it has to be threaded through
    /// rather than re-derived from `relative_prefix` alone. `variant` is pascal-cased before the
    /// [`Self::union_variant_payload`] lookup, matching every other caller of that method
    /// (gleam/dart/kotlin/swift assertion rendering): the fixture path spells the accessor segment
    /// lower (`excel`), but `variant_payload_types` is keyed by the Rust variant's own declared
    /// name (`Excel`).
    fn crossing_declares(
        &self,
        owner: &str,
        absolute_prefix: &str,
        relative_prefix: &str,
        variant: &str,
        relative_suffix: &str,
    ) -> Option<bool> {
        let union_type = enum_type_at_path_from(&self.ir_enum_map, owner, relative_prefix)?;
        let variant_pascal = variant.to_upper_camel_case();
        let payload_type = self.union_variant_payload(&union_type, &variant_pascal)?.1.to_string();
        if relative_suffix.is_empty() {
            return Some(true);
        }
        let crossed_absolute = Self::join_absolute(absolute_prefix, relative_prefix, variant);
        if let Some((next_prefix, next_variant, next_suffix)) = self.find_crossing(&crossed_absolute, relative_suffix)
            && let Some(result) = self.crossing_declares(
                &payload_type,
                &crossed_absolute,
                &next_prefix,
                &next_variant,
                &next_suffix,
            )
        {
            return Some(result);
        }
        super::super::ir_result_fields::type_declares_path(&self.ir_result_field_map, &payload_type, relative_suffix)
    }

    /// Join a fixture-root-relative crossing path back onto `absolute_prefix`, dropping empty
    /// segments so the very first crossing (`absolute_prefix == ""`) doesn't grow a leading dot.
    fn join_absolute(absolute_prefix: &str, relative_prefix: &str, variant: &str) -> String {
        [absolute_prefix, relative_prefix, variant]
            .into_iter()
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join(".")
    }
}
