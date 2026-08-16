//! Shared cfg-expression utilities for language binding backends.
//!
//! Provides recursive parsing of Rust `#[cfg(...)]` condition strings and
//! full-surface feature collection so every backend can forward core-crate
//! features into its own Cargo.toml `[features]` table — preventing
//! `unexpected cfg condition value` errors when items are emitted behind
//! `#[cfg(feature = "X")]` guards.

use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::collections::BTreeSet;

/// Extract every `feature = "X"` name referenced by a cfg expression.
///
/// Recursively descends through `any(...)`, `all(...)`, and `not(...)` so that
/// callers can declare a passthrough Cargo feature for every feature the
/// generated source references. Without this, items emitted behind
/// `#[cfg(feature = "X")]` produce
/// `error: unexpected cfg condition value: X` when the binding crate's
/// `Cargo.toml` only declares an unrelated feature list.
///
/// The IR encodes cfgs via `proc_macro2::TokenStream::to_string()`, which
/// inserts whitespace between tokens (e.g. `any (feature = "a" , ...)`); the
/// evaluator normalises that before parsing.
///
/// Unknown cfg patterns (`target_arch`, `target_os`, ...) yield no features
/// — those are recognised by Cargo directly and don't need passthroughs.
pub fn collect_cfg_feature_names(cfg_str: &str, out: &mut BTreeSet<String>) {
    let normalized = cfg_str.trim().replace(" (", "(");
    let cfg_str = normalized.as_str();

    if let Some(feature) = cfg_str.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"')) {
        out.insert(feature.to_string());
        return;
    }
    if let Some(inner) = cfg_str
        .strip_prefix("any(")
        .and_then(|s| s.strip_suffix(')'))
        .or_else(|| cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')))
    {
        for cond in parse_cfg_list(inner) {
            collect_cfg_feature_names(&cond, out);
        }
        return;
    }
    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        collect_cfg_feature_names(inner.trim(), out);
    }
}

/// Walk the full [`ApiSurface`] and return the set of feature names referenced
/// by any cfg attribute on a type, field, enum variant, or top-level function.
///
/// The set is sorted (via `BTreeSet`) so the resulting Cargo.toml is stable
/// across regenerations.
pub fn collect_cfg_features(api: &ApiSurface) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    // Forwarding features (`<feat> = ["<core>/<feat>"]`) are only valid for the HOST crate's own ~keep
    // features. Types merged from `[[crates.source_crates]]` carry the foreign crate's cfg gates ~keep
    // (e.g. a variant gated on a feature only the source crate defines); forwarding those to the ~keep
    // core dep references a feature the core crate does not define and breaks `cargo` resolution. ~keep
    // Skip any type/enum whose rust_path is not owned by the host crate. ~keep
    let host_crate = api.crate_name.replace('-', "_");
    let is_host = |rust_path: &str| -> bool {
        // Unknown host (empty crate name) or an unqualified path → keep the old, permissive ~keep
        // behavior; only skip a type whose leading path segment names a *different* crate. ~keep
        if host_crate.is_empty() {
            return true;
        }
        match rust_path.split("::").next() {
            Some(first) if !first.is_empty() => first == host_crate,
            _ => true,
        }
    };
    for typ in &api.types {
        if !is_host(&typ.rust_path) {
            continue;
        }
        if let Some(cfg) = &typ.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
        for field in &typ.fields {
            if let Some(cfg) = &field.cfg {
                collect_cfg_feature_names(cfg, &mut out);
            }
        }
    }
    for enum_def in &api.enums {
        if !is_host(&enum_def.rust_path) {
            continue;
        }
        if let Some(cfg) = &enum_def.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
        for variant in &enum_def.variants {
            if let Some(cfg) = &variant.cfg {
                collect_cfg_feature_names(cfg, &mut out);
            }
        }
    }
    for func in &api.functions {
        if let Some(cfg) = &func.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
    }
    out
}

