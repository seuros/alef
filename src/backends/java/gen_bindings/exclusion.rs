//! Compute and apply Java's binding-surface exclusions.
//!
//! Splits into its own file per `file-modularization`: `mod.rs` was already over the
//! repo's 1,000-line cap, so this touched concern (working out which types/functions/services
//! Java must not bind, then filtering an [`ApiSurface`] accordingly) moves out rather than
//! growing the over-limit file further.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, ParamDef, ServiceDef, TypeRef};
use std::collections::HashSet;

pub(super) fn effective_exclude_types(api: &ApiSurface, config: &ResolvedCrateConfig) -> HashSet<String> {
    let mut exclude_types: HashSet<String> = config
        .ffi
        .as_ref()
        .map(|ffi| ffi.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(java) = &config.java {
        exclude_types.extend(java.exclude_types.iter().cloned());
    }
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    exclude_types.extend(
        config
            .opaque_types
            .iter()
            .filter(|(_, path)| path.contains('<'))
            .map(|(name, _)| name.clone()),
    );
    exclude_types
}

/// Names of types carrying a lifetime parameter (e.g. `NodeContext<'a>`).
///
/// A lifetime parameter alone is not a reason a type cannot cross the JNI boundary: the
/// binding holds an opaque handle, or a plain-data record whose fields `extract::type_resolver`
/// already resolved to owned values (`&'a str` becomes `TypeRef::String`, etc.), and the
/// lifetime itself is erased at the C ABI exactly like it is for every other FFI-dependent
/// backend (csharp, go, kotlin, kotlin_android — none of which exclude these types at all).
/// Blanket-excluding every such type from `api.types` used to also strip it from
/// `resolve_visitor_generation`'s lookup, silently dropping the whole visitor pattern
/// (`VisitorBridge.java` and friends) whenever the configured `context_type` happened to be
/// lifetime-parameterized.
///
/// The one place a lifetime-bound type is still unsafe to bind is a *service*
/// (`ServiceDef`): `registrations`/`configurators`/`entrypoints` capture their parameters for
/// the lifetime of a long-running `run`/`finalize` call, and nothing in the IR proves the
/// borrowed data survives that long — so `api_without_excluded_types` drops services that
/// reference one of these names, without touching the type definitions themselves. ~keep
pub(super) fn lifetime_bound_type_names(api: &ApiSurface) -> HashSet<String> {
    api.types
        .iter()
        .filter(|t| t.has_lifetime_params)
        .map(|t| t.name.clone())
        .collect()
}

/// Whether `api_without_excluded_types` has anything to do.
///
/// `exclude_types` no longer carries lifetime-bound type names (see
/// `lifetime_bound_type_names`), so an empty `exclude_types` set does not by itself mean the
/// surface is already clean: a service may still reference a lifetime-bound type and need
/// filtering. Skip the clone-and-filter pass only when there is truly nothing either set could
/// remove. ~keep
pub(super) fn should_filter_excluded_types(api: &ApiSurface, exclude_types: &HashSet<String>) -> bool {
    !exclude_types.is_empty() || !api.services.is_empty()
}

fn references_excluded_type(ty: &TypeRef, exclude_types: &HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

fn signature_references_excluded_type(
    params: &[ParamDef],
    return_type: &TypeRef,
    exclude_types: &HashSet<String>,
) -> bool {
    references_excluded_type(return_type, exclude_types)
        || params
            .iter()
            .any(|param| references_excluded_type(&param.ty, exclude_types))
}

fn service_references_excluded_type(service: &ServiceDef, excluded: &HashSet<String>) -> bool {
    excluded.contains(&service.name)
        || signature_references_excluded_type(&service.constructor.params, &service.constructor.return_type, excluded)
        || service
            .configurators
            .iter()
            .any(|method| signature_references_excluded_type(&method.params, &method.return_type, excluded))
        || service.registrations.iter().any(|registration| {
            signature_references_excluded_type(&registration.metadata_params, &registration.return_type, excluded)
                || registration.variants.iter().any(|variant| {
                    variant
                        .signature_params
                        .iter()
                        .any(|param| references_excluded_type(&param.ty, excluded))
                })
        })
        || service
            .entrypoints
            .iter()
            .any(|entrypoint| signature_references_excluded_type(&entrypoint.params, &entrypoint.return_type, excluded))
}

pub(super) fn api_without_excluded_types(api: &ApiSurface, exclude_types: &HashSet<String>) -> ApiSurface {
    let lifetime_bound_types = lifetime_bound_type_names(api);
    let mut filtered = api.clone();
    filtered.services.retain(|service| {
        !service_references_excluded_type(service, exclude_types)
            && !service_references_excluded_type(service, &lifetime_bound_types)
    });
    filtered.types.retain(|typ| !exclude_types.contains(&typ.name));
    for typ in &mut filtered.types {
        typ.fields
            .retain(|field| !references_excluded_type(&field.ty, exclude_types));
        // Trait methods are exempt: each one owns a positional slot in the C vtable the
        // FFI crate declares from the unfiltered surface, and `java_type_visible` already
        // degrades an excluded type in a bridge signature to a JSON `String`. Dropping the
        // method here would leave Java writing N-1 upcall stubs into an N-slot struct, so
        // every later slot dispatches through the wrong function pointer. ~keep
        if !typ.is_trait {
            typ.methods.retain(|method| {
                !signature_references_excluded_type(&method.params, &method.return_type, exclude_types)
            });
        }
    }
    filtered
        .enums
        .retain(|enum_def| !exclude_types.contains(&enum_def.name));
    for enum_def in &mut filtered.enums {
        for variant in &mut enum_def.variants {
            variant
                .fields
                .retain(|field| !references_excluded_type(&field.ty, exclude_types));
        }
    }
    filtered
        .functions
        .retain(|func| !signature_references_excluded_type(&func.params, &func.return_type, exclude_types));
    filtered.errors.retain(|error| !exclude_types.contains(&error.name));
    filtered
}
