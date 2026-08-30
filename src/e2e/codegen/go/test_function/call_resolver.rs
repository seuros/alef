//! Per-call `FieldResolver` construction for the Go e2e generator.
//!
//! Split out of `test_function.rs` to keep result-shape resolution focused.

use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::{CallConfig, E2eConfig};
use crate::core::ir::{EnumDef, FunctionDef, TypeDef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;

pub(super) const LANG: &str = "go";

pub(in crate::e2e::codegen::go) fn fixture_has_go_callable(fixture: &Fixture, e2e_config: &E2eConfig) -> bool {
    if fixture.is_http_test() {
        return false;
    }
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    if call.skip_languages.iter().any(|language| language == LANG) {
        return false;
    }
    let override_config = call.overrides.get(LANG).or_else(|| e2e_config.call.overrides.get(LANG));
    if override_config
        .and_then(|config| config.client_factory.as_deref())
        .is_some()
    {
        return true;
    }
    let function = override_config
        .and_then(|config| config.function.as_deref())
        .filter(|function| !function.is_empty())
        .unwrap_or(call.function.as_str());
    !function.is_empty()
}

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
    enums: &[EnumDef],
    config: &ResolvedCrateConfig,
) -> FieldResolver {
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_root_type = crate::e2e::codegen::call_ir::resolve_declared_result_type(
        call_config,
        LANG,
        crate::e2e::codegen::call_ir::CallIr { functions, type_defs },
    );
    let excluded_names = go_excluded_type_names(config, type_defs);
    FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_ir_result_fields(
        FieldResolver::go_ir_result_field_facts(type_defs, enums, &excluded_names),
        call_root_type,
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_result_is_byte_payload(call_config.effective_result_is_bytes(LANG))
}

fn go_excluded_type_names(config: &ResolvedCrateConfig, type_defs: &[TypeDef]) -> std::collections::HashSet<String> {
    let has_bridge_params = config.trait_bridges.iter().any(|bridge| bridge.param_name.is_some());
    let has_options_bridge = config.trait_bridges.iter().any(|bridge| {
        bridge.bind_via == crate::core::config::BridgeBinding::OptionsField && bridge.is_active_for(LANG)
    });
    let mut excluded_names = if has_bridge_params || has_options_bridge {
        config.bridge_associated_types()
    } else {
        std::collections::HashSet::new()
    };
    if let Some(ffi) = &config.ffi {
        excluded_names.extend(ffi.exclude_types.iter().cloned());
    }
    if let Some(go) = &config.go {
        excluded_names.extend(go.exclude_types.iter().cloned());
    }
    excluded_names.extend(
        type_defs
            .iter()
            .filter(|type_def| type_def.binding_excluded)
            .map(|type_def| type_def.name.clone()),
    );
    excluded_names.extend(
        config
            .opaque_types
            .iter()
            .filter(|(_, path)| path.contains('<'))
            .map(|(name, _)| name.clone()),
    );
    excluded_names
}