/// Warn when a single-surface binding language's configured feature set (used to decide which
/// `#[cfg(feature = "...")]`-gated FFI exports get glue via [`ApiSurface::with_cfg_filtered_deep`])
/// diverges from the FFI crate's own configured feature set.
///
/// The FFI cdylib is built once, shared by every language binding (`cargo build -p
/// {ffi_crate}` runs with no `--features` override — see `cli::pipeline::commands::build`), and
/// its `[features] default = [...]` list is populated from `features_for_language(Language::Ffi)`
/// (see `scaffold::languages::ffi`). A binding language's own `with_cfg_filtered_deep` call
/// assumes its configured feature set describes that same compiled artifact; if the two lists
/// differ, the omission-based filter is filtering against the wrong assumption — the generated
/// glue can still reference a symbol the shipped library doesn't export, or omit one it does.
/// alef cannot detect the actual mismatch (it doesn't run `cargo build` here), so this only
/// flags the config-level drift that would cause it, naming the assumption so it is a visible,
/// documented constraint rather than a silent landmine. ~keep
pub fn warn_on_ffi_feature_drift(config: &ResolvedCrateConfig, lang: Language) {
    if lang == Language::Ffi {
        return;
    }
    let lang_features: BTreeSet<&str> = config.features_for_language(lang).iter().map(String::as_str).collect();
    let ffi_features: BTreeSet<&str> = config
        .features_for_language(Language::Ffi)
        .iter()
        .map(String::as_str)
        .collect();
    if lang_features != ffi_features {
        tracing::warn!(
            language = %lang,
            lang_features = ?lang_features,
            ffi_features = ?ffi_features,
            "configured feature set for this binding differs from [crates.ffi]'s; cfg-gated FFI \
             exports are included/omitted based on this binding's own feature list, but the \
             linked native library is built once using the FFI crate's feature list — keep them \
             in sync (or set them explicitly to the same value) or generated glue may reference \
             symbols the shipped library doesn't export"
        );
    }
}

/// A parsed `#[cfg(...)]` predicate, preserving `any`/`all`/`not` structure instead of
/// flattening straight to a name set. Needed by callers that must decide what to *do* about an
/// unsatisfied predicate (e.g. which single feature to request to satisfy an `any(...)`) rather
/// than just enumerate every name it mentions — [`collect_cfg_feature_names`] remains the right
/// tool for the latter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfgPredicate {
    /// `feature = "X"`.
    Feature(String),
    /// `all(...)`: every arm must hold.
    All(Vec<CfgPredicate>),
    /// `any(...)`: at least one arm must hold.
    Any(Vec<CfgPredicate>),
    /// `not(...)`.
    Not(Box<CfgPredicate>),
    /// Anything this parser doesn't recognise (`target_arch = "..."`, `windows`, ...).
    Other,
}

/// Parse a `#[cfg(...)]` condition string into a [`CfgPredicate`] tree.
pub fn parse_cfg_predicate(cfg_str: &str) -> CfgPredicate {
    let normalized = cfg_str.trim().replace(" (", "(");
    let cfg_str = normalized.as_str();

    if let Some(feature) = cfg_str.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"')) {
        return CfgPredicate::Feature(feature.to_string());
    }
    if let Some(inner) = cfg_str.strip_prefix("any(").and_then(|s| s.strip_suffix(')')) {
        return CfgPredicate::Any(parse_cfg_list(inner).iter().map(|c| parse_cfg_predicate(c)).collect());
    }
    if let Some(inner) = cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')) {
        return CfgPredicate::All(parse_cfg_list(inner).iter().map(|c| parse_cfg_predicate(c)).collect());
    }
    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        return CfgPredicate::Not(Box::new(parse_cfg_predicate(inner.trim())));
    }
    CfgPredicate::Other
}

