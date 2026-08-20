use crate::core::ir::{ApiSurface, DefaultValue, MethodDef, TypeDef, UnsupportedPublicItem};
use ahash::AHashMap;

use super::super::SerdeDefaultsByType;
use super::super::defaults::{ConstructorIndex, extract_default_values};
use super::super::helpers::{build_rust_path, extract_binding_exclusion_reason, extract_cfg_condition, is_test_gated};
use super::super::postprocess::{enum_default_variant_names, warn_on_default_disagreement};
use super::extract_method;

fn has_non_lifetime_generics(generics: &syn::Generics) -> bool {
    generics
        .params
        .iter()
        .any(|param| !matches!(param, syn::GenericParam::Lifetime(_)))
}

fn record_unsupported_generic_impl_methods(
    item: &syn::ItemImpl,
    crate_name: &str,
    type_name: &str,
    surface: &mut ApiSurface,
    reason: &str,
    methods_are_public_by_trait: bool,
) {
    for impl_item in &item.items {
        let syn::ImplItem::Fn(method) = impl_item else {
            continue;
        };
        if (!methods_are_public_by_trait && !super::super::helpers::is_pub(&method.vis))
            || extract_binding_exclusion_reason(&method.attrs).is_some()
        {
            continue;
        }
        let method_name = method.sig.ident.to_string();
        if method_name.starts_with('_') {
            continue;
        }
        surface.unsupported_public_items.push(UnsupportedPublicItem {
            item_kind: "method".to_string(),
            item_path: format!("{crate_name}::{type_name}.{method_name}"),
            reason: reason.to_string(),
            suggested_fix:
                "exclude the method, configure an opaque/bridge policy, or provide explicit monomorphization metadata"
                    .to_string(),
        });
    }
}

