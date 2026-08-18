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
/// by any cfg attribute on a type, field, method, enum variant, service, or
/// top-level function.
///
/// Methods count: a Rust-emitting backend re-emits a gated method's `#[cfg(feature = "X")]`
/// verbatim into its binding crate, so `X` must exist in that crate's `[features]` table or
/// the build fails with `unexpected cfg condition value: X`. ~keep
///
/// Services count for the same reason: `ServiceDef` carries its own `cfg`, and its
/// `constructor`/`configurators` are `MethodDef`s that carry theirs — see
/// `ApiSurface::with_cfg_filtered_deep`, which drops a cfg-gated service the same way it drops a
/// cfg-gated type/enum/function/method, and `backends::ffi::gen_bindings::helpers::cbindgen_feature_defines`,
/// which reads `ServiceDef::cfg` for the FFI header's `#if` guards. A backend that re-emits a
/// gated service's constructor or configurator gate into its own binding crate needs `X` declared
/// here for the same reason a gated method does. ~keep
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
        for method in &typ.methods {
            if let Some(cfg) = &method.cfg {
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
        for method in &enum_def.methods {
            if let Some(cfg) = &method.cfg {
                collect_cfg_feature_names(cfg, &mut out);
            }
        }
    }
    for func in &api.functions {
        if let Some(cfg) = &func.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
    }
    for service in &api.services {
        if !is_host(&service.rust_path) {
            continue;
        }
        if let Some(cfg) = &service.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
        if let Some(cfg) = &service.constructor.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
        for configurator in &service.configurators {
            if let Some(cfg) = &configurator.cfg {
                collect_cfg_feature_names(cfg, &mut out);
            }
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

/// The gate for an item that sits behind both `owner_cfg` and its own `member_cfg`.
///
/// Returns `member_cfg` alone when satisfying it already guarantees `owner_cfg`. A member declared
/// inside `#[cfg(X)] impl T` inherits `X` into its own gate at extraction time, so combining
/// textually yields `all(X, all(X, Y))` — logically right, but it churns the gate line of every
/// affected item on every regen and reads as a generator bug in the diff. ~keep
#[must_use]
pub fn combine_gates(owner_cfg: &str, member_cfg: &str) -> String {
    let (owner, member) = (owner_cfg.trim(), member_cfg.trim());
    if predicate_implies(&parse_cfg_predicate(member), &parse_cfg_predicate(owner)) {
        return member.to_string();
    }
    format!("all({owner}, {member})")
}

/// Whether `predicate` holding guarantees `required` holds.
///
/// Deliberately incomplete: it recognises only conjunction, which is the shape gate inheritance
/// produces. Anything it cannot prove is reported as "does not imply", so the caller keeps both
/// operands — a redundant gate is noise, a dropped one silently compiles the wrong code out.
fn predicate_implies(predicate: &CfgPredicate, required: &CfgPredicate) -> bool {
    // `Other` is the parser's catch-all, so two unrecognised predicates compare equal without
    // being the same condition. Implication must never be inferred from one. ~keep
    if matches!(required, CfgPredicate::Other) {
        return false;
    }
    if predicate == required {
        return true;
    }
    match predicate {
        CfgPredicate::All(arms) => arms.iter().any(|arm| predicate_implies(arm, required)),
        _ => false,
    }
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
    fn combine_gates_drops_an_owner_the_member_already_requires() {
        assert_eq!(
            combine_gates(
                r#"feature = "client""#,
                r#"all(feature = "client", feature = "streaming")"#
            ),
            r#"all(feature = "client", feature = "streaming")"#
        );
    }

    #[test]
    fn combine_gates_keeps_both_operands_when_the_member_does_not_imply_the_owner() {
        assert_eq!(
            combine_gates(r#"feature = "client""#, r#"feature = "streaming""#),
            r#"all(feature = "client", feature = "streaming")"#
        );
    }

    #[test]
    fn combine_gates_does_not_collapse_two_predicates_the_parser_cannot_read() {
        // Both sides parse to `CfgPredicate::Other`, which compares equal without being the same
        // condition. Collapsing here would compile the member in on a target the owner excludes.
        assert_eq!(
            combine_gates("target_os = \"macos\"", "target_os = \"linux\""),
            "all(target_os = \"macos\", target_os = \"linux\")"
        );
    }

    #[test]
    fn combine_gates_does_not_treat_a_disjunct_as_implying_the_owner() {
        // `any(a, b)` holding does not guarantee `a`; only conjunction licenses the drop.
        assert_eq!(
            combine_gates(
                r#"feature = "client""#,
                r#"any(feature = "client", feature = "server")"#
            ),
            r#"all(feature = "client", any(feature = "client", feature = "server"))"#
        );
    }

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
    fn collect_cfg_features_includes_method_gates() {
        // A Rust-emitting backend re-emits a gated method's `#[cfg(feature = "X")]` into its own
        // crate, so `X` must reach that crate's `[features]` table or the build fails with
        // `unexpected cfg condition value`. ~keep
        use crate::core::ir::MethodDef;

        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            types: vec![TypeDef {
                name: "Client".to_string(),
                rust_path: "mylib::Client".to_string(),
                methods: vec![
                    MethodDef {
                        name: "ping".to_string(),
                        ..Default::default()
                    },
                    MethodDef {
                        name: "stream".to_string(),
                        cfg: Some(r#"feature = "streaming""#.to_string()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            enums: vec![EnumDef {
                name: "Format".to_string(),
                rust_path: "mylib::Format".to_string(),
                methods: vec![MethodDef {
                    name: "from_mime".to_string(),
                    cfg: Some(r#"all(feature = "mime", feature = "sniff")"#.to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let want: BTreeSet<String> = ["mime", "sniff", "streaming"].into_iter().map(String::from).collect();
        assert_eq!(collect_cfg_features(&api), want);
    }

    /// A Rust-emitting backend that re-emits a gated service's constructor or configurator gate
    /// (mirroring how it already re-emits a gated method's) needs `X` declared in the manifest for
    /// the same `unexpected cfg condition value` reason `collect_cfg_features_includes_method_gates`
    /// covers for methods. `ServiceDef` and its `constructor`/`configurators` `MethodDef`s were
    /// previously not walked at all, unlike `backends::ffi::gen_bindings::helpers::cbindgen_feature_defines`,
    /// which already reads `ServiceDef::cfg` for the FFI header's `#if` guards — this test pins the
    /// shared helper to the same coverage. ~keep
    #[test]
    fn collect_cfg_features_includes_service_and_configurator_gates() {
        use crate::core::ir::{MethodDef, ServiceDef};

        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            services: vec![ServiceDef {
                name: "ClientConfig".to_string(),
                rust_path: "mylib::client::ClientConfig".to_string(),
                constructor: MethodDef {
                    name: "new".to_string(),
                    ..Default::default()
                },
                configurators: vec![
                    MethodDef {
                        name: "with_timeout".to_string(),
                        ..Default::default()
                    },
                    MethodDef {
                        name: "with_tower_layer".to_string(),
                        cfg: Some(r#"feature = "tower""#.to_string()),
                        ..Default::default()
                    },
                ],
                registrations: vec![],
                entrypoints: vec![],
                doc: String::new(),
                cfg: None,
            }],
            ..Default::default()
        };

        let want: BTreeSet<String> = ["tower".to_string()].into_iter().collect();
        assert_eq!(collect_cfg_features(&api), want);
    }

    /// A service merged from a foreign `[[crates.source_crates]]` crate must not forward its cfg
    /// to the host crate's `[features]` table, for the same reason a merged type/enum doesn't
    /// (see `collect_cfg_features_excludes_external_source_crate_cfgs`) — forwarding would
    /// reference a feature the host crate does not define.
    #[test]
    fn collect_cfg_features_excludes_external_source_crate_service_cfgs() {
        use crate::core::ir::{MethodDef, ServiceDef};

        let api = ApiSurface {
            crate_name: "hostlib".to_string(),
            services: vec![ServiceDef {
                name: "OtherService".to_string(),
                rust_path: "otherlib::OtherService".to_string(),
                constructor: MethodDef::default(),
                configurators: vec![],
                registrations: vec![],
                entrypoints: vec![],
                doc: String::new(),
                cfg: Some(r#"feature = "foreign-only""#.to_string()),
            }],
            ..Default::default()
        };

        assert!(
            collect_cfg_features(&api).is_empty(),
            "a foreign-owned service's cfg must not forward to the host crate"
        );
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