fn parse_cfg_list(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    result.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, TypeDef};

    #[test]
    fn collect_cfg_feature_names_simple_feature() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"feature = "pdf""#, &mut out);
        assert_eq!(out, BTreeSet::from(["pdf".to_string()]));
    }

    #[test]
    fn collect_cfg_feature_names_any_compound() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"any(feature = "html", feature = "xml")"#, &mut out);
        let want: BTreeSet<String> = ["html", "xml"].into_iter().map(String::from).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn collect_cfg_feature_names_all_compound() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(
            r#"all(feature = "layout-types", not(feature = "wasm-target"))"#,
            &mut out,
        );
        let want: BTreeSet<String> = ["layout-types", "wasm-target"].into_iter().map(String::from).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn parse_cfg_predicate_simple_feature() {
        assert_eq!(
            parse_cfg_predicate(r#"feature = "tokenizer""#),
            CfgPredicate::Feature("tokenizer".to_string())
        );
    }

    #[test]
    fn parse_cfg_predicate_any_preserves_arms() {
        assert_eq!(
            parse_cfg_predicate(r#"any(feature = "native-http", feature = "wasm-http")"#),
            CfgPredicate::Any(vec![
                CfgPredicate::Feature("native-http".to_string()),
                CfgPredicate::Feature("wasm-http".to_string()),
            ])
        );
    }

    #[test]
    fn parse_cfg_predicate_all_preserves_arms() {
        assert_eq!(
            parse_cfg_predicate(r#"all(feature = "layout-types", not(feature = "wasm-target"))"#),
            CfgPredicate::All(vec![
                CfgPredicate::Feature("layout-types".to_string()),
                CfgPredicate::Not(Box::new(CfgPredicate::Feature("wasm-target".to_string()))),
            ])
        );
    }

    #[test]
    fn parse_cfg_predicate_not() {
        assert_eq!(
            parse_cfg_predicate(r#"not(feature = "wasm-target")"#),
            CfgPredicate::Not(Box::new(CfgPredicate::Feature("wasm-target".to_string())))
        );
    }

    #[test]
    fn parse_cfg_predicate_unrecognised_is_other() {
        assert_eq!(parse_cfg_predicate(r#"target_arch = "wasm32""#), CfgPredicate::Other);
    }

    #[test]
    fn collect_cfg_feature_names_ignores_non_feature_cfg() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"target_arch = "wasm32""#, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn collect_cfg_feature_names_whitespace_normalisation() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"any (feature = "a" , feature = "b")"#, &mut out);
        let want: BTreeSet<String> = ["a", "b"].into_iter().map(String::from).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn collect_cfg_features_walks_types_enums_functions() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"feature = "pdf""#, &mut out);
        collect_cfg_feature_names(r#"any(feature = "html", feature = "xml")"#, &mut out);
        collect_cfg_feature_names(
            r#"all(feature = "layout-types", not(feature = "wasm-target"))"#,
            &mut out,
        );
        collect_cfg_feature_names(r#"target_arch = "wasm32""#, &mut out);
        let want: BTreeSet<String> = ["html", "layout-types", "pdf", "wasm-target", "xml"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(out, want);
    }

    #[test]
    fn collect_cfg_features_full_surface_walk() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "PdfDoc".to_string(),
                rust_path: "mylib::PdfDoc".to_string(),
                cfg: Some(r#"feature = "pdf""#.to_string()),
                ..Default::default()
            }],
            enums: vec![EnumDef {
                name: "ImageOutputFormat".to_string(),
                variants: vec![
                    EnumVariant {
                        name: "Native".to_string(),
                        cfg: None,
                        ..Default::default()
                    },
                    EnumVariant {
                        name: "Heic".to_string(),
                        cfg: Some(r#"feature = "heic""#.to_string()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let features = collect_cfg_features(&api);
        let want: BTreeSet<String> = ["heic", "pdf"].into_iter().map(String::from).collect();
        assert_eq!(features, want);
    }

    #[test]
    fn collect_cfg_features_excludes_external_source_crate_cfgs() {
        // A type/enum merged from `[[crates.source_crates]]` carries the foreign crate's rust_path ~keep
        // and cfg gates. Its features must NOT be forwarded to the host crate (they'd map to ~keep
        // `<host>/<feat>` for a feature the host does not define, breaking cargo resolution). ~keep
        let api = ApiSurface {
            crate_name: "hostlib".to_string(),
            types: vec![TypeDef {
                name: "HostDoc".to_string(),
                rust_path: "hostlib::HostDoc".to_string(),
                cfg: Some(r#"feature = "pdf""#.to_string()),
                ..Default::default()
            }],
            enums: vec![EnumDef {
                name: "Strategy".to_string(),
                rust_path: "otherlib::Strategy".to_string(),
                variants: vec![
                    EnumVariant {
                        name: "Auto".to_string(),
                        cfg: None,
                        ..Default::default()
                    },
                    EnumVariant {
                        name: "Advanced".to_string(),
                        cfg: Some(r#"any(test, feature = "foreign-only")"#.to_string()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let features = collect_cfg_features(&api);
        assert_eq!(
            features,
            BTreeSet::from(["pdf".to_string()]),
            "host `pdf` must forward; the foreign `foreign-only` feature must not leak into host passthrough"
        );
    }
}