/// Extract methods from an `impl` block and attach them to the corresponding `TypeDef`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extract_impl_block(
    item: &syn::ItemImpl,
    crate_name: &str,
    module_path: &str,
    surface: &mut ApiSurface,
    type_index: &AHashMap<String, usize>,
    binding_excluded_type_names: &ahash::AHashSet<String>,
    result_wrapping_aliases: &ahash::AHashSet<String>,
    literal_consts: &AHashMap<String, DefaultValue>,
    constructors: &ConstructorIndex<'_>,
    pending_serde_defaults: &SerdeDefaultsByType,
) {
    // Honor `#[cfg_attr(alef, alef(skip))]` (or bare `#[alef(skip)]`) on the impl block
    if extract_binding_exclusion_reason(&item.attrs).is_some() {
        return;
    }

    // The block's own gate applies to every method it contains; `#[cfg(test)]` blocks were
    // already dropped by the caller, so anything left here is a real binding-surface gate. ~keep
    let impl_cfg = extract_cfg_condition(&item.attrs);

    if item.trait_.is_some() {
        extract_trait_impl_methods(
            item,
            crate_name,
            surface,
            type_index,
            result_wrapping_aliases,
            literal_consts,
            impl_cfg.as_deref(),
            constructors,
            pending_serde_defaults,
        );
        return;
    }

    let type_name = match &*item.self_ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(),
        _ => return,
    };

    if binding_excluded_type_names.contains(&type_name)
        || type_index
            .get(&type_name)
            .is_some_and(|&idx| surface.types[idx].binding_excluded)
    {
        return;
    }

    if has_non_lifetime_generics(&item.generics) {
        record_unsupported_generic_impl_methods(
            item,
            crate_name,
            &type_name,
            surface,
            "public methods on generic impl blocks cannot be represented without explicit monomorphization metadata",
            false,
        );
        return;
    }

    let type_is_opaque = item.generics.params.is_empty()
        && (type_index
            .get(&type_name)
            .map(|&idx| surface.types[idx].is_opaque)
            .unwrap_or(false)
            || surface.enums.iter().any(|e| e.name == type_name)
            || surface.errors.iter().any(|e| e.name == type_name)
            || !type_index.contains_key(&type_name));

    let methods: Vec<MethodDef> = item
        .items
        .iter()
        .filter_map(|impl_item| {
            if let syn::ImplItem::Fn(method) = impl_item
                && super::super::helpers::is_pub(&method.vis) {
                    // Skip `#[cfg(test)]` methods (e.g. test-only constructors like
                    if is_test_gated(&method.attrs) {
                        return None;
                    }
                    if !method.sig.generics.params.is_empty() {
                        if extract_binding_exclusion_reason(&method.attrs).is_none() {
                            surface.unsupported_public_items.push(UnsupportedPublicItem {
                                item_kind: "method".to_string(),
                                item_path: format!("{crate_name}::{type_name}.{}", method.sig.ident),
                                reason: "public generic inherent methods cannot be represented without explicit monomorphization metadata".to_string(),
                                suggested_fix: "exclude the method, configure an opaque/bridge policy, or provide explicit monomorphization metadata".to_string(),
                            });
                        }
                        return None;
                    }
                    let method_name = method.sig.ident.to_string();
                    if method_name.starts_with('_') {
                        return None;
                    }
                    if method_name == "new" && !type_is_opaque
                        && let syn::ReturnType::Type(_, ty) = &method.sig.output
                            && matches!(&**ty, syn::Type::Path(p) if p.path.is_ident("Self")) {
                                return None;
                            }
                    return Some(extract_method(
                        method,
                        crate_name,
                        &type_name,
                        None,
                        result_wrapping_aliases,
                        impl_cfg.as_deref(),
                    ));
                }
            None
        })
        .collect();

    if methods.is_empty() {
        return;
    }

    if let Some(&idx) = type_index.get(&type_name) {
        for method in methods {
            // First-wins by name, with no `cfg` merge: when the same method name is provided by
            // two blocks under disjoint gates (`#[cfg(feature = "x")]` / `#[cfg(not(...))]`), the
            // first block's gate is the one that survives onto the retained `MethodDef`. Free
            // functions have `codegen::fn_dedup` for exactly this; methods have no counterpart
            // yet. Merging the gates (OR of the group, mirroring `with_deduped_functions`) is
            // deliberately deferred — do it here and in the trait-impl loop below together. ~keep
            if !surface.types[idx].methods.iter().any(|m| m.name == method.name) {
                surface.types[idx].methods.push(method);
            }
        }
    } else if let Some(error_def) = surface.errors.iter_mut().find(|e| e.name == type_name) {
        const ERROR_METHOD_WHITELIST: &[&str] = &["status_code", "is_transient", "error_type"];
        for method in methods {
            let is_whitelisted = ERROR_METHOD_WHITELIST.contains(&method.name.as_str());
            let already_present = error_def.methods.iter().any(|m| m.name == method.name);
            if is_whitelisted && !already_present {
                error_def.methods.push(method);
            }
        }
    } else if let Some(enum_def) = surface.enums.iter_mut().find(|e| {
        if e.name != type_name {
            return false;
        }
        let crate_prefix = format!("{crate_name}::");
        let rel = e.rust_path.strip_prefix(&*crate_prefix).unwrap_or(e.rust_path.as_str());
        let enum_module_rel = rel.rfind("::").map(|i| &rel[..i]).unwrap_or("");
        if enum_module_rel.is_empty() {
            return true;
        }
        if module_path.is_empty() {
            return false;
        }
        enum_module_rel.starts_with(module_path) || module_path.starts_with(enum_module_rel)
    }) {
        for method in &methods {
            if method.is_static && !enum_def.methods.iter().any(|m| m.name == method.name) {
                enum_def.methods.push(method.clone());
            }
        }
    } else {
        let rust_path = build_rust_path(crate_name, module_path, &type_name);
        surface.types.push(TypeDef {
            name: type_name.clone(),
            rust_path,
            original_rust_path: String::new(),
            fields: vec![],
            methods,
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            doc: String::new(),
            cfg: None,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            super_traits: vec![],
            binding_excluded: true,
            binding_exclusion_reason: Some(
                "synthetic-opaque-from-impl-block (source visibility unverified)".to_string(),
            ),
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        });
    }
}

