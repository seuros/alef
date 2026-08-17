use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, TypeRef};
use ahash::AHashMap;

pub(super) fn resolve_public_default_functions(surface: &mut ApiSurface) {
    let public_methods: AHashMap<(String, String), String> = surface
        .types
        .iter()
        .flat_map(|typ| {
            typ.methods
                .iter()
                .filter(|method| method.is_static && method.params.is_empty() && !method.binding_excluded)
                .map(|method| {
                    (
                        (typ.name.clone(), method.name.clone()),
                        format!("{}::{}", typ.rust_path.replace('-', "_"), method.name),
                    )
                })
        })
        .collect();

    for typ in &mut surface.types {
        for field in &mut typ.fields {
            let Some(DefaultValue::FunctionCall(path)) = &field.typed_default else {
                continue;
            };
            let segments: Vec<_> = path.split("::").collect();
            let [.., owner, method] = segments.as_slice() else {
                continue;
            };
            if let Some(resolved_path) = public_methods.get(&(owner.to_string(), method.to_string())) {
                field.typed_default = Some(DefaultValue::PublicFunctionCall(resolved_path.clone()));
            }
        }
    }
}

/// Returns `true` if the type is a simple leaf type (primitive, String, Bytes, Path, etc.)
/// rather than a complex Named, collection, or Optional type.
fn is_simple_type(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Primitive(_)
            | TypeRef::String
            | TypeRef::Bytes
            | TypeRef::Path
            | TypeRef::Unit
            | TypeRef::Duration
            | TypeRef::Json
    )
}

/// Resolve newtype wrappers in the API surface.
///
/// Single-field tuple structs (`pub struct Foo(T)`) are identified by having exactly
/// one field named `_0`, no methods, and a simple inner type (primitive, String, etc.).
/// For each such newtype, all `TypeRef::Named("Foo")` references throughout the surface
/// are replaced with the inner type `T`, and the newtype TypeDef itself is removed.
/// This makes newtypes fully transparent to backends.
///
/// Tuple structs wrapping complex Named types (e.g., builders) are kept as-is.
pub(super) fn resolve_newtypes(surface: &mut ApiSurface) {
    let newtype_map: AHashMap<String, TypeRef> = surface
        .types
        .iter()
        .filter(|t| t.fields.len() == 1 && t.fields[0].name == "_0" && is_simple_type(&t.fields[0].ty))
        .map(|t| (t.name.clone(), t.fields[0].ty.clone()))
        .collect();

    if newtype_map.is_empty() {
        return;
    }

    let newtype_rust_paths: AHashMap<String, String> = surface
        .types
        .iter()
        .filter(|t| newtype_map.contains_key(&t.name))
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .collect();

    surface.types.retain(|t| !newtype_map.contains_key(&t.name));

    for typ in &mut surface.types {
        for field in &mut typ.fields {
            if let TypeRef::Named(name) = &field.ty
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                field.newtype_wrapper = Some(rust_path.clone());
            }
            if let TypeRef::Optional(inner) = &field.ty
                && let TypeRef::Named(name) = inner.as_ref()
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                field.newtype_wrapper = Some(rust_path.clone());
            }
            if let TypeRef::Vec(inner) = &field.ty
                && let TypeRef::Named(name) = inner.as_ref()
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                field.newtype_wrapper = Some(rust_path.clone());
            }
            resolve_typeref(&newtype_map, &mut field.ty);
        }
        for method in &mut typ.methods {
            for param in &mut method.params {
                if let TypeRef::Named(name) = &param.ty
                    && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
                {
                    param.newtype_wrapper = Some(rust_path.clone());
                }
                resolve_typeref(&newtype_map, &mut param.ty);
            }
            if let TypeRef::Named(name) = &method.return_type
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                method.return_newtype_wrapper = Some(rust_path.clone());
            }
            resolve_typeref(&newtype_map, &mut method.return_type);
        }
    }
    for func in &mut surface.functions {
        for param in &mut func.params {
            if let TypeRef::Named(name) = &param.ty
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                param.newtype_wrapper = Some(rust_path.clone());
            }
            resolve_typeref(&newtype_map, &mut param.ty);
        }
        if let TypeRef::Named(name) = &func.return_type
            && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
        {
            func.return_newtype_wrapper = Some(rust_path.clone());
        }
        resolve_typeref(&newtype_map, &mut func.return_type);
    }
    for enum_def in &mut surface.enums {
        for variant in &mut enum_def.variants {
            for field in &mut variant.fields {
                resolve_typeref(&newtype_map, &mut field.ty);
            }
        }
    }
}

