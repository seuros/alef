use anyhow::{Context as _, Result};
use std::path::PathBuf;

use crate::cli::{cache, dispatch, pipeline, version_pin};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;

/// Surface registry-mode dependency resolution that was deferred to a post-publish pass.
///
/// Deliberately not an error. Registry-mode manifests pin the version the current
/// run produces, so these steps cannot succeed until that version is published --
/// failing here would mean every release run fails on a precondition that is
/// required to be false at that moment. Local-mode e2e, which is what actually
/// gates correctness, still hard-fails on any formatter error. ~keep
fn report_deferred_formatting(crate_name: &str, deferred: &[crate::e2e::format::DeferredFormatting]) {
    if deferred.is_empty() {
        return;
    }
    tracing::warn!(
        "[{crate_name}] {} dependency-resolution step(s) deferred until the pinned version is published:",
        deferred.len()
    );
    for entry in deferred {
        tracing::warn!("  {entry}");
    }
}

fn sync_registry_versions_before_all(
    config_path: &std::path::Path,
    configs: &[&crate::core::config::ResolvedCrateConfig],
) -> Result<bool> {
    let mut versions = std::collections::BTreeSet::new();
    for config in configs {
        let version = config.resolved_version().with_context(|| {
            format!(
                "could not resolve version for crate `{}` from {}",
                config.name, config.version_from
            )
        })?;
        versions.insert(version);
    }
    anyhow::ensure!(
        versions.len() <= 1,
        "alef all cannot synchronize one registry config from multiple crate versions: {}",
        versions.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    let Some(version) = versions.into_iter().next() else {
        return Ok(false);
    };
    pipeline::sync_registry_package_versions(config_path, &version)
}

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::All { clean, skip_frb } => {
            if skip_frb {
                let existing = std::env::var("ALEF_SKIP_COMMANDS").unwrap_or_default();
                let updated = if existing.is_empty() {
                    "flutter_rust_bridge_codegen".to_string()
                } else {
                    format!("{existing},flutter_rust_bridge_codegen")
                };
                // SAFETY: single-threaded CLI dispatch; no concurrent env access here.
                unsafe { std::env::set_var("ALEF_SKIP_COMMANDS", updated) };
            }
            let _ = skip_frb;
            let (mut workspace, mut resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
            let registry_versions_changed = {
                let selected = dispatch::select_crates(&resolved, &context.crate_filter)?;
                sync_registry_versions_before_all(config_path, &selected)?
            };
            if registry_versions_changed {
                (workspace, resolved) = load_config(config_path)?;
                version_pin::check_alef_toml_version(&workspace)?;
            }
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;

            for resolved_cfg in &crates_to_process {
                let Some(e2e_config) = &resolved_cfg.e2e else {
                    continue;
                };
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                if let Some(coverage) = crate::e2e::evaluate_snippet_coverage(
                    resolved_cfg,
                    e2e_config,
                    &api.types,
                    &api.enums,
                    &api.functions,
                )? {
                    crate::e2e::ensure_fresh_snippet_coverage_complete(&coverage)?;
                }
            }

            let config_toml = std::fs::read_to_string(config_path)?;
            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

            let mut grand_binding_count: usize = 0;
            let mut grand_stub_count: usize = 0;
            let mut grand_api_count: usize = 0;
            let mut grand_scaffold_count: usize = 0;
            let mut grand_readme_count: usize = 0;
            let mut grand_e2e_count: usize = 0;
            let mut grand_doc_count: usize = 0;

            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                pipeline::warn_missing_formatters(&languages);
                if multi {
                    tracing::info!(
                        "[{}] Running all for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Running all for: {}", format_languages(&languages));
                }

                let api = pipeline::extract(resolved_cfg, config_path, clean)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

                let mut current_gen_paths = std::collections::HashSet::new();
                let mut changed_languages: std::collections::HashSet<crate::core::config::Language> =
                    std::collections::HashSet::new();
                // Registry-mode dependency resolution that had to wait for a publish.
                // Collected rather than raised so finalisation, the orphan sweep and
                // docs all still run; reported once the pipeline has completed. ~keep
                let mut deferred_formatting: Vec<crate::e2e::format::DeferredFormatting> = Vec::new();

                tracing::info!("Generating bindings...");
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, clean, config_path)?;

                let mut binding_count: usize = 0;
                for (lang, lang_files) in &bindings {
                    let lang_str = lang.to_string();

                    for file in lang_files.iter().filter(|file| file.carries_alef_marker()) {
                        current_gen_paths.insert(base_dir.join(&file.path));
                    }

                    let hashes: Vec<(String, String)> = lang_files
                        .iter()
                        .map(|f| {
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&f.content),
                            )
                        })
                        .collect();

                    let cache_key = format!("{}.{lang_str}", resolved_cfg.name);
                    let stored = cache::read_generation_hashes(&cache_key).unwrap_or_default();
                    let cache_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                    if cache_match && !clean && generated_files_match_disk(lang_files, &base_dir) {
                        tracing::info!("  [{lang_str}] up to date (skipping)");
                        continue;
                    }

                    let single = vec![(*lang, lang_files.clone())];
                    let report = pipeline::write_files_report(&single, &base_dir)?;
                    binding_count += report.changed_count();
                    if report.changed_count() > 0 {
                        changed_languages.insert(*lang);
                    }
                    let _ = cache::write_generation_hashes(&cache_key, &hashes);
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                if !api.services.is_empty() {
                    let svc_files = pipeline::generate_service_api(&api, resolved_cfg, &languages)?;
                    if !svc_files.is_empty() {
                        for (_, files) in &svc_files {
                            for file in files.iter().filter(|file| file.carries_alef_marker()) {
                                current_gen_paths.insert(base_dir.join(&file.path));
                            }
                        }
                        let report = pipeline::write_files_report(&svc_files, &base_dir)?;
                        let svc_count = report.changed_count();
                        tracing::info!("Generated {svc_count} service API files");
                        for (lang, generated) in &svc_files {
                            if generated
                                .iter()
                                .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                            {
                                changed_languages.insert(*lang);
                            }
                        }
                    }
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                tracing::info!("Generating scaffolding...");
                // `alef all` always resolves the crate's full configured language set (there is
                // no `--lang` filter on this command), so the crate-wide scaffold manifest below
                // is always written from a complete file list and never clobbers another
                // language's recorded paths. See `write_scaffold_manifest`'s doc for why a
                // `--lang`-filtered caller must not call it. ~keep
                let previous_scaffold_paths = cache::read_scaffold_manifest(&resolved_cfg.name);
                let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                let scaffold_count = pipeline::write_scaffold_files_with_overwrite(&scaffold_files, &base_dir, clean)?;
                let scaffold_output_paths: Vec<PathBuf> =
                    scaffold_files.iter().map(|file| base_dir.join(&file.path)).collect();
                for file in scaffold_files.iter().filter(|file| file.carries_alef_marker()) {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }
                let scaffold_keep: std::collections::HashSet<PathBuf> = scaffold_output_paths.iter().cloned().collect();
                let scaffold_sweep_roots = pipeline::generate_sweep_roots(&languages, false, resolved_cfg, &base_dir);
                pipeline::sweep_manifest_orphans(&previous_scaffold_paths, &scaffold_keep, &scaffold_sweep_roots)?;
                cache::write_scaffold_manifest(&resolved_cfg.name, &scaffold_output_paths)?;
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                tracing::info!("Running post-build processing...");
                complete_generated_artifacts(&languages, resolved_cfg, &base_dir)?;
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                tracing::info!("Generating type stubs...");
                let stubs = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;

                let stub_hashes: Vec<(String, String)> = stubs
                    .iter()
                    .flat_map(|(_, fs)| {
                        fs.iter().map(|f| {
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&f.content),
                            )
                        })
                    })
                    .collect();
                let stubs_cache_key = format!("{}.stubs", resolved_cfg.name);
                let stored_stubs = cache::read_generation_hashes(&stubs_cache_key).unwrap_or_default();
                let stubs_match =
                    !stub_hashes.is_empty() && stub_hashes.iter().all(|(p, h)| stored_stubs.get(p) == Some(h));

                let stub_count = if !stubs_match || clean {
                    let report = pipeline::write_files_report(&stubs, &base_dir)?;
                    let count = report.changed_count();
                    let _ = cache::write_generation_hashes(&stubs_cache_key, &stub_hashes);
                    for (lang, generated) in &stubs {
                        if generated
                            .iter()
                            .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                        {
                            changed_languages.insert(*lang);
                        }
                    }
                    count
                } else {
                    tracing::info!("  [stubs] up to date (skipping)");
                    0
                };

                for (_, files) in &stubs {
                    for file in files.iter().filter(|file| file.carries_alef_marker()) {
                        current_gen_paths.insert(base_dir.join(&file.path));
                    }
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                let mut api_count = 0;
                if resolved_cfg.generate.public_api {
                    let public_api_files = pipeline::generate_public_api(&api, resolved_cfg, &languages, config_path)?;
                    if !public_api_files.is_empty() {
                        let api_hashes: Vec<(String, String)> = public_api_files
                            .iter()
                            .flat_map(|(_, fs)| {
                                fs.iter().map(|f| {
                                    let normalized = pipeline::normalize_content(&f.path, &f.content);
                                    (
                                        base_dir.join(&f.path).display().to_string(),
                                        cache::hash_content(&normalized),
                                    )
                                })
                            })
                            .collect();
                        let api_cache_key = format!("{}.public_api", resolved_cfg.name);
                        let stored_api = cache::read_generation_hashes(&api_cache_key).unwrap_or_default();
                        let api_match =
                            !api_hashes.is_empty() && api_hashes.iter().all(|(p, h)| stored_api.get(p) == Some(h));

                        for (_, files) in &public_api_files {
                            for file in files.iter().filter(|file| file.carries_alef_marker()) {
                                current_gen_paths.insert(base_dir.join(&file.path));
                            }
                        }

                        if !api_match || clean {
                            let report = pipeline::write_files_report(&public_api_files, &base_dir)?;
                            api_count = report.changed_count();
                            tracing::info!("Generated {api_count} public API files");
                            let _ = cache::write_generation_hashes(&api_cache_key, &api_hashes);
                        } else {
                            tracing::info!("  [public_api] up to date (skipping)");
                        }
                    }
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                if !api.version.is_empty() {
                    let pkg = base_dir.join("Package.swift");
                    if let Ok(content) = std::fs::read_to_string(&pkg) {
                        let updated = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{}", api.version));
                        if updated != content {
                            std::fs::write(&pkg, updated)?;
                        }
                    }
                }

                let mut e2e_count = 0;
                if let Some(e2e_config) = &resolved_cfg.e2e {
                    let all_calls = std::iter::once(("_default", &e2e_config.call))
                        .chain(e2e_config.calls.iter().map(|(k, v)| (k.as_str(), v)));
                    for (call_name, call_config) in all_calls {
                        if call_config.function.is_empty() || call_config.module.is_empty() {
                            continue;
                        }
                        let module_path = call_config.module.replace('-', "_");
                        let function_name = &call_config.function;
                        match crate::extract::validate_call_export(&api, &module_path, function_name) {
                            crate::extract::ExportValidation::Ok => {}
                            crate::extract::ExportValidation::NotFound { function } => {
                                anyhow::bail!(
                                    "e2e call '{call_name}': function '{function}' was not found in the extracted API surface. \
                                 Check that it is declared `pub` and that its source file is listed in \
                                 [[crate.sources]] or [[crate.source_crates]]."
                                );
                            }
                            crate::extract::ExportValidation::WrongPath {
                                function,
                                declared_module,
                                actual_paths,
                            } => {
                                let paths = actual_paths.join(", ");
                                anyhow::bail!(
                                    "e2e call '{call_name}': function '{function}' is not exported at module path \
                                 '{declared_module}' -- the Rust codegen would emit `use {declared_module}::{function};`. \
                                 Actual rust_path(s) found: {paths}. \
                                 Fix: either add `pub use <path>::{function};` at the crate root, \
                                 or update `module` in [e2e.calls.{call_name}] to the correct path."
                                );
                            }
                        }
                    }

                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                    let ir_json = serde_json::to_string(&api)?;
                    let e2e_stage_hash = cache::compute_stage_hash(&ir_json, "e2e", &config_toml, &fixture_hash);
                    if !clean && cache::is_stage_cached(&resolved_cfg.name, "e2e", &e2e_stage_hash) {
                        tracing::info!("  [e2e] up to date (skipping)");
                        let cached_paths = cache::read_stage_paths(&resolved_cfg.name, "e2e");
                        deferred_formatting.extend(crate::e2e::format::run_formatters_for_cached_paths(
                            &cached_paths,
                            &base_dir,
                            e2e_config,
                        )?);
                        for path in cached_paths {
                            current_gen_paths.insert(path);
                        }
                    } else {
                        tracing::info!("Generating e2e test suites...");
                        let previous_paths = cache::read_stage_paths(&resolved_cfg.name, "e2e");
                        let files = crate::e2e::generate_e2e(
                            resolved_cfg,
                            e2e_config,
                            None,
                            &api.types,
                            &api.enums,
                            &api.functions,
                        )?;
                        e2e_count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                        let managed_files: Vec<_> = files
                            .iter()
                            .filter(|file| file.carries_alef_marker())
                            .cloned()
                            .collect();
                        deferred_formatting.extend(crate::e2e::format::run_formatters(&managed_files, e2e_config)?);

                        let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();

                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        let e2e_output_root = base_dir.join(&e2e_config.output);
                        pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &[e2e_output_root])?;

                        cache::write_stage_hash(&resolved_cfg.name, "e2e", &e2e_stage_hash, &output_paths)?;

                        for path in output_paths {
                            current_gen_paths.insert(path);
                        }
                    }
                    pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                    let test_apps_stage_hash =
                        cache::compute_stage_hash(&ir_json, "test-apps", &config_toml, &fixture_hash);
                    if !clean && cache::is_stage_cached(&resolved_cfg.name, "test-apps", &test_apps_stage_hash) {
                        tracing::info!("  [test-apps] up to date (skipping)");
                        let cached_paths = cache::read_stage_paths(&resolved_cfg.name, "test-apps");
                        let mut registry_e2e_config = e2e_config.clone();
                        registry_e2e_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                        deferred_formatting.extend(crate::e2e::format::run_formatters_for_cached_paths(
                            &cached_paths,
                            &base_dir,
                            &registry_e2e_config,
                        )?);
                        for path in cached_paths {
                            current_gen_paths.insert(path);
                        }
                    } else {
                        tracing::info!("Generating registry-mode test apps...");
                        let previous_paths = cache::read_stage_paths(&resolved_cfg.name, "test-apps");
                        let mut registry_e2e_config = e2e_config.clone();
                        registry_e2e_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                        let registry_e2e_ref = &registry_e2e_config;

                        let files = crate::e2e::generate_e2e(
                            resolved_cfg,
                            registry_e2e_ref,
                            None,
                            &api.types,
                            &api.enums,
                            &api.functions,
                        )?;
                        let test_apps_count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                        e2e_count += test_apps_count;
                        let managed_files: Vec<_> = files
                            .iter()
                            .filter(|file| file.carries_alef_marker())
                            .cloned()
                            .collect();
                        deferred_formatting
                            .extend(crate::e2e::format::run_formatters(&managed_files, registry_e2e_ref)?);

                        let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();

                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        let test_apps_root = base_dir.join(registry_e2e_ref.effective_output());
                        pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &[test_apps_root])?;

                        cache::write_stage_hash(&resolved_cfg.name, "test-apps", &test_apps_stage_hash, &output_paths)?;

                        for path in output_paths {
                            current_gen_paths.insert(path);
                        }
                    }
                    pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;
                }

                tracing::info!("Generating READMEs...");
                let readme_languages = crate::readme::expand_configured_readme_languages(resolved_cfg, &languages);
                let readme_files = pipeline::readme(&api, resolved_cfg, &readme_languages)?;
                let readme_count = pipeline::write_scaffold_files_with_overwrite(&readme_files, &base_dir, true)?;
                for file in readme_files.iter().filter(|file| file.carries_alef_marker()) {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                tracing::info!("Generating docs...");
                let docs_api = pipeline::extract(resolved_cfg, config_path, false)?;
                let doc_languages = resolve_doc_languages(resolved_cfg, None)?;
                let doc_files =
                    crate::docs::generate_docs_stage(&docs_api, resolved_cfg, &doc_languages, None, &base_dir)?;
                let doc_count = pipeline::write_scaffold_files_with_overwrite(&doc_files, &base_dir, clean)?;
                for file in doc_files.iter().filter(|file| file.carries_alef_marker()) {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                let cleanup_roots = pipeline::generate_sweep_roots(&languages, false, resolved_cfg, &base_dir);
                let previous_paths: Vec<_> = languages
                    .iter()
                    .flat_map(|language| cache::read_lang_manifest(&resolved_cfg.name, &language.to_string()))
                    .collect();
                pipeline::sweep_manifest_orphans(&previous_paths, &current_gen_paths, &cleanup_roots)?;

                if !changed_languages.is_empty() {
                    tracing::info!("Formatting generated files...");
                    let mut files_to_format = bindings.clone();
                    files_to_format.extend(stubs.clone());
                    // `None` selects the converging whole-tree pass, which is what a full regen needs
                    // and what `converge_full_regen_formatting` documents itself as serving. Passing
                    // `Some(&changed_languages)` took the single-pass branch instead, so the loop that
                    // exists precisely because poly's .cs/.java/.json engines are not single-pass
                    // idempotent never ran on the one command that regenerates everything: `alef all`
                    // left drift that a second `alef all` would silently settle, and stamped hashes
                    // over it. The language filter is wrong for the workspace-wide `cargo sort -n -w`
                    // folded into that loop too, which must cover crates this run did not generate. ~keep
                    pipeline::format_generated(&files_to_format, resolved_cfg, &base_dir, None);
                }

                tracing::info!("Finalising hashes...");
                // Sweeping (not the plain path-tracked `finalize_hashes` used by the
                // earlier per-stage checkpoints above) so that a language dropped from
                // `bindings` by the per-language cache in `pipeline::generate` -- and
                // therefore never added to `current_gen_paths` -- still gets its
                // on-disk output re-stamped from `cleanup_roots`. Safe to run after
                // `sweep_manifest_orphans` above: it clones `current_gen_paths` rather
                // than mutating it, so the orphan sweep already saw the untouched,
                // precisely-tracked set.
                pipeline::finalize_hashes_sweeping(
                    &current_gen_paths,
                    &cleanup_roots,
                    &sources_hash,
                    &alef_toml_bytes,
                )?;

                // Reported only now, after finalisation, the orphan sweep and docs have
                // all run. Raising at the point of failure is what made the release
                // unreachable: these steps resolve the very version the run produces. ~keep
                report_deferred_formatting(&resolved_cfg.name, &deferred_formatting);

                grand_binding_count += binding_count;
                grand_stub_count += stub_count;
                grand_api_count += api_count;
                grand_scaffold_count += scaffold_count;
                grand_readme_count += readme_count;
                grand_e2e_count += e2e_count;
                grand_doc_count += doc_count;
            }

            pipeline::install_poly_hooks(&base_dir);

            tracing::info!(
                "Done: {grand_binding_count} binding files, {grand_stub_count} stub files, {grand_api_count} API files, {grand_scaffold_count} scaffold files, {grand_readme_count} readme files, {grand_e2e_count} e2e files, {grand_doc_count} doc files"
            );
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}

#[cfg(test)]
#[path = "all_commands_tests.rs"]
mod tests;
