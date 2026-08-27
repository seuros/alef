use crate::codegen::cfg::is_host_owned_rust_path;
use crate::core::ir::EnumDef;

use super::ConversionConfig;
use super::helpers::{binding_to_core_match_arm_ext_cfg, core_enum_path_remapped, core_to_binding_match_arm_ext_cfg};

/// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
/// cfg gate; the generated binding crate never declares a Cargo feature for it (see
/// `codegen::cfg::collect_cfg_gates`, the same authority this asks), so forwarding it verbatim as
/// `#[cfg(feature = "...")]` is an `unexpected cfg condition value` error. Such an arm is dropped
/// entirely instead -- named and counted via `tracing::warn!`, not silently -- mirroring
/// `backends::ffi::gen_bindings::types::gen_enum_from_i32_rs_helper` and
/// `backends::swift::gen_rust_crate::enums::emit_enum_wrapper`. A host-owned cfg keeps its arm and
/// its `#[cfg(...)]`: forwarding already declared that feature, so the gate is valid. ~keep
///
/// Shared by every backend that calls [`gen_enum_from_core_to_binding_cfg`] /
/// [`gen_enum_from_binding_to_core_cfg`] (napi, magnus, rustler, wasm) -- fixing the decision once
/// here fixes it for all of them, the same "ask cfg.rs" pattern `is_host_owned_rust_path` exists
/// to enforce. ~keep
fn emit_cfg_gated_arm(
    enum_def: &EnumDef,
    variant: &crate::core::ir::EnumVariant,
    is_host_enum: bool,
    arm: String,
    direction: &str,
) -> Option<minijinja::value::Value> {
    let Some(cfg) = variant.cfg.as_deref() else {
        return Some(minijinja::context! { arm => arm, cfg => Option::<&str>::None });
    };
    if !is_host_enum {
        tracing::warn!(
            enum_name = %enum_def.name,
            enum_rust_path = %enum_def.rust_path,
            variant_name = %variant.name,
            cfg = cfg,
            direction = direction,
            "dropping enum conversion match arm for a foreign-crate variant behind a #[cfg(...)] \
             this binding crate cannot declare as a Cargo feature; the variant is unreachable \
             from this conversion"
        );
        return None;
    }
    Some(minijinja::context! { arm => arm, cfg => cfg })
}

/// Whether a generated `From<...>` match over `enum_def`'s variants needs a trailing
/// `_ => Default::default()` catch-all to stay exhaustive under `-D warnings`.
///
/// A cfg-gated variant that is host-owned keeps a match arm carrying the identical
/// `#[cfg(...)]` guard as the variant's own declaration ([`emit_cfg_gated_arm`] and every
/// backend-local mirror of it), so in any single build the variant and its arm compile in or
/// out together: the match stays exhaustive either way and a catch-all over it is never
/// reachable. Only a cfg-gated variant that gets DROPPED -- a foreign-crate variant whose
/// `#[cfg(...)]` names a feature this binding crate cannot declare, see
/// `codegen::cfg::is_host_owned_rust_path` -- leaves a real gap: the match loses that arm
/// unconditionally while the matched type may still carry the variant, so a catch-all is
/// required. `has_cfg_variants` alone (ignoring host ownership) over-reports this and produces
/// an `unreachable_patterns` error the moment every cfg-gated variant happens to be host-owned,
/// which is the common case once every binding language forwards its cfg features (alef #464).
///
/// `has_excluded_variants` covers the orthogonal case where the compared type carries variants
/// entirely absent from the generated arms regardless of cfg -- pass `true` only when matching a
/// representation (typically the CORE type) that can hold more variants than the binding
/// generates arms for; pass `false` when matching the binding's own type, which by construction
/// never contains a variant absent from the arm list. ~keep
#[must_use]
pub fn enum_conversion_needs_catch_all(
    has_cfg_variants: bool,
    is_host_enum: bool,
    has_excluded_variants: bool,
) -> bool {
    has_excluded_variants || (has_cfg_variants && !is_host_enum)
}

/// Generate `impl From<BindingEnum> for core::Enum` (binding -> core).
pub fn gen_enum_from_binding_to_core(enum_def: &EnumDef, core_import: &str) -> String {
    gen_enum_from_binding_to_core_cfg(enum_def, core_import, &ConversionConfig::default())
}

/// Generate `impl From<BindingEnum> for core::Enum` with backend-specific config.
pub fn gen_enum_from_binding_to_core_cfg(enum_def: &EnumDef, core_import: &str, config: &ConversionConfig) -> String {
    let core_path = core_enum_path_remapped(enum_def, core_import, config.source_crate_remaps);
    let binding_name = format!("{}{}", config.type_name_prefix, enum_def.name);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);

    let arms: Vec<minijinja::value::Value> = enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            let arm = binding_to_core_match_arm_ext_cfg(
                &binding_name,
                &variant.name,
                &variant.fields,
                config.binding_enums_have_data,
                config,
                crate::codegen::conversions::helpers::variant_emits_tuple_form(enum_def, variant)
                    && config.binding_tuple_form_for_variants,
            );
            emit_cfg_gated_arm(enum_def, variant, is_host_enum, arm, "binding_to_core")
        })
        .collect();

    let has_cfg_variants = enum_def.variants.iter().any(|v| v.cfg.is_some());
    let needs_catch_all = enum_conversion_needs_catch_all(has_cfg_variants, is_host_enum, false);

    crate::codegen::template_env::render(
        "conversions/enum_from_binding_to_core",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            has_excluded_variants => needs_catch_all,
        },
    )
}

/// Generate `impl From<core::Enum> for BindingEnum` (core -> binding).
pub fn gen_enum_from_core_to_binding(enum_def: &EnumDef, core_import: &str) -> String {
    gen_enum_from_core_to_binding_cfg(enum_def, core_import, &ConversionConfig::default())
}

/// Generate `impl From<core::Enum> for BindingEnum` with backend-specific config.
pub fn gen_enum_from_core_to_binding_cfg(enum_def: &EnumDef, core_import: &str, config: &ConversionConfig) -> String {
    let core_path = core_enum_path_remapped(enum_def, core_import, config.source_crate_remaps);
    let binding_name = format!("{}{}", config.type_name_prefix, enum_def.name);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);

    let arms: Vec<minijinja::value::Value> = enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            let arm = core_to_binding_match_arm_ext_cfg(
                &core_path,
                &variant.name,
                &variant.fields,
                config.binding_enums_have_data,
                config,
                crate::codegen::conversions::helpers::variant_emits_tuple_form(enum_def, variant)
                    && config.binding_tuple_form_for_variants,
            );
            emit_cfg_gated_arm(enum_def, variant, is_host_enum, arm, "core_to_binding")
        })
        .collect();

    let has_cfg_variants = enum_def.variants.iter().any(|v| v.cfg.is_some());
    let needs_catch_all =
        enum_conversion_needs_catch_all(has_cfg_variants, is_host_enum, !enum_def.excluded_variants.is_empty());

    crate::codegen::template_env::render(
        "conversions/enum_from_core_to_binding",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            has_excluded_variants => needs_catch_all,
        },
    )
}
