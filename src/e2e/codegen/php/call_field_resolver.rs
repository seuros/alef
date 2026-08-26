//! Per-call `FieldResolver` construction for `test_method.rs`.
//!
//! Extracted out of `test_method.rs`, which sits at the file-size ratchet's frozen ceiling
//! (`tests/file_size_baseline.txt`) and may not grow. Behavior is otherwise unchanged from the
//! block this replaces, plus `with_anchored_optional_paths`: without it, an `Option<Vec<T>>`
//! segment field reached through an array-projected path (e.g. `entries[0].sections`) never
//! matches `with_ir_fields`'s bare-name-only optional set once the path crosses more than one
//! segment, so the per-segment accessor renderer emits an un-unwrapped index/`.length` access.
//! `with_anchored_optional_paths` materializes the IR-anchored answer for this fixture's own
//! assertion paths into the same lookup set, mirroring `presentation.rs`'s existing use of it.

use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::{FieldResolver, PhpGetterMap};
use crate::e2e::fixture::Fixture;
use std::collections::{HashMap, HashSet};

pub(super) fn build_call_field_resolver(
    e2e_config: &E2eConfig,
    call_config: &CallConfig,
    fixture: &Fixture,
    type_defs: &[crate::core::ir::TypeDef],
    per_call_getter_map: &PhpGetterMap,
) -> FieldResolver {
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    FieldResolver::new_with_php_getters(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &HashSet::new(),
        &HashMap::new(),
        per_call_getter_map.clone(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()))
}
