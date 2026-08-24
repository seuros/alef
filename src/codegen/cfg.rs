//! Shared cfg-expression utilities for language binding backends.
//!
//! Provides recursive parsing of Rust `#[cfg(...)]` condition strings and
//! full-surface feature collection so every backend can forward core-crate
//! features into its own Cargo.toml `[features]` table — preventing
//! `unexpected cfg condition value` errors when items are emitted behind
//! `#[cfg(feature = "X")]` guards.

use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, cfg_feature_satisfied};
use anyhow::Context as _;
use std::collections::{BTreeSet, HashSet};
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
/// A flattening of [`collect_cfg_gates`], which owns the walk itself: this answers "which
/// feature names must the binding crate's `[features]` table declare", while the gate set
/// answers "which conditions decide whether an item is emitted". Both questions are asked of the
/// same positions, and deriving one from the other is what keeps them from drifting apart. ~keep
pub fn collect_cfg_features(api: &ApiSurface) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for gate in collect_cfg_gates(api) {
        collect_cfg_feature_names(&gate, &mut out);
    }
    out
}

/// Walk the full [`ApiSurface`] and return every distinct `#[cfg(...)]` condition string
/// attached to a type, field, method, enum variant, service, or top-level function.
///
/// The whole expression, not the feature names inside it: a caller that must decide whether an
/// item is *emitted* has to evaluate the condition (via
/// [`crate::core::ir::cfg_feature_satisfied`], the same evaluator
/// [`ApiSurface::with_cfg_filtered_deep`] filters with), and a name set cannot answer that. Set
/// difference over names says `any(feature = "a", feature = "b")` is unsatisfied for a binding
/// that enables only `a`, and says every gate is unsatisfied for a binding that enables the
/// umbrella `full` — both wrong, both a warning that fires with false content. ~keep
///
/// Which positions are walked, and why each one counts:
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
/// The set is sorted (via `BTreeSet`) so every derived artifact — a Cargo.toml `[features]`
/// list, a warning's reported gate list — is stable across regenerations.
#[must_use]
pub fn collect_cfg_gates(api: &ApiSurface) -> BTreeSet<String> {
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
            out.insert(cfg.clone());
        }
        for field in &typ.fields {
            if let Some(cfg) = &field.cfg {
                out.insert(cfg.clone());
            }
        }
        for method in &typ.methods {
            if let Some(cfg) = &method.cfg {
                out.insert(cfg.clone());
            }
        }
    }
    for enum_def in &api.enums {
        if !is_host(&enum_def.rust_path) {
            continue;
        }
        if let Some(cfg) = &enum_def.cfg {
            out.insert(cfg.clone());
        }
        for variant in &enum_def.variants {
            if let Some(cfg) = &variant.cfg {
                out.insert(cfg.clone());
            }
        }
        for method in &enum_def.methods {
            if let Some(cfg) = &method.cfg {
                out.insert(cfg.clone());
            }
        }
    }
    for func in &api.functions {
        if let Some(cfg) = &func.cfg {
            out.insert(cfg.clone());
        }
    }
    for service in &api.services {
        if !is_host(&service.rust_path) {
            continue;
        }
        if let Some(cfg) = &service.cfg {
            out.insert(cfg.clone());
        }
        if let Some(cfg) = &service.constructor.cfg {
            out.insert(cfg.clone());
        }
        for configurator in &service.configurators {
            if let Some(cfg) = &configurator.cfg {
                out.insert(cfg.clone());
            }
        }
    }
    out
}

