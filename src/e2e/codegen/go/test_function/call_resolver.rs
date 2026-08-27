//! Per-call `FieldResolver` construction for the Go e2e generator.
//!
//! Split out of `test_function.rs`, which is over the 1,000-line cap and may not grow.

use crate::core::config::e2e::{CallConfig, E2eConfig};
use crate::core::ir::{FunctionDef, TypeDef};
use crate::e2e::field_access::FieldResolver;

pub(super) const LANG: &str = "go";

/// Build the field resolver for one call, anchored at the call's declared Rust return type.
///
/// Anchoring `with_ir_result_fields` mirrors the rust/python/java/csharp/elixir generators and is
/// purely additive: `result_field_oracle_knows` only ever REFUSES what it positively knows the
/// root type lacks, so an unresolved root leaves every anchored answer disabled. ~keep
pub(super) fn build_call_field_resolver(
    e2e_config: &E2eConfig,
    call_config: &CallConfig,
    functions: &[FunctionDef],
    type_defs: &[TypeDef],
) -> FieldResolver {
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_root_type = crate::e2e::codegen::call_ir::resolve_declared_result_type(
        call_config,
        LANG,
        crate::e2e::codegen::call_ir::CallIr { functions, type_defs },
    );
    FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, LANG), call_root_type)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_result_is_byte_payload(call_config.effective_result_is_bytes(LANG))
}
