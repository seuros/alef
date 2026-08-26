//! Per-call `FieldResolver` construction for `test_method.rs`.
//!
//! Extracted out of `test_method.rs`, which sits at the file-size ratchet's frozen ceiling
//! (`tests/file_size_baseline.txt`) and may not grow. Behavior is otherwise unchanged from the
//! block this replaces, plus `with_ir_result_fields`/`with_anchored_optional_paths`: without
//! `with_ir_result_fields`, `FieldResolver`'s `ir_result_field_map` keeps its default `root_type:
//! None`, which makes `with_anchored_optional_paths` an unconditional no-op (it early-returns on
//! an unresolved root) regardless of what paths it is given. With both wired in, an
//! `Option<Vec<T>>` segment field reached through an array-projected path (e.g.
//! `entries[0].sections`) — which never matches `with_ir_fields`'s bare-name-only optional set
//! once the path crosses more than one segment — resolves correctly, and
//! `with_anchored_optional_paths` materializes that IR-anchored answer for this fixture's own
//! assertion paths into the lookup set the per-segment accessor renderer consults, mirroring
//! `presentation.rs`'s existing use of it.

use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::collections::HashSet;

#[allow(clippy::too_many_arguments)]
pub(super) fn build_call_field_resolver(
    e2e_config: &E2eConfig,
    call_config: &CallConfig,
    fixture: &Fixture,
    lang: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> FieldResolver {
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_root_type = resolve_declared_result_type(call_config, lang, CallIr { functions, type_defs });
    FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_enum_fields(e2e_config.effective_fields_enum(call_config).clone())
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
    .with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), call_root_type.clone())
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, lang), call_root_type)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()))
}
