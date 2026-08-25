//! Tests for the Rustler-local same-name function re-gating pass.

use super::regate_ungated_same_name_functions;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

fn make_param(name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::String,
        ..Default::default()
    }
}

fn make_fn(name: &str, rust_path: &str, cfg: Option<&str>, param_names: &[&str]) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
        original_rust_path: String::new(),
        params: param_names.iter().map(|n| make_param(n)).collect(),
        return_type: TypeRef::String,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: cfg.map(|s| s.to_string()),
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn normalize_cfg(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The canonical fallback shape: two disjoint gated arms plus an ungated stub fallback.
/// The two gated arms must survive untouched, and the ungated stub must be re-gated to
/// `not(any(<both gated cfgs>))` so it never compiles alongside an active arm.
#[test]
fn regates_ungated_fallback_against_gated_arms() {
    let input = vec![
        make_fn(
            "known_models",
            "krate::text::ner::known_models",
            Some(r#"feature = "ner-onnx""#),
            &[],
        ),
        make_fn(
            "known_models",
            "krate::text::ner::known_models",
            Some(r#"not(feature = "ner-onnx")"#),
            &[],
        ),
        make_fn("known_models", "krate::text::ner::known_models", None, &[]),
    ];

    let out = regate_ungated_same_name_functions(&input);

    assert_eq!(
        out.len(),
        3,
        "all three arms must survive — only the ungated one is re-gated"
    );
    assert_eq!(
        out[0].cfg.as_deref(),
        Some(r#"feature = "ner-onnx""#),
        "first gated arm must be untouched"
    );
    assert_eq!(
        out[1].cfg.as_deref(),
        Some(r#"not(feature = "ner-onnx")"#),
        "second gated arm must be untouched"
    );

    let fallback_cfg = out[2].cfg.as_deref().expect("the ungated fallback must now be gated");
    let norm = normalize_cfg(fallback_cfg);
    assert!(
        norm.starts_with("not(any("),
        "fallback must be gated as not(any(...)); got: {fallback_cfg}"
    );
    assert!(
        norm.contains(&normalize_cfg(r#"feature = "ner-onnx""#)),
        "fallback gate must reference the first arm's cfg; got: {fallback_cfg}"
    );
    assert!(
        norm.contains(&normalize_cfg(r#"not(feature = "ner-onnx")"#)),
        "fallback gate must reference the second arm's cfg; got: {fallback_cfg}"
    );
}

/// A single gated arm plus an ungated fallback re-gates to `not(<that cfg>)` without the `any(...)`
/// wrapper.
#[test]
fn single_gated_arm_regates_without_any_wrapper() {
    let input = vec![
        make_fn(
            "download_model",
            "krate::download_model",
            Some(r#"feature = "ner""#),
            &[],
        ),
        make_fn("download_model", "krate::download_model", None, &[]),
    ];

    let out = regate_ungated_same_name_functions(&input);

    assert_eq!(out.len(), 2);
    assert_eq!(out[0].cfg.as_deref(), Some(r#"feature = "ner""#));
    assert_eq!(
        out[1].cfg.as_deref(),
        Some(r#"not(feature = "ner")"#),
        "single-arm fallback must be gated as not(<cfg>) with no any(...) wrapper"
    );
}

/// Disjoint gated arms with NO ungated fallback are left completely untouched — they are already
/// mutually exclusive and Rustler compiles exactly one.
#[test]
fn leaves_purely_gated_disjoint_arms_untouched() {
    let input = vec![
        make_fn(
            "default_model_name",
            "krate::default_model_name",
            Some(r#"feature = "ner-onnx""#),
            &[],
        ),
        make_fn(
            "default_model_name",
            "krate::default_model_name",
            Some(r#"not(feature = "ner-onnx")"#),
            &[],
        ),
    ];

    let out = regate_ungated_same_name_functions(&input);

    assert_eq!(out.len(), 2);
    assert_eq!(out[0].cfg.as_deref(), Some(r#"feature = "ner-onnx""#));
    assert_eq!(out[1].cfg.as_deref(), Some(r#"not(feature = "ner-onnx")"#));
}

/// A unique ungated function is not a duplicate hazard and must pass through unchanged.
#[test]
fn leaves_unique_ungated_function_untouched() {
    let input = vec![make_fn("extract_bytes", "krate::extract_bytes", None, &["bytes"])];
    let out = regate_ungated_same_name_functions(&input);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].cfg.is_none(),
        "a unique ungated function must remain unconditional"
    );
}

/// Two ungated entries with the same name (no gated arm) are left untouched — there is no gated cfg
/// to anchor a `not(...)` against, so this pass does not invent one.
#[test]
fn leaves_all_ungated_group_untouched() {
    let input = vec![
        make_fn("known_models", "krate::known_models", None, &[]),
        make_fn("known_models", "krate::known_models", None, &[]),
    ];
    let out = regate_ungated_same_name_functions(&input);
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|f| f.cfg.is_none()));
}

/// The pass preserves the relative order of every entry and only mutates `cfg`.
#[test]
fn preserves_order_and_only_mutates_cfg() {
    let input = vec![
        make_fn("before", "krate::before", None, &[]),
        make_fn(
            "known_models",
            "krate::known_models",
            Some(r#"feature = "ner-onnx""#),
            &[],
        ),
        make_fn("known_models", "krate::known_models", None, &[]),
        make_fn("after", "krate::after", None, &[]),
    ];

    let out = regate_ungated_same_name_functions(&input);

    let names: Vec<&str> = out.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["before", "known_models", "known_models", "after"]);
    assert!(out[0].cfg.is_none(), "unrelated `before` stays ungated");
    assert!(out[3].cfg.is_none(), "unrelated `after` stays ungated");
    assert_eq!(out[2].cfg.as_deref(), Some(r#"not(feature = "ner-onnx")"#));
}

/// Two gated arms where one's cfg is already one of the other's `any(...)` disjuncts must not
/// nest into `any(any(a, b), b)` when OR-merged for the fallback's `not(...)` gate — the merge is
/// semantic (by disjunct set), not by raw predicate text.
#[test]
fn merges_gated_cfgs_by_disjunct_not_by_raw_text() {
    let input = vec![
        make_fn(
            "score_pair",
            "krate::score_pair",
            Some(r#"any(feature = "scoring-presets", feature = "scoring")"#),
            &[],
        ),
        make_fn(
            "score_pair",
            "krate::score_pair",
            Some(r#"feature = "scoring-presets""#),
            &[],
        ),
        make_fn("score_pair", "krate::score_pair", None, &[]),
    ];

    let out = regate_ungated_same_name_functions(&input);

    assert_eq!(out.len(), 3);
    assert_eq!(
        out[0].cfg.as_deref(),
        Some(r#"any(feature = "scoring-presets", feature = "scoring")"#),
        "gated arms are never rewritten, only referenced"
    );
    assert_eq!(
        out[1].cfg.as_deref(),
        Some(r#"feature = "scoring-presets""#),
        "gated arms are never rewritten, only referenced"
    );

    let fallback_cfg = out[2].cfg.as_deref().expect("the ungated fallback must now be gated");
    assert_eq!(
        fallback_cfg, r#"not(any(feature = "scoring-presets", feature = "scoring"))"#,
        "the second arm's cfg is already one of the first arm's disjuncts, so OR-merging it again \
         must not produce a redundant any(any(...), ...) gate; got: {fallback_cfg}"
    );
}

/// End-to-end proof that the Rustler pipeline — `dedup_same_name_functions` then
/// `regate_ungated_same_name_functions`, exactly as `native.rs`'s `generate_bindings` chains
/// them — protects against a disagreeing-signature group the same way the FFI and single-surface
/// backends do.
///
/// This file's own pass never merges or discards an entry: it only rewrites the `cfg` of
/// already-ungated members, and a purely-gated group (no ungated member) is left completely
/// untouched regardless of the gated arms' signatures — see
/// `leaves_purely_gated_disjoint_arms_untouched` above. So `regate_ungated_same_name_functions`
/// has no `pick_canonical_entry`-equivalent step and cannot itself silently drop a real
/// signature; a `real_variant_signatures_agree`-style guard added here would never have anything
/// to reject. The merge/collapse step Rustler shares with every other emitting backend is
/// `codegen::fn_dedup::dedup_same_name_functions`, called first in `native.rs` (before this
/// file's pass ever runs) — its own `real_variant_signatures_agree` guard is what protects this
/// pipeline. This test pins that end-to-end behaviour so the guarantee is proven against the
/// actual call chain, not assumed. ~keep
#[test]
fn full_pipeline_preserves_disagreeing_real_signatures() {
    let input = vec![
        make_fn(
            "compute",
            "krate::compute",
            Some(r#"feature = "fast""#),
            &["x", "precision"],
        ),
        make_fn("compute", "krate::compute", Some(r#"not(feature = "fast")"#), &["x"]),
    ];

    let deduped = crate::codegen::fn_dedup::dedup_same_name_functions(&input);
    let out = regate_ungated_same_name_functions(&deduped);

    assert_eq!(
        out.len(),
        2,
        "disagreeing real signatures must reach the Rustler NIF emitter as two distinct entries: {out:?}"
    );
    assert_eq!(out[0].params.len(), 2);
    assert_eq!(out[1].params.len(), 1);
}

/// The pass is a pure transformation — it must not mutate the input slice.
#[test]
fn does_not_mutate_input() {
    let input = vec![
        make_fn(
            "known_models",
            "krate::known_models",
            Some(r#"feature = "ner-onnx""#),
            &[],
        ),
        make_fn("known_models", "krate::known_models", None, &[]),
    ];
    let snapshot = input.clone();

    let _ = regate_ungated_same_name_functions(&input);

    for (before, after) in snapshot.iter().zip(input.iter()) {
        assert_eq!(before.cfg, after.cfg, "input cfg must be untouched");
    }
}