/// Recursively replace `TypeRef::Named(name)` with the newtype's inner type.
fn resolve_typeref(newtype_map: &AHashMap<String, TypeRef>, ty: &mut TypeRef) {
    match ty {
        TypeRef::Named(name) => {
            if let Some(inner) = newtype_map.get(name.as_str()) {
                *ty = inner.clone();
            }
        }
        TypeRef::Optional(inner) => resolve_typeref(newtype_map, inner),
        TypeRef::Vec(inner) => resolve_typeref(newtype_map, inner),
        TypeRef::Map(k, v) => {
            resolve_typeref(newtype_map, k);
            resolve_typeref(newtype_map, v);
        }
        _ => {}
    }
}

/// Resolve unresolved `trait_source` on methods after all source files have been processed.
///
/// When `impl Trait for Type` is encountered before the trait definition has been extracted
/// (e.g., `pub mod extractors` comes before `pub mod plugins` in lib.rs), the single-segment
/// trait name lookup fails because the trait `TypeDef` doesn't exist yet. This pass retroactively
/// resolves those methods by matching method names against trait types' method lists. ~keep
pub(super) fn resolve_trait_sources(surface: &mut ApiSurface) {
    let mut trait_method_map: AHashMap<String, Vec<(String, String)>> = AHashMap::new();
    let mut trait_methods_set: AHashMap<String, Vec<String>> = AHashMap::new();

    for typ in &surface.types {
        if !typ.is_trait {
            continue;
        }
        let method_names: Vec<String> = typ.methods.iter().map(|m| m.name.clone()).collect();
        trait_methods_set.insert(typ.name.clone(), method_names.clone());
        for method_name in &method_names {
            trait_method_map
                .entry(method_name.clone())
                .or_default()
                .push((typ.name.clone(), typ.rust_path.replace('-', "_")));
        }
    }

    if trait_method_map.is_empty() {
        return;
    }

    for typ in &mut surface.types {
        if typ.is_trait {
            continue;
        }

        let unresolved_names: Vec<String> = typ
            .methods
            .iter()
            .filter(|m| m.trait_source.is_none())
            .map(|m| m.name.clone())
            .collect();

        for method in &mut typ.methods {
            if method.trait_source.is_some() {
                continue;
            }
            let Some(candidates) = trait_method_map.get(&method.name) else {
                continue;
            };

            if candidates.len() == 1 {
                method.trait_source = Some(candidates[0].1.clone());
            } else {
                let best = candidates.iter().max_by_key(|(trait_name, _)| {
                    trait_methods_set
                        .get(trait_name)
                        .map(|trait_methods| {
                            trait_methods
                                .iter()
                                .filter(|method_name| unresolved_names.contains(method_name))
                                .count()
                        })
                        .unwrap_or(0)
                });
                if let Some((_, rust_path)) = best {
                    method.trait_source = Some(rust_path.clone());
                }
            }
        }
    }
}

/// True when `value` carries no [`DefaultValue::Unresolved`] and no
/// [`DefaultValue::FunctionCall`]/[`DefaultValue::PublicFunctionCall`] anywhere within it
/// (including nested inside a `TupleVariant`/`StructVariant`/`ListLiteral` payload).
///
/// `Unresolved` is the documented "alef could not read this" marker and is never safe to
/// compare. `FunctionCall`/`PublicFunctionCall` get the same treatment though neither is
/// documented as unknown: each names a real zero-argument function whose return value alef
/// never evaluates, so two `FunctionCall`s naming different paths are exactly as undecidable as
/// one `Unresolved` value — asserting they disagree would be a guess about what the function
/// returns, not a reading. Only a value built entirely from literals, `Empty`, `None`, or
/// variants/lists of those is safe to compare. ~keep
fn is_fully_known(value: &DefaultValue) -> bool {
    match value {
        DefaultValue::Unresolved(_) | DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_) => false,
        DefaultValue::BoolLiteral(_)
        | DefaultValue::StringLiteral(_)
        | DefaultValue::IntLiteral(_)
        | DefaultValue::FloatLiteral(_)
        | DefaultValue::EnumVariant(_)
        | DefaultValue::Empty
        | DefaultValue::None => true,
        DefaultValue::TupleVariant(_, args) => args.iter().all(is_fully_known),
        DefaultValue::StructVariant(_, fields) => fields.iter().all(|(_, value)| is_fully_known(value)),
        DefaultValue::ListLiteral(items) => items.iter().all(is_fully_known),
    }
}