/// The full set of Cargo features the generated FFI crate's `Cargo.toml` enables by default,
/// once `scaffold::languages::ffi::scaffold_ffi` writes it: [`Language::Ffi`]'s configured
/// feature list (minus `serde`, which is a passthrough dependency, never a default) unioned with
/// every feature name [`collect_cfg_features`] finds referenced by an emitted
/// `#[cfg(feature = "X")]` gate in the FFI surface, excluding any name declared in
/// `[crates.ffi].extra_features` -- those stay declare-only by design (mutually-exclusive
/// alternatives such as a `wasm-http` backend forwarding feature).
///
/// This is the ONE derivation of "what does the compiled FFI cdylib actually build with by
/// default". `scaffold_ffi` must build its `[features] default = [...]` list from exactly this,
/// and [`warn_on_ffi_feature_drift`] must compare a binding language's configured feature set
/// against exactly this -- never against `features_for_language(Language::Ffi)` a second time --
/// because the FFI cdylib is built once from this effective set, not from its own configured
/// list alone. Two call sites re-deriving the same answer is exactly how this repo's FFI feature
/// drift warning went blind to the drift it exists to catch (see
/// `github.com/xberg-io/alef/issues/257`): the warning compared configured-against-configured
/// while the scaffolder had long since started unioning in `collect_cfg_features`. ~keep
///
/// Preserves the FFI language config's own feature order first, then the cfg-discovered names in
/// [`collect_cfg_features`]'s sorted order -- matching the order `scaffold_ffi` has always
/// emitted the `default = [...]` list in.
#[must_use]
pub fn effective_ffi_default_features(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<String> {
    let passthrough: Vec<&str> = config
        .features_for_language(Language::Ffi)
        .iter()
        .map(String::as_str)
        .filter(|f| *f != "serde")
        .collect();
    let extra_declared: &[String] = config.ffi.as_ref().map(|c| c.extra_features.as_slice()).unwrap_or(&[]);
    let emitted: Vec<String> = collect_cfg_features(api)
        .into_iter()
        .filter(|name| {
            !name.is_empty()
                && name != "serde"
                && !passthrough.contains(&name.as_str())
                && !extra_declared.iter().any(|declared| declared == name)
        })
        .collect();
    passthrough.into_iter().map(str::to_string).chain(emitted).collect()
}

/// Feature names [`collect_cfg_features`] finds referenced in `api` that `present` does not
/// contain.
///
/// A plain set difference, reused for two different meanings of "present": callers pass the
/// manifest's declared `[features]` keys to find names missing a forwarding row at all, and pass
/// the set [`features_reachable_from_default`] computes to find names that are declared but not
/// actually turned on. ~keep
///
/// A Rust-emitting backend (Magnus/Ruby, Rustler/Elixir) copies a source item's
/// `#[cfg(feature = "X")]` verbatim into the binding crate; that gate then resolves against the
/// *binding* crate's own `[features]` table, not the core crate's. A name this returns means the
/// binding crate's manifest does not declare (or does not enable) `X` as its own passthrough
/// feature, so every definition (and, if the backend also re-emits the gate on a registration
/// statement, every registration) behind that gate silently compiles out of the binding crate
/// even though the core crate has `X` on.
#[must_use]
pub fn undeclared_cfg_features(api: &ApiSurface, present: &BTreeSet<String>) -> BTreeSet<String> {
    collect_cfg_features(api).difference(present).cloned().collect()
}

/// Feature names transitively enabled when `default` is enabled, per `members_of` -- a lookup
/// from a feature name to the *local* (same-manifest) feature names its own value array lists.
///
/// Cargo enables a feature and everything its value array names, recursively; this walks that
/// graph starting at `default`. `members_of` is expected to already have dropped any
/// `crate/feature` forwarding target: that name lives in a different crate's feature graph, not
/// this manifest's own, and a `#[cfg(feature = "X")]` gate in this crate's generated source can
/// only ever be checking a name from this crate's own graph. Declaring "default" itself is never
/// reported as enabled -- it names the entry point into the graph, not a feature a generated
/// `#[cfg(feature = "default")]` gate could reference. A name absent from the table (queried but
/// with no value array) simply contributes no further members, so a leaf forwarding feature such
/// as `tokenizer = ["core/tokenizer"]` terminates the walk instead of erroring. ~keep
fn features_reachable_from_default(members_of: impl Fn(&str) -> Vec<String>) -> BTreeSet<String> {
    let mut enabled = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut queue = std::collections::VecDeque::from([String::from("default")]);
    while let Some(name) = queue.pop_front() {
        if !visited.insert(name.clone()) {
            continue;
        }
        if name != "default" {
            enabled.insert(name.clone());
        }
        queue.extend(members_of(&name));
    }
    enabled
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

/// Read the feature names transitively **enabled** (not merely declared) when `default` is
/// enabled, per the `[features]` table of the Cargo.toml at `manifest_path`.
///
/// This is what `alef` can prove statically from the manifest alone: it walks the local feature
/// graph reachable from `default` via [`features_reachable_from_default`]. It cannot see a
/// `--features` flag some external build tool (`mix`, `rake-compiler`, `cargo build
/// --no-default-features --features X`, ...) passes at build time, nor a workspace-level feature
/// unification pulling this crate in through another member -- none of that is visible from the
/// manifest on disk. A feature this returns is provably on; a feature this omits might still be
/// turned on some other way alef cannot observe, so the honest question this answers is narrower
/// than "is this feature enabled" and reads as "is this feature enabled by default". ~keep
///
/// Returns `None` for the same reasons [`read_declared_cargo_features`] does (unreadable/
/// unparseable manifest); returns `Some(<empty set>)` when the manifest has no `[features]`
/// table, or has one with no `default` key, at all.
#[must_use]
pub fn read_default_enabled_cargo_features(manifest_path: &Path) -> Option<BTreeSet<String>> {
    let content = std::fs::read_to_string(manifest_path).ok()?;
    let manifest = toml::from_str::<toml::Value>(&content).ok()?;
    let features_table = manifest.get("features").and_then(toml::Value::as_table);
    Some(features_reachable_from_default(|name| {
        features_table
            .and_then(|table| table.get(name))
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
            .filter(|member| !member.contains('/'))
            .map(str::to_owned)
            .collect()
    }))
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

/// Expand a configured feature list into every feature name it actually turns on, by following
/// the core crate's own `[features]` table.
///
/// Cargo enables an aggregate feature's members transitively, but
/// [`crate::core::ir::cfg_feature_satisfied`] compares a `#[cfg(feature = "X")]` gate against the
/// configured list LITERALLY, with exactly one umbrella name -- `full` -- hard-coded as
/// satisfying every gate. So a consumer whose core crate defines its own aggregate (the
/// `android-target` shape [`crate::scaffold::android_target_feature_line`] already exists to
/// serve) and who configures that aggregate name for a binding, or for one target of a binding,
/// gets every `#[cfg(feature = "<member>")]` item silently dropped from that surface even though
/// the build the aggregate describes compiles them. That is underexposure with no diagnostic:
/// alef's own scaffolder expands the same aggregate into its members when it writes the binding
/// crate's `[features]` table, so the two derivations disagree about what the artifact contains.
/// Expanding the list before it is used as an enabled-feature set makes the filter agree with the
/// manifest alef itself emits. ~keep
///
/// Falls back to `requested` verbatim when the core manifest cannot be located, read, or parsed
/// -- the same permissive fallback [`core_crate_declared_features`] takes. Widening the enabled
/// set is only ever safe when the manifest proves the members are enabled; an unreadable manifest
/// proves nothing, so it must leave the caller exactly where it was. ~keep
#[must_use]
pub fn expand_configured_features(config: &ResolvedCrateConfig, requested: &[String]) -> Vec<String> {
    crate::scaffold::core_feature_closure(config, requested)
        .0
        .into_iter()
        .collect()
}

/// Insert every name [`undeclared_cfg_features`] finds missing from `existing`'s own
/// `[features]` table -- forwarding each to `core_crate_name` the same way the sibling rows
/// `scaffold_ruby_cargo`/`scaffold_elixir_cargo` already write do (`<feature> =
/// ["<core_crate_name>/<feature>"]`) -- and, separately, every referenced name missing from
/// `default`, appending it to that array.
///
/// Declaring a Cargo feature does not enable it: a forwarding row alone leaves `#[cfg(feature =
/// "X")]` false unless something turns `X` on, and neither `mix`/`rake-compiler`/`cargo` build
/// wrapper this repair supports passes a `--features` flag. `scaffold_ruby_cargo` and
/// `scaffold_elixir_cargo` already put every name [`collect_cfg_features`] finds straight into
/// `default` on a fresh scaffold (see their own `default = [...]` line); this mirrors that so a
/// feature that is already declared but was never added to `default` -- the exact shape a
/// manifest patched by an earlier version of this function is left in -- still gets fixed on the
/// next repair pass, not just a brand-new feature. ~keep
///
/// Returns `Ok(None)` when nothing needs to change (every referenced feature is already declared
/// and enabled by default, or every missing one is absent from `core_declared_features` and
/// therefore must not be invented), so callers can distinguish "checked, no update needed" from
/// "wrote the merge" without a further content diff.
///
/// Parses with `toml_edit::DocumentMut`, not the `toml` crate [`read_declared_cargo_features`]
/// uses: `toml_edit` preserves every byte it does not touch -- comments, key order, blank lines,
/// a hand-added `[package.metadata.*]` table -- so the only lines this can ever change are the
/// new feature rows it inserts and the `default` array entries it appends. A `[features]` table
/// absent from `existing` is created; `toml_edit` appends a new table at the document's end
/// rather than reflowing existing ones, so this never disturbs any other table's position. A
/// `default` array is created the same way if the table has none yet, and an existing one keeps
/// every entry it already has -- only missing names are pushed onto the end.
///
/// This is `alef scaffold`'s answer to the "re-run `alef scaffold`" remedy the compile-out warning
/// (`warn_on_undeclared_binding_cfg_features`) prescribes: the manifest this repairs is user-owned
/// and `write_scaffold_files_report`'s ownership guard rightly refuses to blindly overwrite it, but
/// a purely additive `[features]` change cannot corrupt, reorder, or drop anything else in the
/// file, so it is safe to apply on its own, narrower write path even when the guard would
/// otherwise refuse the whole manifest. ~keep
pub fn merge_missing_cfg_features(
    existing: &str,
    api: &ApiSurface,
    core_crate_name: &str,
    core_declared_features: &BTreeSet<String>,
) -> anyhow::Result<Option<String>> {
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .context("existing manifest is not valid TOML")?;

    let features_table_ref = doc.get("features").and_then(toml_edit::Item::as_table);
    let declared: BTreeSet<String> = features_table_ref
        .map(|table| table.iter().map(|(key, _)| key.to_string()).collect())
        .unwrap_or_default();
    let enabled_by_default = features_reachable_from_default(|name| {
        features_table_ref
            .and_then(|table| table.get(name))
            .and_then(toml_edit::Item::as_array)
            .into_iter()
            .flat_map(toml_edit::Array::iter)
            .filter_map(toml_edit::Value::as_str)
            .filter(|member| !member.contains('/'))
            .map(str::to_owned)
            .collect()
    });

    let referenced: BTreeSet<String> = collect_cfg_features(api)
        .into_iter()
        .filter(|feature| core_declared_features.contains(feature))
        .collect();
    let needs_declaration: BTreeSet<String> = referenced.difference(&declared).cloned().collect();
    let needs_default: BTreeSet<String> = referenced.difference(&enabled_by_default).cloned().collect();

    if needs_declaration.is_empty() && needs_default.is_empty() {
        return Ok(None);
    }

    let features_table = doc
        .entry("features")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("[features] exists in the manifest but is not a table")?;

    for feature in &needs_declaration {
        let mut forwarded = toml_edit::Array::new();
        forwarded.push(format!("{core_crate_name}/{feature}"));
        features_table.insert(feature, toml_edit::Item::Value(toml_edit::Value::Array(forwarded)));
    }

    if !needs_default.is_empty() {
        let default_array = features_table
            .entry("default")
            .or_insert_with(|| toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new())))
            .as_array_mut()
            .context("features.default exists but is not an array")?;
        let already_listed: BTreeSet<String> = default_array
            .iter()
            .filter_map(toml_edit::Value::as_str)
            .map(str::to_owned)
            .collect();
        for feature in &needs_default {
            if !already_listed.contains(feature) {
                default_array.push(feature.clone());
            }
        }
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
/// **enable by default** every feature the generated Rust source for `language` references via a
/// forwarded `#[cfg(feature = "X")]`.
///
/// Declaring `X` in `[features]` is not the same as turning it on: `#[cfg(feature = "X")]` is
/// still false for any binding crate build that doesn't pass `--features X`, and none of the
/// build wrappers alef scaffolds (`mix`, `rake-compiler`, the FFI cdylib's own `cargo build`, ...)
/// do. This checks [`read_default_enabled_cargo_features`] -- the set reachable by walking the
/// manifest's own feature graph from `default` -- rather than merely whether `X` is a key in
/// `[features]` at all, so a feature that is declared but never made reachable from `default`
/// still warns instead of reading as fixed. That is a real state this repository has shipped: a
/// prior `[features]` merge that inserted the forwarding row but not a `default` entry left the
/// manifest declaring `X` while `cargo rustc --print cfg` still omitted it.
///
/// This is the most alef can prove **statically** from the manifest: it cannot see a `--features`
/// flag an external build tool passes at build time, nor a workspace-level feature unification
/// pulling this crate in through another member. A feature this reports missing is provably off
/// by default; a feature this does not report might still be off if something on the build path
/// alef cannot observe fails to pass it.
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
    let Some(enabled_by_default) = read_default_enabled_cargo_features(manifest_path) else {
        return;
    };
    let missing = undeclared_cfg_features(api, &enabled_by_default);
    if missing.is_empty() {
        return;
    }
    tracing::warn!(
        language = %language,
        manifest = %manifest_path.display(),
        missing_features = ?missing,
        "generated bindings reference #[cfg(feature = \"...\")] gates this crate's own Cargo.toml \
         does not enable by default; the affected definitions (and their registrations) will \
         silently compile out of this binding even though the core crate has the feature on -- \
         re-run `alef scaffold` to add the missing features to this crate's [features] table and \
         its default list"
    );
}

/// Warn when a binding language's own cfg filter and the shared FFI cdylib disagree about
/// whether a `#[cfg(feature = "...")]`-gated item exists.
///
/// The FFI cdylib is built once and shared by every language binding (`cargo build -p
/// {ffi_crate}` runs with no `--features` override — see `cli::pipeline::commands::build`), and
/// its `[features] default = [...]` list is populated by [`effective_ffi_default_features`] (see
/// `scaffold::languages::ffi::scaffold_ffi`) — the FFI language config's own feature list
/// UNIONED with every feature name an emitted cfg gate references, not the FFI language config's
/// feature list alone. Each binding meanwhile filters its own surface with
/// [`ApiSurface::with_cfg_filtered_deep`] under its configured feature list. When the two answers
/// for one gate differ, the binding either references a symbol the shipped library does not
/// export or silently omits one it does.
///
/// This evaluates each gate with [`crate::core::ir::cfg_feature_satisfied`] — the very function
/// the filter uses — rather than differencing the two feature-name sets. A name difference is not
/// a gate difference, and treating it as one made this warning fire with false content on the
/// commonest configuration there is: with `features = ["full"]` on both sides, every gate is
/// satisfied on both sides and nothing is omitted, yet the set difference reported every
/// cfg-discovered name as a coverage gap, because `full` is a universal satisfier inside
/// `cfg_feature_satisfied` that no set operation over literal names can model. `any(...)` gates
/// were misreported the same way. A warning whose text is routinely false trains its reader to
/// ignore it, which costs more than having no warning at all. ~keep
///
/// alef cannot observe the real build (it does not run `cargo build` here), so this reports the
/// config-level divergence that would cause the failure, and stays a warning: configuring a
/// narrower surface than the cdylib exports is legitimate, deliberate, and must not fail a build.
///
/// The two directions have different failure modes, so they get different warnings:
/// - the binding keeps a gated item the cdylib was NOT built with: UNSAFE — glue referencing a
///   symbol the shipped library does not export, i.e. a link/runtime failure. Reachable when the
///   gate's feature is declare-only on the FFI side (`[crates.ffi].extra_features`), which
///   [`effective_ffi_default_features`] excludes from the cdylib's defaults on purpose.
/// - the binding DROPS a gated item the cdylib does export: a SAFE coverage gap — the binding's
///   surface is silently smaller than the artifact it links against. This is the case a
///   configured list that satisfies no gate produces, and the one worth naming precisely. ~keep
///
/// Only the FFI side expands aggregate feature names through the core crate's `[features]` table
/// (via [`expand_configured_features`], as `backends::jni`'s target gate does): cargo really does
/// enable an aggregate's members when it compiles the cdylib, so the artifact contains them. The
/// binding side is deliberately NOT expanded — its filter compares literally, so a binding
/// configured with an aggregate really does drop the members' items, and expanding here would
/// hide exactly the underexposure this warning exists to report. ~keep
pub fn warn_on_ffi_feature_drift(api: &ApiSurface, config: &ResolvedCrateConfig, lang: Language) {
    if lang == Language::Ffi {
        return;
    }
    let binding_owned = config.features_for_language(lang);
    let binding_enabled: HashSet<&str> = binding_owned.iter().map(String::as_str).collect();
    let ffi_owned = expand_configured_features(config, &effective_ffi_default_features(api, config));
    let cdylib_enabled: HashSet<&str> = ffi_owned.iter().map(String::as_str).collect();

    let mut unsafe_gates: BTreeSet<String> = BTreeSet::new();
    let mut coverage_gaps: BTreeSet<String> = BTreeSet::new();
    for gate in collect_cfg_gates(api) {
        let kept_by_binding = cfg_feature_satisfied(Some(&gate), &binding_enabled);
        let built_into_cdylib = cfg_feature_satisfied(Some(&gate), &cdylib_enabled);
        match (kept_by_binding, built_into_cdylib) {
            (true, false) => {
                unsafe_gates.insert(gate);
            }
            (false, true) => {
                coverage_gaps.insert(gate);
            }
            _ => {}
        }
    }

    if !unsafe_gates.is_empty() {
        tracing::warn!(
            language = %lang,
            unsatisfied_in_cdylib = ?unsafe_gates,
            cdylib_default_features = ?BTreeSet::from_iter(ffi_owned.iter().map(String::as_str)),
            "this binding's configured feature set satisfies cfg gates the FFI cdylib's effective \
             default feature set does not; with_cfg_filtered_deep keeps glue for those items even \
             though the linked native library was never built with them — this is unsafe and can \
             produce glue that references symbols the shipped library doesn't export"
        );
    }
    if !coverage_gaps.is_empty() {
        let mut missing_features = BTreeSet::new();
        for gate in &coverage_gaps {
            collect_cfg_feature_names(gate, &mut missing_features);
        }
        missing_features.retain(|name| !binding_enabled.contains(name.as_str()));
        tracing::warn!(
            language = %lang,
            unsatisfied_gates = ?coverage_gaps,
            missing_features = ?missing_features,
            configured_features = ?BTreeSet::from_iter(binding_enabled.iter().copied()),
            "the FFI cdylib's effective default feature set satisfies cfg gates this binding's \
             configured feature set does not; with_cfg_filtered_deep safely omits that glue, but \
             the shipped native library does export those items — add the missing features to \
             this binding's configured feature list to close the coverage gap"
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
