//! Rust-side trait-bridge helpers for the WASM backend that are not themselves emitters:
//! a post-processing fixup over generated builder calls, and the registration surface reported
//! to the reference-doc renderer.

use crate::codegen::generators::trait_bridge::to_camel_case;
use crate::core::backend::TraitBridgeRegistrationSurface;
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::ApiSurface;

pub(super) fn forward_trait_bridge_builder_fields(mut content: String, trait_bridges: &[TraitBridgeConfig]) -> String {
    for bridge in trait_bridges {
        if let Some(field_name) = bridge.resolved_options_field() {
            let param_name = bridge.param_name.as_deref().unwrap_or(field_name);
            let pattern = format!(".{}({}.as_ref().map(|v| &v.inner))", field_name, param_name);
            let replacement = format!(".{}({}.map(|v| (*v.inner).clone()))", field_name, param_name);
            content = content.replace(&pattern, &replacement);
        }
    }
    content
}

/// The JS names `trait_bridge::gen_trait_bridge` exports for each configured bridge.
///
/// `to_camel_case` is the same helper the emitter feeds into `#[wasm_bindgen(js_name = ...)]`.
/// Registration additionally requires `registry_getter`, without which
/// `WasmBridgeGenerator::gen_registration_fn` emits nothing.
///
/// `exclude_languages` is deliberately not consulted: the WASM emission path does not check it
/// either, so filtering here would hide a function the module really does export. The only gate
/// the emitter applies is that the trait resolve in the `ApiSurface`. ~keep
pub(super) fn registration_surface(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> Vec<TraitBridgeRegistrationSurface> {
    config
        .trait_bridges
        .iter()
        .filter_map(|bridge| {
            let trait_def = api.types.iter().find(|t| t.is_trait && t.name == bridge.trait_name)?;
            // A visitor bridge takes `gen_visitor_bridge`, which emits no registry API. ~keep
            let is_visitor_bridge = bridge.type_alias.is_some()
                && bridge.register_fn.is_none()
                && bridge.super_trait.is_none()
                && trait_def.methods.iter().all(|method| method.has_default_impl);
            if is_visitor_bridge {
                return None;
            }
            let surface = TraitBridgeRegistrationSurface {
                trait_name: trait_def.name.clone(),
                register_symbol: bridge
                    .register_fn
                    .as_deref()
                    .filter(|_| bridge.registry_getter.is_some())
                    .map(to_camel_case),
                unregister_symbol: bridge.unregister_fn.as_deref().map(to_camel_case),
                clear_symbol: bridge.clear_fn.as_deref().map(to_camel_case),
            };
            let emits_nothing = surface.register_symbol.is_none()
                && surface.unregister_symbol.is_none()
                && surface.clear_symbol.is_none();
            (!emits_nothing).then_some(surface)
        })
        .collect()
}
