//! Same-named function entry re-gating for the Rustler NIF emitter.
//!
//! Rustler discovers NIFs from `#[rustler::nif]`-annotated functions: every annotated function
//! that survives `cfg` evaluation is registered. When a crate exposes the same public function
//! under disjoint `cfg` gates plus an *ungated* fallback, the Rustler emitter would emit one
//! `#[rustler::nif]` definition per surface entry. If more than one of them compiles for a given
//! feature set, Rustler's `on_load` aborts the whole module with
//! `{:error, {:bad_lib, 'Duplicate NIF entry for ...'}}`.
//!
//! The canonical trigger is an inline fallback module paired with cfg-gated real modules:
//!
//! ```ignore
//! #[cfg(feature = "ner")]
//! pub mod ner;                                  // real module: known_models under ner-onnx pair
//!
//! #[cfg(not(feature = "ner"))]
//! pub mod ner { pub fn known_models() { .. } }  // inline stub fallback
//! ```
//!
//! `known_models` is extracted three times: `#[cfg(feature = "ner-onnx")]`,
//! `#[cfg(not(feature = "ner-onnx"))]`, and the inline-stub entry. The extractor does not compose
//! the enclosing `#[cfg(not(feature = "ner"))]` module gate onto the inline-stub function, so its
//! surface entry is *unconditional* — and the emitted NIF compiles alongside whichever
//! `ner-onnx` arm is active, producing the duplicate registration.
//!
//! Unlike the FFI dedup (`backends::ffi::gen_bindings::functions::cfg_dedup`), the Rustler arms
//! have distinct bodies (real impl vs. stub), so they must each survive as their own
//! `#[rustler::nif]` definition. Collapsing them into one entry would drop the cfg-specific bodies.
//! Instead, `regate_ungated_same_name_functions` keeps every gated arm and re-gates each ungated
//! entry to `not(any(<all gated cfgs in the group>))`, so the fallback NIF compiles only when no
//! gated arm does. For the `ner` case the gated arms are exhaustive, so the fallback is gated out
//! entirely — exactly the desired result (no ungated duplicate). All other backends and the e2e
//! validator continue to see the untouched multi-entry surface.

use crate::codegen::cfg::{CfgPredicate, parse_cfg_predicate};
use crate::core::ir::FunctionDef;
use ahash::AHashMap;

/// Returns a `Vec<FunctionDef>` in which any *ungated* entry sharing its `name` with at least one
/// *gated* entry is re-gated to `not(any(<gated cfgs>))`.
///
/// Functions whose `name` is unique, groups with no ungated member, and groups whose members are
/// all ungated pass through unchanged. The relative order of the input is preserved; only the
/// `cfg` field of ungated entries in mixed groups is rewritten.
pub(in crate::backends::rustler::gen_bindings) fn regate_ungated_same_name_functions(
    functions: &[FunctionDef],
) -> Vec<FunctionDef> {
    let groups = collect_function_groups(functions);

    let mut gated_cfg_by_name: AHashMap<&str, Option<String>> = AHashMap::new();
    for (name, indices) in &groups {
        if !is_mixed_gated_group(indices, functions) {
            continue;
        }
        let gated = merge_gated_cfgs(indices.iter().map(|&i| functions[i].cfg.as_deref()));
        gated_cfg_by_name.insert(name.as_str(), gated);
    }

    if gated_cfg_by_name.is_empty() {
        return functions.to_vec();
    }

    functions
        .iter()
        .cloned()
        .map(|mut function| {
            if function.cfg.is_none()
                && let Some(Some(gated)) = gated_cfg_by_name.get(function.name.as_str())
            {
                function.cfg = Some(format!("not({gated})"));
            }
            function
        })
        .collect()
}

fn collect_function_groups(functions: &[FunctionDef]) -> AHashMap<String, Vec<usize>> {
    let mut name_to_indices: AHashMap<String, Vec<usize>> = AHashMap::new();
    for (idx, func) in functions.iter().enumerate() {
        name_to_indices.entry(func.name.clone()).or_default().push(idx);
    }
    name_to_indices
}

/// A group is "mixed-gated" when it has at least one ungated (`cfg = None`) member and at least
/// one gated (`cfg = Some`) member. Only mixed groups carry the duplicate-NIF hazard.
fn is_mixed_gated_group(indices: &[usize], functions: &[FunctionDef]) -> bool {
    if indices.len() <= 1 {
        return false;
    }
    let has_ungated = indices.iter().any(|&i| functions[i].cfg.is_none());
    let has_gated = indices.iter().any(|&i| functions[i].cfg.is_some());
    has_ungated && has_gated
}

/// OR-merge the distinct cfg predicates of the *gated* members of a group.
///
/// Returns `None` when no member is gated (caller guards against this via `is_mixed_gated_group`).
/// A single distinct gated cfg is returned verbatim; multiple are wrapped in `any(...)`.
///
/// Dedup is semantic, not textual: a predicate whose `any(...)` arms (or itself, if it isn't an
/// `any(...)`) are already covered by an already-accumulated predicate's arms is dropped instead of
/// appended. Without this, a group with a compound gated arm (e.g. `any(feature = "a", feature =
/// "b")`) and a second gated arm that is one of that arm's own disjuncts (e.g. bare `feature =
/// "b"`) would OR-merge into the redundant `any(any(feature = "a", feature = "b"), feature = "b")`
/// instead of the equivalent, minimal `any(feature = "a", feature = "b")`. ~keep
fn merge_gated_cfgs<'a>(cfgs: impl Iterator<Item = Option<&'a str>>) -> Option<String> {
    let mut distinct: Vec<(&'a str, CfgPredicate)> = Vec::new();
    for cfg in cfgs.flatten() {
        if distinct.iter().any(|(raw, _)| *raw == cfg) {
            continue;
        }
        let parsed = parse_cfg_predicate(cfg);
        if distinct.iter().any(|(_, existing)| predicate_covers(existing, &parsed)) {
            continue;
        }
        distinct.retain(|(_, existing)| !predicate_covers(&parsed, existing));
        distinct.push((cfg, parsed));
    }
    match distinct.len() {
        0 => None,
        1 => Some(distinct[0].0.to_string()),
        _ => Some(format!(
            "any({})",
            distinct.iter().map(|(raw, _)| *raw).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// A predicate's top-level OR-disjuncts: an `any(...)`'s own arms, or the predicate itself as a
/// single-element list when it isn't an `any(...)`.
fn cfg_disjuncts(predicate: &CfgPredicate) -> Vec<&CfgPredicate> {
    match predicate {
        CfgPredicate::Any(arms) => arms.iter().collect(),
        other => vec![other],
    }
}

/// Whether `covering`'s disjuncts already contain every one of `covered`'s disjuncts, i.e.
/// `covering` holds whenever `covered` does, making `covered` redundant to OR in alongside it.
fn predicate_covers(covering: &CfgPredicate, covered: &CfgPredicate) -> bool {
    let covering_arms = cfg_disjuncts(covering);
    cfg_disjuncts(covered).iter().all(|arm| covering_arms.contains(arm))
}

#[cfg(test)]
mod tests;
