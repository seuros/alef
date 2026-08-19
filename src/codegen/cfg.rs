//! Shared cfg-expression utilities for language binding backends.
//!
//! Provides recursive parsing of Rust `#[cfg(...)]` condition strings and
//! full-surface feature collection so every backend can forward core-crate
//! features into its own Cargo.toml `[features]` table — preventing
//! `unexpected cfg condition value` errors when items are emitted behind
//! `#[cfg(feature = "X")]` guards.

use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use anyhow::Context as _;
use std::collections::BTreeSet;
use std::path::Path;

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
/// `errors[].methods[].cfg` is deliberately NOT walked, unlike in
/// `backends::ffi::gen_bindings::helpers::cbindgen_feature_defines`: no backend re-emits an error
/// method's gate. Every error-introspection wrapper (`codegen::error_gen::gen_ffi_error_methods`
/// and its per-language siblings) is emitted ungated, and `ApiSurface::with_cfg_filtered_deep`
/// drops the method instead when the feature is off, so no crate needs the feature declared. Teach
/// one of those emitters to re-emit `MethodDef::rust_cfg_attribute` and this walk must grow the
/// position with it. ~keep
///
/// The position-by-position coverage of this walk and of `cbindgen_feature_defines` — including
/// the `is_host` asymmetry, which is intentional and must not be collapsed — is pinned by
/// `backends::ffi::gen_bindings::tests::feature_defines`. ~keep
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

/// Feature names [`collect_cfg_features`] finds referenced in `api` that `declared` does not
/// contain.
///
/// A Rust-emitting backend (Magnus/Ruby, Rustler/Elixir) copies a source item's
/// `#[cfg(feature = "X")]` verbatim into the binding crate; that gate then resolves against the
/// *binding* crate's own `[features]` table, not the core crate's. A name this returns means the
/// binding crate's manifest does not declare `X` as its own passthrough feature, so every
/// definition (and, if the backend also re-emits the gate on a registration statement, every
/// registration) behind that gate silently compiles out of the binding crate even though the core
/// crate has `X` on.
#[must_use]
pub fn undeclared_cfg_features(api: &ApiSurface, declared: &BTreeSet<String>) -> BTreeSet<String> {
    collect_cfg_features(api).difference(declared).cloned().collect()
}

/// Read the `[features]` table keys of the Cargo.toml at `manifest_path`.
///
/// Returns `None` when the file cannot be read or parsed as TOML -- e.g. the binding crate has
/// not been scaffolded yet -- so callers can tell "nothing to check" apart from "checked and the
/// table is empty". Returns `Some(<empty set>)` when the file parses but declares no `[features]`
/// table at all, which is the exact shape a manifest has before its first cfg-gated symbol ever
/// existed.
#[must_use]
pub fn read_declared_cargo_features(manifest_path: &Path) -> Option<BTreeSet<String>> {
    let content = std::fs::read_to_string(manifest_path).ok()?;
    // `toml` 1.x's `FromStr for Value` parses a bare value, not a document; use `from_str` or
    // every real Cargo.toml silently yields `None` here. ~keep
    let manifest = toml::from_str::<toml::Value>(&content).ok()?;
    let features = manifest
        .get("features")
        .and_then(toml::Value::as_table)
        .map(|table| table.keys().cloned().collect())
        .unwrap_or_default();
    Some(features)
}

/// Read the feature names the **core** crate itself declares, resolving its manifest via
/// [`crate::scaffold::core_crate_manifest_path`].
///
/// Returns an empty set when the manifest cannot be located (no `workspace_root`, e.g. a
/// standalone scaffold run) or cannot be read/parsed, so a caller forwarding a cfg-gated feature
/// to the core crate can treat "cannot verify" the same as "core does not declare it": inventing
/// a forwarding row `feature = ["<core>/<feature>"]` for a name the core crate does not have
/// breaks `cargo` dependency resolution outright, which is worse than the compile-out this module
/// exists to repair, so silence here must never widen what gets forwarded. ~keep
#[must_use]
pub fn core_crate_declared_features(config: &ResolvedCrateConfig) -> BTreeSet<String> {
    let Some(manifest_path) = crate::scaffold::core_crate_manifest_path(config) else {
        return BTreeSet::new();
    };
    read_declared_cargo_features(&manifest_path).unwrap_or_default()
}