/// The unit variant a hand-written `impl Default for SomeEnum` returns, when the body is a bare
/// `Self::Variant` / `SomeEnum::Variant` tail expression.
///
/// `#[derive(Default)]` records its choice on the variant itself (`EnumVariant::is_default`), but a
/// manual impl carries the same fact only in its body, and every consumer of `is_default` — the Go,
/// Rustler, Dart, WASM, Kotlin, Magnus and PHP backends, plus the generated Rust mirror enum's
/// `#[default]` marker — would otherwise fall back to the *first declared* variant or to no default
/// at all. Both are guesses that silently disagree with the Rust core whenever the real default is
/// declared elsewhere in the enum. Reading it here turns that guess into a fact.
///
/// Deliberately narrow: only a bare path to a unit variant is recognised. A tuple/struct variant, a
/// `match`, or any computed body leaves `is_default` unset, so callers keep their existing honest
/// fallback rather than receiving a fabricated variant. ~keep
fn manual_default_unit_variant(item: &syn::ItemImpl) -> Option<String> {
    let default_fn = item.items.iter().find_map(|impl_item| match impl_item {
        syn::ImplItem::Fn(method) if method.sig.ident == "default" => Some(method),
        _ => None,
    })?;

    let tail = match default_fn.block.stmts.last()? {
        syn::Stmt::Expr(expr, _) => expr,
        _ => return None,
    };
    let expr = match tail {
        syn::Expr::Return(ret) => ret.expr.as_deref()?,
        other => other,
    };
    let syn::Expr::Path(path_expr) = expr else {
        return None;
    };

    let segments = &path_expr.path.segments;
    if segments.len() != 2 {
        return None;
    }
    let qualifier = segments.first()?.ident.to_string();
    let variant = segments.last()?;
    if !variant.arguments.is_none() {
        return None;
    }
    let self_type = match &*item.self_ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    };
    (qualifier == "Self" || Some(&qualifier) == self_type.as_ref()).then(|| variant.ident.to_string())
}

