//! Best-effort repair of a Rust-emitting binding crate's `[features]` table.
//!
//! `codegen::cfg::warn_on_undeclared_binding_cfg_features` warns when generated source for
//! `Ruby` (Magnus) or `Elixir` (Rustler) forwards a `#[cfg(feature = "X")]` gate the binding
//! crate's own `Cargo.toml` does not declare, and prescribes re-running `alef scaffold`. That
//! manifest is `generated_header: true`, so a fresh scaffold with no prior file on disk already
//! writes the right `[features]` table (both `scaffold_ruby_cargo` and `scaffold_elixir_cargo`
//! derive it from `codegen::cfg::collect_cfg_features`) -- but once the file exists,
//! `write_scaffold_files_report`'s ownership guard only overwrites it wholesale when it can prove
//! alef authored the bytes on disk. A manifest that predates the marker scheme, or one whose
//! marker a formatter/hand-edit moved past the guard's scan window, is refused forever: the
//! prescribed remedy becomes a permanent no-op. This module closes that gap with a narrower,
//! always-safe operation instead of widening the guard: inserting a missing forwarding row is
//! purely additive and cannot corrupt, reorder, or drop anything else already in the file, so it
//! runs on its own write path and does not need the guard's proof of full-file authorship. ~keep

use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::path::PathBuf;

/// One Rust-emitting binding manifest this repair covers, paired with the language it belongs to.
fn managed_manifests(config: &ResolvedCrateConfig) -> Vec<(Language, PathBuf)> {
    vec![
        (Language::Ruby, super::ruby_native_manifest_path(config)),
        (
            Language::Elixir,
            PathBuf::from(super::elixir_native_crate_dir(config)).join("Cargo.toml"),
        ),
    ]
}

/// Add every cfg-forwarded feature the generated source for `languages` references but an
/// existing binding manifest does not yet declare, preserving the rest of each manifest exactly.
///
/// Skips a language absent from `languages` (scaffolding one language must not touch another's
/// manifest) and a manifest that does not exist yet on disk (a fresh `alef scaffold` run creates
/// it correctly the first time, via `scaffold_ruby_cargo`/`scaffold_elixir_cargo`'s own
/// `collect_cfg_features` call -- there is nothing here to repair). Failures are logged and
/// skipped rather than propagated: this is best-effort maintenance on top of an already-completed
/// scaffold run, not a required step it should abort.
///
/// Returns the manifest paths this call actually changed, for the caller's own reporting.
pub(crate) fn repair_missing_cfg_binding_features(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> Vec<PathBuf> {
    let mut repaired = Vec::new();
    for (language, relative_manifest) in managed_manifests(config) {
        if !languages.contains(&language) {
            continue;
        }
        let manifest_path = crate::codegen::cfg::resolve_against_workspace_root(config, &relative_manifest);
        let Ok(existing) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let core_declared_features = crate::codegen::cfg::core_crate_declared_features(config);
        match crate::codegen::cfg::merge_missing_cfg_features(&existing, api, &config.name, &core_declared_features) {
            Ok(Some(patched)) => match std::fs::write(&manifest_path, &patched) {
                Ok(()) => {
                    tracing::info!(
                        language = %language,
                        manifest = %manifest_path.display(),
                        "added missing cfg-forwarded feature(s) to this binding crate's [features] table"
                    );
                    repaired.push(manifest_path);
                }
                Err(error) => tracing::warn!(
                    language = %language,
                    manifest = %manifest_path.display(),
                    %error,
                    "failed to write repaired [features] table"
                ),
            },
            Ok(None) => {}
            Err(error) => tracing::warn!(
                language = %language,
                manifest = %manifest_path.display(),
                %error,
                "failed to repair [features] table"
            ),
        }
    }
    repaired
}
