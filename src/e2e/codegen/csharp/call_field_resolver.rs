//! Per-call `FieldResolver` construction for `csharp.rs::render_test_method`.
//!
//! Extracted out of `csharp.rs`, which sits at the file-size ratchet's frozen ceiling
//! (`tests/file_size_baseline.txt`) and may not grow. Behavior is otherwise unchanged from the
//! block this replaces, plus `with_anchored_optional_paths`: without it, an `Option<Vec<T>>`
//! segment field reached through an array-projected path (e.g. `entries[0].sections`) never
//! matches `with_ir_fields`'s bare-name-only optional set once the path crosses more than one
//! segment, so the per-segment accessor renderer emits an un-unwrapped index/null-forgiving
//! access. `with_anchored_optional_paths` materializes the IR-anchored answer for this
//! fixture's own assertion paths into the same lookup set, mirroring `presentation.rs`'s
//! existing use of it.

use super::values;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;

#[allow(clippy::too_many_arguments)]
pub(super) fn build_call_field_resolver(
    e2e_config: &E2eConfig,
    call_config: &CallConfig,
    fixture: &Fixture,
    enum_fields: &HashMap<String, String>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> FieldResolver {
    let cs_overrides = call_config.overrides.get("csharp");
    // See `values::effective_csharp_enum_fields` for why this must be an effective (per-call
    // override wins, not merges) set rather than the raw global `fields_enum`.
    let effective_enum_fields =
        values::effective_csharp_enum_fields(e2e_config, call_config, enum_fields, cs_overrides);

    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_root_type = values::resolve_csharp_call_root_type(call_config, type_defs, functions);
    FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_enum_fields(effective_enum_fields)
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
    .with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), call_root_type.clone())
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts(type_defs, "csharp"),
        call_root_type,
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()))
}
