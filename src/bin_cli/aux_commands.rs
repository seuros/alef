use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process;

use crate::cli::pipeline::run_optional;
use crate::cli::{cache, commands, dispatch, pipeline};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::Init { lang } => {
            tracing::info!("Initializing alef project");
            if let Some(langs) = &lang {
                tracing::info!("  Languages: {}", langs.join(", "));
            }
            pipeline::init(config_path, lang.clone())?;
            tracing::info!("  Created alef.toml");

            let (_workspace, resolved) = load_config(config_path)?;
            let resolved_cfg = &resolved[0];
            let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
            let base_dir = std::env::current_dir()?;

            let api = pipeline::extract(resolved_cfg, config_path, false)?;
            let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

            tracing::info!("  Generating bindings...");
            let bindings = pipeline::generate(&api, resolved_cfg, &languages, false, config_path, true)?;
            let mut binding_count: usize = 0;
            let mut all_paths = std::collections::HashSet::new();
            for (lang_key, lang_files) in &bindings {
                for file in lang_files.iter().filter(|file| file.carries_alef_marker()) {
                    all_paths.insert(base_dir.join(&file.path));
                }
                let single = vec![(*lang_key, lang_files.clone())];
                binding_count += pipeline::write_files(&single, &base_dir)?;
            }
            if languages.contains(&crate::core::config::Language::Ffi) {
                pipeline::check_ffi_header_freshness(resolved_cfg, &base_dir)?;
            }

            tracing::info!("  Generating scaffolding...");
            let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
            let scaffold_count = pipeline::write_scaffold_files(&scaffold_files, &base_dir)?;
            for file in scaffold_files.iter().filter(|file| file.carries_alef_marker()) {
                all_paths.insert(base_dir.join(&file.path));
            }

            tracing::info!("  Formatting...");
            let managed_bindings: Vec<_> = bindings
                .iter()
                .map(|(language, files)| (*language, pipeline::managed_generated_files(files)))
                .collect();
            pipeline::format_generated(&managed_bindings, resolved_cfg, &base_dir, None);

            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
            pipeline::finalize_hashes(&all_paths, &sources_hash, &alef_toml_bytes)?;

            pipeline::install_poly_hooks(&base_dir);

            tracing::info!("Initialized: {binding_count} binding files, {scaffold_count} scaffold files");
            Ok(None)
        }
        Commands::Schema {
            output,
            schema_version,
            check,
        } => {
            let version = schema_version.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"));
            if check {
                crate::core::config::check_alef_config_schema(&output, version)?;
                tracing::info!("Schema is up to date: {}", output.display());
            } else {
                crate::core::config::write_alef_config_schema(&output, version)?;
                tracing::info!("Wrote schema to {}", output.display());
            }
            Ok(None)
        }
        Commands::Adopt { target, write } => {
            let base_dir = std::env::current_dir()?;
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;

            // The diff a human consents to has to be against the bytes a real generate
            // would write, so the same extract/generate/stubs/scaffold sweep `alef diff`
            // performs is what feeds it -- not a cheaper approximation. ~keep
            let mut managed = Vec::new();
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, true, config_path, true)?;
                let stubs = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
                let scaffold = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                for (_language, files) in bindings.iter().chain(stubs.iter()) {
                    managed.extend(commands::adopt::managed_outputs(files, &base_dir));
                }
                managed.extend(commands::adopt::managed_outputs(&scaffold, &base_dir));
            }

            let options = commands::adopt::AdoptOptions {
                target,
                base_dir,
                write,
            };
            let report = commands::adopt::run(&options, &managed)?;

            for diff in &report.diffs {
                crate::bin_cli::output::fragment(&diff.body);
                crate::bin_cli::output::blank();
            }
            for path in &report.already_owned {
                tracing::info!("already alef-owned, nothing to adopt: {}", path.display());
            }
            if report.preview {
                crate::bin_cli::output::line(
                    "Nothing was written. Re-run with --write to stamp these files so alef can regenerate them.",
                );
            } else {
                tracing::info!("Adopted {} file(s)", report.adopted.len());
            }
            Ok(None)
        }
        Commands::Migrate { path, write } => {
            let migrate_path = path.unwrap_or_else(|| context.config_path.clone());
            let options = commands::migrate::MigrateOptions {
                path: migrate_path,
                write,
            };
            commands::migrate::run(options)?;
            Ok(None)
        }
        Commands::E2e { action } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let resolved_cfg = crates_to_process
                .iter()
                .find(|c| c.e2e.is_some())
                .copied()
                .unwrap_or_else(|| crates_to_process[0]);
            let e2e_config = resolved_cfg.e2e.as_ref().context("no [e2e] section in alef.toml")?;
            match action {
                E2eAction::Generate { lang, registry, strict } => {
                    if registry {
                        tracing::warn!(
                            "`alef e2e generate --registry` is deprecated. \
                             Use `alef test-apps generate` instead. \
                             `alef e2e generate` is local-mode only."
                        );
                    }
                    let config_toml = std::fs::read_to_string(config_path)?;
                    let base_dir = std::env::current_dir()?;
                    let mut grand_count: usize = 0;
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };
                        let fixtures_dir = std::path::Path::new(&this_e2e_config.fixtures);
                        let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                        let api = pipeline::extract(e2e_crate, config_path, false)?;
                        let ir_json = serde_json::to_string(&api)?;
                        let cache_key = if registry { "e2e-registry" } else { "e2e" };
                        let effective_e2e_config;
                        let e2e_ref = if registry {
                            let mut cloned = this_e2e_config.clone();
                            cloned.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                            effective_e2e_config = cloned;
                            &effective_e2e_config
                        } else {
                            this_e2e_config
                        };
                        let stage_hash = cache::compute_stage_hash(&ir_json, cache_key, &config_toml, &fixture_hash);
                        if cache::is_stage_cached(&e2e_crate.name, cache_key, &stage_hash) {
                            let cached_paths = cache::read_stage_paths(&e2e_crate.name, cache_key);
                            grand_count += cached_paths.len();
                            crate::e2e::format::warn_deferred(&crate::e2e::format::run_formatters_for_cached_paths(
                                &cached_paths,
                                &base_dir,
                                e2e_ref,
                                strict,
                            )?);
                            if let Some(snippets) = &this_e2e_config.snippets {
                                let coverage_path = base_dir
                                    .join(&snippets.output)
                                    .join(crate::e2e::snippets::COVERAGE_MANIFEST);
                                crate::e2e::report_cached_snippet_coverage(&coverage_path)?;
                            }
                            tracing::info!("E2E tests up to date (cached)");
                            continue;
                        }
                        if registry {
                            tracing::info!("Generating e2e test apps (registry mode)...");
                        } else {
                            tracing::info!("Generating e2e test suites...");
                        }
                        let languages = lang.as_deref();
                        let files = crate::e2e::generate_e2e(
                            e2e_crate,
                            e2e_ref,
                            languages,
                            &api.types,
                            &api.enums,
                            &api.functions,
                        )?;
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        let report = pipeline::write_scaffold_files_report(&files, &base_dir, true)?;
                        let count = report.expected_count();
                        let managed_files = pipeline::managed_generated_files(&files);

                        if managed_files
                            .iter()
                            .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                        {
                            crate::e2e::format::warn_deferred(&crate::e2e::format::run_formatters(
                                &managed_files,
                                e2e_ref,
                                strict,
                            )?);
                        }

                        let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        let e2e_output_root = base_dir.join(e2e_ref.effective_output());
                        let sweep_roots: Vec<PathBuf> = if lang.is_some() {
                            let snippet_output_root = e2e_ref
                                .snippets
                                .as_ref()
                                .map(|snippets| base_dir.join(&snippets.output));
                            pipeline::targeted_e2e_sweep_roots(
                                &output_paths,
                                &e2e_output_root,
                                snippet_output_root.as_deref(),
                            )
                        } else {
                            vec![e2e_output_root]
                        };
                        let previous_paths = cache::read_stage_paths(&e2e_crate.name, cache_key);
                        pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &sweep_roots)?;

                        cache::write_stage_hash(&e2e_crate.name, cache_key, &stage_hash, &output_paths)?;
                        grand_count += count;
                    }
                    tracing::info!("Generated {grand_count} e2e files");
                    Ok(None)
                }
                E2eAction::SnippetsMigrate {
                    existing_root,
                    lang,
                    json,
                } => {
                    let snippet_config = e2e_config
                        .snippets
                        .as_ref()
                        .context("no [e2e.snippets] section in alef.toml")?;
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixtures = crate::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let api = pipeline::extract(resolved_cfg, config_path, false)?;
                    let fallback_languages = if e2e_config.languages.is_empty() {
                        crate::e2e::default_e2e_languages(&resolved_cfg.languages)
                    } else {
                        e2e_config.languages.clone()
                    };
                    let languages = lang
                        .as_deref()
                        .unwrap_or_else(|| snippet_config.languages_or(&fallback_languages));
                    let generated = crate::e2e::snippets::generate_snippets(
                        &fixtures,
                        languages,
                        e2e_config,
                        snippet_config,
                        resolved_cfg,
                        &api.types,
                        &api.enums,
                        &api.functions,
                    )?;
                    let entries = crate::e2e::snippets::migration::compare_root(
                        &existing_root,
                        std::path::Path::new(&snippet_config.output),
                        &generated,
                    )?;
                    write_snippet_migration_report(&entries, json)?;
                    Ok(None)
                }
                E2eAction::Init => {
                    tracing::info!("Initializing e2e fixtures directory...");
                    let created = crate::e2e::scaffold::init_fixtures(e2e_config, resolved_cfg)?;
                    for path in &created {
                        tracing::info!("  created {path}");
                    }
                    tracing::info!("Initialized {} file(s)", created.len());
                    Ok(None)
                }
                E2eAction::Scaffold {
                    id,
                    category,
                    description,
                } => {
                    let path =
                        crate::e2e::scaffold::scaffold_fixture(e2e_config, resolved_cfg, &id, &category, &description)?;
                    tracing::info!("Created {path}");
                    Ok(None)
                }
                E2eAction::List => {
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixtures = crate::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let groups = crate::e2e::fixture::group_fixtures(&fixtures);

                    crate::bin_cli::output::line(format_args!("Fixtures: {} total", fixtures.len()));
                    for group in &groups {
                        crate::bin_cli::output::line(format_args!(
                            "  {}: {} fixture(s)",
                            group.category,
                            group.fixtures.len()
                        ));
                    }
                    Ok(None)
                }
                E2eAction::Validate => {
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    tracing::info!("Validating fixtures in {}...", fixtures_dir.display());

                    let mut all_errors = crate::e2e::validate::validate_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to validate fixtures from {}", fixtures_dir.display()))?;

                    let fixtures = crate::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let semantic_errors =
                        crate::e2e::validate::validate_fixtures_semantic(&fixtures, e2e_config, &e2e_config.languages);
                    all_errors.extend(semantic_errors);

                    if all_errors.is_empty() {
                        crate::bin_cli::output::line("All fixtures are valid.");
                        Ok(None)
                    } else {
                        use crate::e2e::validate::Severity;
                        let error_count = all_errors.iter().filter(|e| e.severity == Severity::Error).count();
                        let warning_count = all_errors.iter().filter(|e| e.severity == Severity::Warning).count();
                        crate::bin_cli::output::line(format_args!(
                            "Found {} error(s) and {} warning(s):",
                            error_count, warning_count
                        ));
                        for err in &all_errors {
                            crate::bin_cli::output::line(format_args!("  {err}"));
                        }
                        if error_count > 0 {
                            process::exit(1);
                        }
                        Ok(None)
                    }
                }
            }
        }
        Commands::TestApps { action } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let _resolved_cfg = crates_to_process
                .iter()
                .find(|c| c.e2e.is_some())
                .copied()
                .unwrap_or_else(|| crates_to_process[0]);
            let _ = _resolved_cfg.e2e.as_ref().context("no [e2e] section in alef.toml")?;
            match action {
                TestAppsAction::Generate {
                    lang,
                    clean,
                    jobs: _,
                    strict,
                } => {
                    let config_toml = std::fs::read_to_string(config_path)?;
                    let base_dir = std::env::current_dir()?;
                    let mut grand_count: usize = 0;
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };

                        let mut registry_config = this_e2e_config.clone();
                        registry_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                        let e2e_ref = &registry_config;
                        let output_root = base_dir.join(e2e_ref.effective_output());

                        let fixtures_dir = std::path::Path::new(&this_e2e_config.fixtures);
                        let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                        let api = pipeline::extract(e2e_crate, config_path, false)?;
                        let ir_json = serde_json::to_string(&api)?;
                        let selector = lang
                            .as_deref()
                            .map(|languages| languages.join("-"))
                            .unwrap_or_else(|| "all".to_string());
                        let cache_key = format!("test-apps-{selector}");
                        let previous_paths = cache::read_stage_paths(&e2e_crate.name, &cache_key);
                        let stage_hash = cache::compute_stage_hash(&ir_json, &cache_key, &config_toml, &fixture_hash);
                        if !clean && cache::is_stage_cached(&e2e_crate.name, &cache_key, &stage_hash) {
                            let cached_paths = cache::read_stage_paths(&e2e_crate.name, &cache_key);
                            crate::e2e::format::warn_deferred(&crate::e2e::format::run_formatters_for_cached_paths(
                                &cached_paths,
                                &base_dir,
                                e2e_ref,
                                strict,
                            )?);
                            tracing::info!("Test apps up to date (cached)");
                            continue;
                        }

                        tracing::info!("Generating registry-mode test apps...");
                        let languages = lang.as_deref();
                        let files = crate::e2e::generate_e2e(
                            e2e_crate,
                            e2e_ref,
                            languages,
                            &api.types,
                            &api.enums,
                            &api.functions,
                        )?;
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        let report = pipeline::write_scaffold_files_report(&files, &base_dir, true)?;
                        let count = report.changed_count();
                        let managed_files: Vec<_> = files
                            .iter()
                            .filter(|file| file.carries_alef_marker())
                            .cloned()
                            .collect();

                        let generated_langs: Vec<String> = languages
                            .map(|ls| ls.iter().map(|s| s.to_string()).collect())
                            .unwrap_or_else(|| e2e_ref.languages.clone());
                        for lang_name in &generated_langs {
                            let lock_missing = matches!(lang_name.as_str(), "node" | "wasm")
                                && !output_root.join(lang_name).join("pnpm-lock.yaml").exists();
                            if !lock_missing
                                && !report
                                    .changed_paths
                                    .iter()
                                    .any(|path| path.starts_with(output_root.join(lang_name)))
                            {
                                continue;
                            }
                            if lang_name == "node" || lang_name == "wasm" {
                                let test_app_dir = output_root.join(lang_name);
                                let package_json = test_app_dir.join("package.json");
                                if package_json.exists() {
                                    tracing::info!("Regenerating {}/pnpm-lock.yaml...", lang_name);
                                    run_optional(
                                        "pnpm",
                                        &[
                                            "install",
                                            "--lockfile-only",
                                            "-C",
                                            test_app_dir.to_string_lossy().as_ref(),
                                        ],
                                    );
                                }
                            } else if lang_name == "php" {
                                let test_app_dir = output_root.join(lang_name);
                                let composer_json = test_app_dir.join("composer.json");
                                if composer_json.exists() {
                                    tracing::info!("Regenerating {}/composer.lock...", lang_name);
                                    run_optional(
                                        "composer",
                                        &[
                                            "update",
                                            "--lock",
                                            "--no-install",
                                            "--working-dir",
                                            test_app_dir.to_string_lossy().as_ref(),
                                        ],
                                    );
                                }
                            }
                        }

                        if managed_files
                            .iter()
                            .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                        {
                            crate::e2e::format::warn_deferred(&crate::e2e::format::run_formatters(
                                &managed_files,
                                e2e_ref,
                                strict,
                            )?);
                        }

                        let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        let sweep_roots: Vec<PathBuf> = if lang.is_some() {
                            let snippet_output_root = e2e_ref
                                .snippets
                                .as_ref()
                                .map(|snippets| base_dir.join(&snippets.output));
                            pipeline::targeted_e2e_sweep_roots(
                                &output_paths,
                                &output_root,
                                snippet_output_root.as_deref(),
                            )
                        } else {
                            vec![output_root]
                        };
                        pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &sweep_roots)?;

                        cache::write_stage_hash(&e2e_crate.name, &cache_key, &stage_hash, &output_paths)?;
                        grand_count += count;
                    }
                    tracing::info!("Generated {grand_count} test-app files");
                    Ok(None)
                }
                TestAppsAction::Run { lang } => {
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };
                        let all_names: Vec<String> = if this_e2e_config.languages.is_empty() {
                            crate::e2e::default_e2e_languages(&e2e_crate.languages)
                        } else {
                            this_e2e_config.languages.clone()
                        };
                        let names: Vec<String> = match lang.as_deref() {
                            Some(filter) => all_names
                                .into_iter()
                                .filter(|n| filter.iter().any(|f| f == n))
                                .collect(),
                            None => all_names,
                        };
                        if names.is_empty() {
                            continue;
                        }
                        tracing::info!("Running test apps for: {}", names.join(", "));
                        pipeline::test_apps_run(e2e_crate, &names)?;
                    }
                    Ok(None)
                }
            }
        }
        other => Ok(Some(other)),
    }
}

fn write_snippet_migration_report(
    entries: &[crate::e2e::snippets::migration::MigrationEntry],
    json: bool,
) -> Result<()> {
    if json {
        crate::bin_cli::output::payload(serde_json::to_string_pretty(entries)?);
        return Ok(());
    }
    for entry in entries {
        use crate::e2e::snippets::migration::MigrationStatus;
        let status = match entry.status {
            MigrationStatus::Identical => "identical",
            MigrationStatus::Different => "different",
            MigrationStatus::NoGeneratedEquivalent => "no_generated_equivalent",
        };
        crate::bin_cli::output::line(format_args!("{status}\t{}", entry.path.display()));
    }
    Ok(())
}
