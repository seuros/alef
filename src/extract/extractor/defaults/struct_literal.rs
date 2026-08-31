//! Reading the initializers of a `Self { … }` struct literal into one [`DefaultValue`] per field.

use super::{DefaultValue, EvalScope, FieldMemberExt, expr_to_default_value, extract_cfg_condition, unreadable};
use ahash::AHashMap;
use quote::ToTokens;

/// The `cfg` predicate whose arm loses to its own negation. See [`resolve_duplicate_cfg_arms`].
const WASM32_POSITIVE: &str = "target_arch = \"wasm32\"";

/// Map each initializer of a struct literal to a `DefaultValue`.
///
/// `Self { #[cfg(feature = "x")] limit: 9 }` supplies this initializer only in builds that enable
/// the feature — but the field's own declaration must carry the identical gate, or the literal
/// would leave the field uninitialized in the other build and not compile. So wherever the field
/// exists at all, this is the only initializer that could have supplied it, and reading it is a
/// reading rather than a guess. A lone `cfg`-attributed initializer is therefore read exactly as
/// its bare counterpart would be, down to a call alef cannot evaluate folding to
/// [`DefaultValue::FunctionCall`] — refusing the `cfg`-gated spelling of a shape the ungated
/// spelling already accepts would draw a distinction with no basis in what alef can prove.
///
/// That argument turns on the literal initializing every field, which a `..base` rest expression
/// breaks: it lets an *ungated* field carry a `cfg`-gated initializer, taking its value from a
/// base this pass never read in the other build. `refuse_cfg_gated` re-imposes the wholesale
/// refusal for exactly that case.
///
/// A non-`cfg` attribute (`cfg_attr`, an attribute macro, `#[rustfmt::skip]`, …) always keeps the
/// wholesale refusal: its effect on the initializer is not knowable from source, so the
/// initializer's very presence is undetermined — the reasoning an attributed mutation statement is
/// refused for, see `mutation`'s module doc. ~keep
pub(super) fn struct_expr_defaults(
    struct_expr: &syn::ExprStruct,
    scope: &EvalScope<'_>,
) -> AHashMap<String, DefaultValue> {
    let refuse_cfg_gated = struct_expr.rest.is_some();
    let mut grouped: Vec<(String, Vec<&syn::FieldValue>)> = Vec::new();
    for field in &struct_expr.fields {
        let Some(ident) = field.member_named() else {
            continue;
        };
        let name = ident.to_string();
        match grouped.iter_mut().find(|(existing, _)| existing == &name) {
            Some((_, entries)) => entries.push(field),
            None => grouped.push((name, vec![field])),
        }
    }

    let mut defaults = AHashMap::new();
    for (name, entries) in grouped {
        let value = match entries.as_slice() {
            [field] => resolve_initializer(field, scope, &name, refuse_cfg_gated),
            arms => resolve_duplicate_cfg_arms(arms, scope, &name),
        };
        if let DefaultValue::Unresolved(source) = &value {
            tracing::debug!(
                target: "alef::extract::defaults",
                rust_type = scope.self_type,
                field = %name,
                initializer = %source,
                "field initializer is not constant-foldable; its default is unresolved"
            );
        }
        defaults.insert(name, value);
    }
    defaults
}

/// Resolve one struct-literal field initializer. See [`struct_expr_defaults`] for the policy.
fn resolve_initializer(
    field: &syn::FieldValue,
    scope: &EvalScope<'_>,
    name: &str,
    refuse_cfg_gated: bool,
) -> DefaultValue {
    let readable =
        field.attrs.is_empty() || (!refuse_cfg_gated && field.attrs.iter().all(|attr| attr.path().is_ident("cfg")));
    if !readable {
        return unreadable(&field.expr);
    }
    expr_to_default_value(&field.expr, scope, scope.field_types.get(name))
}

/// Two initializers for one field name compile only as mutually exclusive `cfg` arms, exactly one
/// of which survives `cfg`-stripping in any single build. Alef emits one binding per build rather
/// than per target, so it must pick one arm as *the* documented default instead of letting hash
/// map insertion order decide.
///
/// This is not a general `cfg` evaluator: it recognises the one complementary pair seen in
/// practice, `target_arch = "wasm32"` against its negation, and prefers the arm that is not
/// positively gated on wasm32. Alef's wasm backend has its own mechanism for fields that differ
/// under wasm32, so every other backend's doc comment and per-field literal should quote the
/// native value. Because such a pair covers every build, a `..base` rest expression can never
/// supply the field and the [`resolve_initializer`] rest guard does not apply.
///
/// Any duplicate that does not reduce to exactly this pair — more than two arms, or neither/both
/// naming wasm32 — has no established policy and stays `Unresolved` rather than being guessed. ~keep
fn resolve_duplicate_cfg_arms(arms: &[&syn::FieldValue], scope: &EvalScope<'_>, name: &str) -> DefaultValue {
    if let [first, second] = arms {
        let first_is_wasm32 = extract_cfg_condition(&first.attrs).as_deref() == Some(WASM32_POSITIVE);
        let second_is_wasm32 = extract_cfg_condition(&second.attrs).as_deref() == Some(WASM32_POSITIVE);
        if first_is_wasm32 && !second_is_wasm32 {
            return resolve_initializer(second, scope, name, false);
        }
        if second_is_wasm32 && !first_is_wasm32 {
            return resolve_initializer(first, scope, name, false);
        }
    }

    let joined = arms
        .iter()
        .map(|field| field.expr.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" | ");
    DefaultValue::Unresolved(joined)
}