/// Extract methods from a trait impl and attach them to an existing type in the surface.
#[allow(clippy::too_many_arguments)]
fn extract_trait_impl_methods(
    item: &syn::ItemImpl,
    crate_name: &str,
    surface: &mut ApiSurface,
    type_index: &AHashMap<String, usize>,
    result_wrapping_aliases: &ahash::AHashSet<String>,
    literal_consts: &AHashMap<String, DefaultValue>,
    impl_cfg: Option<&str>,
    constructors: &ConstructorIndex<'_>,
    pending_serde_defaults: &SerdeDefaultsByType,
) {
    let type_name = match &*item.self_ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    };

    let Some(type_name) = type_name else { return };

    let Some(&idx) = type_index.get(&type_name) else {
        if let Some((path, _)) = &item.trait_
            && path.segments.last().is_some_and(|s| s.ident == "Default")
            && let Some(enum_def) = surface.enums.iter_mut().find(|e| e.name == type_name)
        {
            enum_def.has_default = true;
            if let Some(variant_name) = manual_default_unit_variant(item)
                && let Some(variant) = enum_def
                    .variants
                    .iter_mut()
                    .find(|v| v.name == variant_name && v.fields.is_empty() && !v.originally_had_data_fields)
            {
                variant.is_default = true;
            }
        }
        return;
    };

    if has_non_lifetime_generics(&item.generics) {
        record_unsupported_generic_impl_methods(
            item,
            crate_name,
            &type_name,
            surface,
            "public trait implementation methods on generic impl blocks cannot be represented without explicit monomorphization metadata",
            true,
        );
        return;
    }

    const STD_TRAITS: &[&str] = &[
        "Default",
        "Clone",
        "Copy",
        "Debug",
        "Display",
        "Drop",
        "PartialEq",
        "Eq",
        "PartialOrd",
        "Ord",
        "Hash",
        "From",
        "Into",
        "TryFrom",
        "TryInto",
        "Iterator",
        "IntoIterator",
        "Send",
        "Sync",
        "Sized",
        "Unpin",
        "Serialize",
        "Deserialize",
    ];
    let trait_source = item.trait_.as_ref().and_then(|(path, _)| {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let trait_name = segments.last().map(|s| s.as_str()).unwrap_or("");
        if STD_TRAITS.contains(&trait_name) {
            return None;
        }
        if segments.len() == 1 {
            let trait_name = &segments[0];
            surface
                .types
                .iter()
                .find(|t| t.is_trait && t.name == *trait_name)
                .map(|t| t.rust_path.replace('-', "_"))
        } else {
            Some(segments.join("::").replace('-', "_"))
        }
    });

    let type_def = &mut surface.types[idx];

    let is_default_trait_impl = item
        .trait_
        .as_ref()
        .is_some_and(|(path, _)| path.segments.last().is_some_and(|segment| segment.ident == "Default"));
    if is_default_trait_impl {
        // NOTE: this also sets `has_default` for a *manual* `impl Default`, so the flag does not
        // distinguish a derived (type-zero) default from a hand-written one. Telling those apart
        // is `DefaultValue::Unresolved`'s job, not this flag's. ~keep
        let self_type = type_def.name.clone();
        type_def.has_default = true;
        extract_default_values(item, &self_type, &mut type_def.fields, literal_consts, constructors);
        if let Some(serde_defaults) = pending_serde_defaults.get(&type_def.rust_path) {
            // Only the enums this crate has extracted so far (this file's own module, plus any
            // earlier file in `sources`) are visible here — a manual `impl Default` naming a
            // variant of an enum declared in a *later* source file cannot be proven to agree, so
            // it falls back to `warn_on_default_disagreement`'s ordinary (warn-on-mismatch)
            // behavior instead of guessing. See `agrees_via_enum_default`. ~keep
            let enum_default_variants = enum_default_variant_names(&surface.enums);
            warn_on_default_disagreement(
                &type_def.rust_path,
                &type_def.fields,
                serde_defaults,
                &enum_default_variants,
            );
        }
    }

    let is_conversion_trait = item.trait_.as_ref().is_some_and(|(path, _)| {
        path.segments
            .last()
            .is_some_and(|s| matches!(s.ident.to_string().as_str(), "From" | "Into" | "TryFrom" | "TryInto"))
    });
    if is_conversion_trait {
        return;
    }

    let is_std_trait_impl = item.trait_.as_ref().is_some_and(|(path, _)| {
        path.segments
            .last()
            .is_some_and(|s| STD_TRAITS.contains(&s.ident.to_string().as_str()))
    });
    if is_std_trait_impl && !is_default_trait_impl {
        return;
    }

    for impl_item in &item.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            if !method.sig.generics.params.is_empty() {
                if extract_binding_exclusion_reason(&method.attrs).is_none() {
                    surface.unsupported_public_items.push(UnsupportedPublicItem {
                        item_kind: "method".to_string(),
                        item_path: format!("{crate_name}::{type_name}.{}", method.sig.ident),
                        reason: "public generic trait implementation methods cannot be represented without explicit monomorphization metadata".to_string(),
                        suggested_fix: "exclude the method, configure an opaque/bridge policy, or provide explicit monomorphization metadata".to_string(),
                    });
                }
                continue;
            }
            let method_def = extract_method(
                method,
                crate_name,
                &type_name,
                trait_source.clone(),
                result_wrapping_aliases,
                impl_cfg,
            );
            // First-wins by name, no `cfg` merge — see the note in `extract_impl_block`. ~keep
            if !type_def.methods.iter().any(|m| m.name == method_def.name) {
                type_def.methods.push(method_def);
            }
        }
    }
}
