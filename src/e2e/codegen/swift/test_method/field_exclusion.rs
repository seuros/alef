//! Whether an assertion path names a field or type the Swift binding excludes.
//!
//! Split out of `test_method.rs` at the concept boundary: this answers one question
//! ("is this path excluded?") from the two `[languages.swift]` exclusion maps, and shares
//! nothing with the rendering logic around it beyond the maps themselves. ~keep

use std::collections::{HashMap, HashSet};

/// Returns `true` when `field_path` traverses a field or resolves to a type excluded from Swift.
///
/// Walks the dot/bracket-separated path segments through `field_types` (owner type → field name
/// → inner named type, populated in `build_swift_first_class_map`) from `root_type`, checking at
/// each segment whether `(current_type, segment)` appears in `excluded_fields_by_type` (from
/// `[languages.swift].exclude_fields` entries spelled `"TypeName.field_name"`) or the type the
/// segment advances into appears in `excluded_types` (from `[languages.swift].exclude_types`).
///
/// Returns `false` when both maps are empty, or the walk reached every segment and found nothing.
///
/// ~keep When the walk cannot answer for every segment — no `root_type`, or an unrecorded hop — it
/// falls back to matching a bare segment name against the union of every excluded type's field
/// set. That fallback used to run FIRST and unconditionally, so one
/// `exclude_fields` entry naming a leaf that other, un-excluded types also declare dropped every
/// assertion whose path merely contains that name — enough to void a whole fixture category, and
/// reported as `ExcludedFromSwiftBinding`, a verdict that reads as a deliberate config decision
/// rather than the misclassification it is. The precise walk could never correct it because it
/// never ran; ordering it last keeps the net where it was load-bearing and nowhere else.
pub(super) fn is_assertion_field_swift_excluded(
    field_path: &str,
    root_type: Option<&str>,
    field_types: &HashMap<String, HashMap<String, String>>,
    excluded_fields_by_type: &HashMap<String, HashSet<String>>,
    excluded_types: &HashSet<String>,
) -> bool {
    if excluded_fields_by_type.is_empty() && excluded_types.is_empty() {
        return false;
    }
    // Split the field path on '.', '[', ']', discarding empty tokens and tokens
    // that are pure numeric indices (e.g. "0" from "results[0].extracted_keywords").
    let segments: Vec<&str> = field_path
        .split(['.', '[', ']'])
        .filter(|s| !s.is_empty() && !s.chars().all(|c: char| c.is_ascii_digit()))
        .collect();
    // Type-aware walk (precise when `root_type` is known).
    let mut current_type: Option<String> = root_type.map(|s| s.to_string());
    let mut every_segment_walked = current_type.is_some();
    for &segment in &segments {
        let Some(owner_str) = current_type.as_deref() else {
            every_segment_walked = false;
            break;
        };
        // 1. Explicitly excluded (owner_type, field_name) pair.
        if excluded_fields_by_type
            .get(owner_str)
            .is_some_and(|fields| fields.contains(segment))
        {
            return true;
        }
        // Advance the type cursor to the named type that `segment` leads into.
        let next: Option<String> = field_types.get(owner_str).and_then(|m| m.get(segment).cloned());
        // 2. The resolved target type is excluded from the Swift binding.
        if let Some(ref next_type) = next
            && excluded_types.contains(next_type.as_str())
        {
            return true;
        }
        current_type = next;
    }
    if every_segment_walked {
        return false;
    }
    // Type-blind name fallback; see this function's doc for why it must stay last. ~keep
    segments
        .iter()
        .any(|segment| excluded_fields_by_type.values().any(|fields| fields.contains(*segment)))
}