/// Insert every name [`undeclared_cfg_features`] finds missing from `existing`'s own
/// `[features]` table, forwarding each to `core_crate_name` the same way the sibling rows
/// `scaffold_ruby_cargo`/`scaffold_elixir_cargo` already write do (`<feature> =
/// ["<core_crate_name>/<feature>"]`).
///
/// Returns `Ok(None)` when nothing needs to change (every referenced feature is already declared,
/// or every missing one is absent from `core_declared_features` and therefore must not be
/// invented), so callers can distinguish "checked, no update needed" from "wrote the merge"
/// without a further content diff.
///
/// Parses with `toml_edit::DocumentMut`, not the `toml` crate [`read_declared_cargo_features`]
/// uses: `toml_edit` preserves every byte it does not touch -- comments, key order, blank lines,
/// a hand-added `[package.metadata.*]` table -- so the only lines this can ever change are the
/// new feature rows it inserts. A `[features]` table absent from `existing` is created; `toml_edit`
/// appends a new table at the document's end rather than reflowing existing ones, so this never
/// disturbs any other table's position.
///
/// This is `alef scaffold`'s answer to the "re-run `alef scaffold`" remedy the compile-out warning
/// (`warn_on_undeclared_binding_cfg_features`) prescribes: the manifest this repairs is user-owned
/// and `write_scaffold_files_report`'s ownership guard rightly refuses to blindly overwrite it, but
/// a single additive `[features]` row cannot corrupt, reorder, or drop anything else in the file,
/// so it is safe to apply on its own, narrower write path even when the guard would otherwise
/// refuse the whole manifest. ~keep
pub fn merge_missing_cfg_features(
    existing: &str,
    api: &ApiSurface,
    core_crate_name: &str,
    core_declared_features: &BTreeSet<String>,
) -> anyhow::Result<Option<String>> {
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .context("existing manifest is not valid TOML")?;

    let declared: BTreeSet<String> = doc
        .get("features")
        .and_then(toml_edit::Item::as_table)
        .map(|table| table.iter().map(|(key, _)| key.to_string()).collect())
        .unwrap_or_default();

    let missing: BTreeSet<String> = undeclared_cfg_features(api, &declared)
        .into_iter()
        .filter(|feature| core_declared_features.contains(feature))
        .collect();

    if missing.is_empty() {
        return Ok(None);
    }

    let features_table = doc
        .entry("features")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("[features] exists in the manifest but is not a table")?;

    for feature in &missing {
        let mut forwarded = toml_edit::Array::new();
        forwarded.push(format!("{core_crate_name}/{feature}"));
        features_table.insert(feature, toml_edit::Item::Value(toml_edit::Value::Array(forwarded)));
    }

    Ok(Some(doc.to_string()))
}

/// Resolve a `GeneratedFile`-style path (relative to the project root) against
/// `config.workspace_root`, falling back to the process's current directory.
///
/// Mirrors the resolution [`core_crate_declared_features`] uses to read a sibling crate's
/// manifest back off disk, so a caller that has a `resolve_output_dir()` result (itself already
/// relative to the project root) can locate a file next to it on disk.
#[must_use]
pub fn resolve_against_workspace_root(config: &ResolvedCrateConfig, relative: &Path) -> std::path::PathBuf {
    let root = config
        .workspace_root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    root.join(relative)
}

/// Warn when the binding crate's own (already-scaffolded) Cargo.toml at `manifest_path` does not
/// declare every feature the generated Rust source for `language` references via a forwarded
/// `#[cfg(feature = "X")]`.
///
/// `alef scaffold` writes this manifest's `[features]` table once, from
/// [`collect_cfg_features`] evaluated at scaffold time (see `scaffold::languages::ruby` and
/// `scaffold::languages::elixir`); `alef build` does not regenerate it. A cfg-gated item added to
/// the core crate after that scaffold run is therefore referenced by the next `alef build`'s
/// generated source without the manifest ever being told about it -- the exact condition that
/// turned a compiling Ruby extension into one that fails with `E0425: cannot find value`, and an
/// Elixir NIF into one whose function silently returns `:nif_not_loaded` at runtime while the
/// generated docs and type stubs keep advertising it. This is a best-effort, read-only check: a
/// missing or unparseable manifest is treated as "nothing to verify yet", not an error, mirroring
/// `scaffold::languages::elixir::get_core_crate_features`'s same permissive philosophy for the
/// same class of file. ~keep
pub fn warn_on_undeclared_binding_cfg_features(api: &ApiSurface, language: Language, manifest_path: &Path) {
    let Some(declared) = read_declared_cargo_features(manifest_path) else {
        return;
    };
    let missing = undeclared_cfg_features(api, &declared);
    if missing.is_empty() {
        return;
    }
    tracing::warn!(
        language = %language,
        manifest = %manifest_path.display(),
        missing_features = ?missing,
        "generated bindings reference #[cfg(feature = \"...\")] gates this crate's own Cargo.toml \
         does not declare; the affected definitions (and their registrations) will silently \
         compile out of this binding even though the core crate has the feature on -- re-run \
         `alef scaffold` to add the missing features to this crate's [features] table"
    );
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
mod tests;