/// Warn when a field's serde-reader default and its final `#[derive(Default)]`/manual
/// `impl Default` value are both fully known and disagree (issue #153).
///
/// `FieldDef::typed_default` is a single slot written up to three times: the serde reader sets
/// it first (`extract::extractor::helpers::fields::extract_field`), then either
/// `#[derive(Default)]` (`extract::extractor::types::extract_struct`) or a manual `impl Default`
/// (`extract::extractor::defaults::extract_default_values`) unconditionally overwrites it. By
/// the time extraction finishes, the serde value is gone — a binding generated from it would
/// silently disagree with the Rust core whenever the other path is taken instead. `serde_defaults`
/// is the serde reader's value, captured by the caller before it was overwritten (see
/// `extract::extractor::types::extract_struct` and the `pending_serde_defaults` threaded through
/// `extract::extractor::mod::extract_items` down to `extract::extractor::functions::impl_blocks`).
///
/// `#[derive(Default)]`'s blanket `Empty` write is treated as a genuinely known value here, not a
/// placeholder: `Empty` is documented (see `DefaultValue::Empty`, and the comment on the
/// `has_default` write in `extract_struct`) as an assertion that the derived default *is* the
/// field's type-zero, exactly as `Vec::new()` or `Default::default()` inside a manual `impl
/// Default` are. Refusing to compare it would silently exempt the most common case (a struct
/// that derives `Default` while also carrying `#[serde(default)]` fields) from this diagnostic. ~keep
pub(crate) fn warn_on_default_disagreement(
    rust_path: &str,
    fields: &[FieldDef],
    serde_defaults: &AHashMap<String, DefaultValue>,
) {
    for field in fields {
        let Some(serde_default) = serde_defaults.get(&field.name) else {
            continue;
        };
        let Some(actual_default) = &field.typed_default else {
            continue;
        };
        if !is_fully_known(serde_default) || !is_fully_known(actual_default) {
            tracing::debug!(
                target: "alef::extract::defaults",
                rust_type = rust_path,
                field = %field.name,
                "field default comparison skipped: serde default or resolved default is not fully known"
            );
            continue;
        }
        if serde_default != actual_default {
            tracing::warn!(
                target: "alef::extract::defaults",
                rust_type = rust_path,
                field = %field.name,
                serde_default = ?serde_default,
                resolved_default = ?actual_default,
                "field's `#[serde(default)]` value disagrees with its `#[derive(Default)]`/`impl Default` \
                 value; a binding generated from one will silently differ from the Rust core when the \
                 other path is taken"
            );
        }
    }
}

#[cfg(test)]
mod default_disagreement_tests {
    use super::*;
    use tracing_test::traced_test;

    fn field(name: &str, typed_default: Option<DefaultValue>) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            typed_default,
            ..Default::default()
        }
    }

    fn serde_defaults(entries: &[(&str, DefaultValue)]) -> AHashMap<String, DefaultValue> {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect()
    }

    /// (a) Two known values that genuinely disagree must be reported, naming the type, the
    /// field, and both values.
    #[test]
    #[traced_test]
    fn genuine_disagreement_between_two_known_values_is_reported() {
        let fields = vec![field("retries", Some(DefaultValue::IntLiteral(5)))];
        let defaults = serde_defaults(&[("retries", DefaultValue::IntLiteral(3))]);

        warn_on_default_disagreement("my_crate::Config", &fields, &defaults);

        assert!(logs_contain("my_crate::Config"));
        assert!(logs_contain("retries"));
        assert!(logs_contain("IntLiteral(3)"));
        assert!(logs_contain("IntLiteral(5)"));
    }

    /// (e) `#[derive(Default)]`'s blanket `Empty` write is a genuinely known value, not a
    /// placeholder: this is the realistic shape of (a) for a derived struct, where the serde
    /// reader recovered a concrete value but the derive gave every field the type's zero.
    #[test]
    #[traced_test]
    fn derived_defaults_empty_disagreeing_with_a_known_serde_default_is_reported() {
        let fields = vec![field("host", Some(DefaultValue::Empty))];
        let defaults = serde_defaults(&[("host", DefaultValue::StringLiteral("localhost".to_string()))]);

        warn_on_default_disagreement("my_crate::Client", &fields, &defaults);

        assert!(logs_contain("my_crate::Client"));
        assert!(logs_contain("host"));
        assert!(logs_contain("StringLiteral(\"localhost\")"));
        assert!(logs_contain("Empty"));
    }

    /// (b) Agreement must not be reported.
    #[test]
    #[traced_test]
    fn agreeing_known_defaults_are_not_reported() {
        let fields = vec![field("retries", Some(DefaultValue::IntLiteral(3)))];
        let defaults = serde_defaults(&[("retries", DefaultValue::IntLiteral(3))]);

        warn_on_default_disagreement("my_crate::Config", &fields, &defaults);

        assert!(!logs_contain("disagrees"));
    }

    /// (e) continued: `Empty` on both sides (bare `#[serde(default)]` alongside
    /// `#[derive(Default)]`) is agreement, not a placeholder collision, and must not be reported.
    #[test]
    #[traced_test]
    fn matching_empty_defaults_on_both_sides_are_not_reported() {
        let fields = vec![field("count", Some(DefaultValue::Empty))];
        let defaults = serde_defaults(&[("count", DefaultValue::Empty)]);

        warn_on_default_disagreement("my_crate::Config", &fields, &defaults);

        assert!(!logs_contain("disagrees"));
    }

    /// (c) `Unresolved` on the resolved (`#[derive(Default)]`/`impl Default`) side is an
    /// unknown, not a disagreement, even though the serde side is a known, differing literal.
    #[test]
    #[traced_test]
    fn unresolved_resolved_default_is_not_reported() {
        let fields = vec![field(
            "level",
            Some(DefaultValue::Unresolved("Self::builder().level(9).build()".to_string())),
        )];
        let defaults = serde_defaults(&[("level", DefaultValue::IntLiteral(9))]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults);

        assert!(!logs_contain("disagrees"));
    }

    /// (c) continued: `Unresolved` on the serde side is likewise silent.
    #[test]
    #[traced_test]
    fn unresolved_serde_default_is_not_reported() {
        let fields = vec![field("level", Some(DefaultValue::IntLiteral(9)))];
        let defaults = serde_defaults(&[("level", DefaultValue::Unresolved("compute_default()".to_string()))]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults);

        assert!(!logs_contain("disagrees"));
    }

    /// (d) An `Unresolved` nested inside a `TupleVariant` payload makes the whole value
    /// unknown, even though the variant name itself matches on both sides.
    #[test]
    #[traced_test]
    fn nested_unresolved_inside_a_tuple_variant_is_not_reported() {
        let fields = vec![field(
            "mode",
            Some(DefaultValue::TupleVariant(
                "Custom".to_string(),
                vec![DefaultValue::Unresolved("compute()".to_string())],
            )),
        )];
        let defaults = serde_defaults(&[(
            "mode",
            DefaultValue::TupleVariant("Custom".to_string(), vec![DefaultValue::IntLiteral(5)]),
        )]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults);

        assert!(!logs_contain("disagrees"));
    }

    /// (d) continued: the same nested-unknown rule applies to a `StructVariant` payload.
    #[test]
    #[traced_test]
    fn nested_unresolved_inside_a_struct_variant_is_not_reported() {
        let fields = vec![field(
            "kind",
            Some(DefaultValue::StructVariant(
                "Curated".to_string(),
                vec![("label".to_string(), DefaultValue::Unresolved("compute()".to_string()))],
            )),
        )];
        let defaults = serde_defaults(&[(
            "kind",
            DefaultValue::StructVariant(
                "Curated".to_string(),
                vec![("label".to_string(), DefaultValue::StringLiteral("balanced".to_string()))],
            ),
        )]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults);

        assert!(!logs_contain("disagrees"));
    }

    /// A `FunctionCall` names a real function whose return value alef never evaluates, so it is
    /// treated the same as `Unresolved`: comparing it against a differing literal would be a
    /// guess about what the function returns, not a reading, even when the paths look unrelated
    /// to the literal on the other side.
    #[test]
    #[traced_test]
    fn function_call_default_is_not_compared_even_against_a_differing_literal() {
        let fields = vec![field("token", Some(DefaultValue::StringLiteral("abc".to_string())))];
        let defaults = serde_defaults(&[("token", DefaultValue::FunctionCall("generate_token".to_string()))]);

        warn_on_default_disagreement("my_crate::Auth", &fields, &defaults);

        assert!(!logs_contain("disagrees"));
    }

    /// A field with no serde default recorded (most fields) must not be touched at all.
    #[test]
    #[traced_test]
    fn field_absent_from_serde_defaults_is_skipped() {
        let fields = vec![field("untouched", Some(DefaultValue::IntLiteral(1)))];
        let defaults = serde_defaults(&[]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults);

        assert!(!logs_contain("disagrees"));
    }
}
