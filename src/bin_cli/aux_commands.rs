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
        Commands::Adopt {
            targets,
            write,
            converged_only,
            clobber_create_once_seeds,
        } => {
            let base_dir = std::env::current_dir()?;
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;

            // The diff a human consents to has to be against the bytes a real generate
            // would write, so the full managed surface -- every stage `alef all` writes,
            // not a hand-maintained subset of it -- is what feeds it. Shared verbatim
            // with `alef verify`'s frozen-file report so the report and the remedy for
            // the same fact cannot disagree; see `collect_managed_surface`. A stage
            // failure it tolerated is only ours to ignore when none of the requested
            // `targets` could have come from that stage -- otherwise this run cannot
            // answer the ownership question the operator actually asked, and must say
            // so rather than adopt against possibly-stale bytes. ~keep
            let mut managed = Vec::new();
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let (surface, stage_failures) =
                    collect_managed_surface(&languages, &api, resolved_cfg, config_path, &base_dir)?;
                for failure in &stage_failures {
                    if failure.affects_any(&targets) {
                        anyhow::bail!(
                            "[{}] {} -- this affects one of the requested targets, so `alef adopt` \
                             cannot answer for it",
                            failure.stage,
                            failure.message
                        );
                    }
                    tracing::debug!(
                        stage = failure.stage,
                        "tolerating stage failure: no requested target comes from this stage: {}",
                        failure.message
                    );
                }
                managed.extend(commands::adopt::managed_outputs(&surface, &base_dir));
            }

            // One target's refusal must not silently cancel the other fifty-three. `run` bails
            // whenever a target resolves to nothing adoptable -- no match, or (far more commonly on
            // a repo-wide sweep) only create-once seeds -- and propagating that straight out of the
            // loop meant a single `config.m4` early in a sorted list of 54 refused paths ended the
            // command before one file was stamped, with an exit code that named only that path. Each
            // target is now reported independently and the run fails at the end iff any did. ~keep
            let mut target_failures: Vec<(String, anyhow::Error)> = Vec::new();
            for target in &targets {
                let options = commands::adopt::AdoptOptions {
                    target: target.clone(),
                    base_dir: base_dir.clone(),
                    write,
                    converged_only,
                    clobber_create_once_seeds,
                };
                let report = match commands::adopt::run(&options, &managed) {
                    Ok(report) => report,
                    Err(error) => {
                        target_failures.push((target.clone(), error));
                        continue;
                    }
                };

                if !report.unreadable.is_empty() {
                    // Named, not counted, and on stdout beside the diffs: this list is the whole
                    // result for these paths, and it reports alef being unable to say anything --
                    // not the operator having a decision left to make. A binary alef *does* emit
                    // (a `.jar`) is no longer here: it is diffed by size and digest and adopted
                    // through the ownership record like any other unstampable format. ~keep
                    crate::bin_cli::output::blank();
                    crate::bin_cli::output::line(
                        "NOT ADOPTED -- alef could not read these matches. Their bytes are neither valid \
                         UTF-8 nor one of alef's own base64-encoded binary outputs, so alef can state \
                         nothing about them and leaves them alone:",
                    );
                    for path in &report.unreadable {
                        crate::bin_cli::output::line(format_args!("  {}", path.display()));
                    }
                    crate::bin_cli::output::blank();
                }

                if !report.skipped_create_once.is_empty() {
                    // Every path, never a count, and on stdout with the drifted diffs rather
                    // than through `tracing`: this list is the command's result for these
                    // paths, and `-q` must not be able to hide the one output that says work
                    // is about to be destroyed. The consequence is spelled out because
                    // "skipped" alone reads as "nothing happened", when the fact the operator
                    // needs is what adopting them *would* have cost. ~keep
                    crate::bin_cli::output::blank();
                    crate::bin_cli::output::line(
                        "NOT ADOPTED -- create-once seeds. alef emits each of these only when the file is \
                         absent, so it is a placeholder that the copy on disk has almost certainly grown \
                         past. Adopting one consents to alef REPLACING its contents with that placeholder \
                         on the next generate:",
                    );
                    for path in &report.skipped_create_once {
                        crate::bin_cli::output::line(format_args!("  {}", path.display()));
                    }
                    crate::bin_cli::output::line(
                        "Read each one and confirm it holds nothing you wrote, then re-run with \
                         --clobber-create-once-seeds to adopt them anyway.",
                    );
                    crate::bin_cli::output::blank();
                }

                for diff in &report.diffs {
                    crate::bin_cli::output::fragment(&diff.body);
                    crate::bin_cli::output::blank();
                }
                if !report.converged.is_empty() {
                    // Summarised, never diffed. See `cli::commands::adopt`'s header: a
                    // converged file's diff is the file itself echoed back, and printing
                    // 12,000 of them buries the drifted diffs printed just above -- the
                    // only ones with content to read. ~keep
                    crate::bin_cli::output::line(format_args!(
                        "{} file(s) already match generated output byte-for-byte apart from the marker; \
                         adopting them changes no content.",
                        report.converged.len()
                    ));
                }
                for path in &report.already_owned {
                    tracing::info!("already alef-owned, nothing to adopt: {}", path.display());
                }
                if report.preview {
                    if !report.diffs.is_empty() && !report.converged.is_empty() {
                        crate::bin_cli::output::line(format_args!(
                            "Re-run with --converged-only --write to adopt the {} converged file(s) alone, \
                             then review the {} drifted diff(s) above before adopting those.",
                            report.converged.len(),
                            report.diffs.len()
                        ));
                    }
                    crate::bin_cli::output::line(
                        "Nothing was written. Re-run with --write to stamp these files so alef can regenerate them.",
                    );
                } else {
                    tracing::info!("Adopted {} file(s)", report.adopted.len());
                    if !report.skipped_drifted.is_empty() {
                        crate::bin_cli::output::line(format_args!(
                            "Left {} drifted file(s) untouched (--converged-only). Their diffs are above; \
                             adopt them with an explicit target once you have read each one.",
                            report.skipped_drifted.len()
                        ));
                    }
                    if !report.recorded_unstampable.is_empty() {
                        // These adoptions changed no bytes in the files themselves — the whole
                        // consent lives in `.alef-ownership.toml`. Leave it uncommitted and the
                        // human read the diff for nothing: every other checkout, CI included,
                        // still refuses these paths. Naming the file is the difference between
                        // an adoption and an adoption that took effect. ~keep
                        tracing::info!(
                            "{} of these carry no marker syntax; their ownership is recorded in \
                             .alef-ownership.toml. Commit that file or the adoption applies only to \
                             this working copy.",
                            report.recorded_unstampable.len()
                        );
                    }
                }
            }
            if !target_failures.is_empty() {
                for (target, error) in &target_failures {
                    tracing::error!("{target}: {error:#}");
                }
                anyhow::bail!(
                    "{} of {} adopt target(s) could not be adopted; each is reported above",
                    target_failures.len(),
                    targets.len()
                );
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
                E2eAction::Generate {
                    lang,
                    registry,
                    strict,
                    no_strict_assertions,
                } => {
                    if registry {
                        tracing::warn!(
                            "`alef e2e generate --registry` is deprecated. \
                             Use `alef test-apps generate` instead. \
                             `alef e2e generate` is local-mode only."
                        );
                    }
                    if no_strict_assertions {
                        // SAFETY: single-threaded CLI dispatch; no concurrent env access here.
                        unsafe { std::env::set_var(crate::e2e::codegen::STRICT_ASSERTIONS_ENV, "0") };
                    }
                    let config_toml = std::fs::read_to_string(config_path)?;
                    let base_dir = std::env::current_dir()?;
                    let mut grand_count: usize = 0;
                    // Deferred the same way `all_commands::handle` defers it, and for the same
                    // reason: `sweep_manifest_orphans` and `cache::write_stage_hash` right after the
                    // write below are unsafe to run on a generator failure (stale-cache and
                    // last-good-output-deletion hazards). Both are gated on this being `None`. ~keep
                    let mut e2e_stage_error: Option<anyhow::Error> = None;
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };
                        let fixtures_dir = std::path::Path::new(&this_e2e_config.fixtures);
                        let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                        let api = pipeline::extract(e2e_crate, config_path, false)?;
                        let ir_json = serde_json::to_string(&api)?;
                        let cache_key = e2e_stage_cache_key(registry, lang.as_deref());
                        let effective_e2e_config;
                        let e2e_ref = if registry {
                            let mut cloned = this_e2e_config.clone();
                            cloned.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                            effective_e2e_config = cloned;
                            &effective_e2e_config
                        } else {
                            this_e2e_config
                        };
                        let stage_hash = cache::compute_stage_hash(&ir_json, &cache_key, &config_toml, &fixture_hash);
                        if cache::is_stage_cached(&e2e_crate.name, &cache_key, &stage_hash) {
                            let cached_paths = cache::read_stage_paths(&e2e_crate.name, &cache_key);
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
                        let (files, generator_error) = crate::e2e::generate_e2e(
                            e2e_crate,
                            e2e_ref,
                            languages,
                            &api.types,
                            &api.enums,
                            &api.functions,
                            &api.errors,
                        )?;
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        let report = pipeline::write_scaffold_files_report(&files, &base_dir, true)?;
                        pipeline::report_refused_writes(&report);
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
                        if let Some(error) = generator_error {
                            if e2e_stage_error.is_some() {
                                tracing::error!("[{}] e2e codegen failed: {error:#}", e2e_crate.name);
                            }
                            e2e_stage_error.get_or_insert(error);
                        } else {
                            let previous_paths = cache::read_stage_paths(&e2e_crate.name, &cache_key);
                            pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &sweep_roots, &[])?;

                            cache::write_stage_hash(&e2e_crate.name, &cache_key, &stage_hash, &output_paths)?;
                        }
                        grand_count += count;
                    }
                    tracing::info!("Generated {grand_count} e2e files");
                    if let Some(error) = e2e_stage_error {
                        return Err(error);
                    }
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
                    // Deferred the same way `all_commands::handle` defers it -- see the `e2e`
                    // command's `e2e_stage_error` above for the cache-poisoning and
                    // orphan-deletion hazard this gates against. ~keep
                    let mut e2e_stage_error: Option<anyhow::Error> = None;
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
                        let (files, generator_error) = crate::e2e::generate_e2e(
                            e2e_crate,
                            e2e_ref,
                            languages,
                            &api.types,
                            &api.enums,
                            &api.functions,
                            &api.errors,
                        )?;
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        let report = pipeline::write_scaffold_files_report(&files, &base_dir, true)?;
                        pipeline::report_refused_writes(&report);
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
                        if let Some(error) = generator_error {
                            if e2e_stage_error.is_some() {
                                tracing::error!("[{}] test-apps codegen failed: {error:#}", e2e_crate.name);
                            }
                            e2e_stage_error.get_or_insert(error);
                        } else {
                            pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &sweep_roots, &[])?;

                            cache::write_stage_hash(&e2e_crate.name, &cache_key, &stage_hash, &output_paths)?;
                        }
                        grand_count += count;
                    }
                    tracing::info!("Generated {grand_count} test-app files");
                    if let Some(error) = e2e_stage_error {
                        return Err(error);
                    }
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
                        pipeline::test_apps_run(e2e_crate, config_path, &names)?;
                    }
                    Ok(None)
                }
            }
        }
        other => Ok(Some(other)),
    }
}

/// Stage-cache key for `alef e2e generate`, fed to `cache::compute_stage_hash` as the stage name
/// and so encoded into the stage hash as well as the manifest filename.
///
/// The `--lang` selection is part of the key because a scoped run only writes the languages it was
/// asked for. Recorded under an unscoped key, that partial output is a complete stage as far as
/// the next run can tell: an unscoped `alef e2e generate` reads the hit and regenerates nothing
/// for any other language, leaving them stale with no diagnostic. `all` stands for "no selector
/// given", the same spelling `alef test-apps generate` uses, and the key stays selector-shaped
/// even when unscoped rather than collapsing to a bare `e2e` -- an entry keyed `e2e` carries no
/// evidence of which scope wrote it, and that is precisely the ambiguity being removed.
///
/// Consequence, and the same one `test-apps-{selector}` already has: `all_commands`'s e2e stage
/// still uses the bare `e2e` key (correctly -- it is unconditionally unscoped, passing `None` to
/// `generate_e2e`), so `alef all` and `alef e2e generate` no longer warm each other's stage cache
/// and each pays one regeneration after the other ran. ~keep
fn e2e_stage_cache_key(registry: bool, lang: Option<&[String]>) -> String {
    let stage = if registry { "e2e-registry" } else { "e2e" };
    let selector = lang
        .map(|languages| languages.join("-"))
        .unwrap_or_else(|| "all".to_string());
    format!("{stage}-{selector}")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The outcome under test is the cache *decision*, taken through `cache::is_stage_cached` --
    /// the same predicate the command branches on -- rather than a comparison of key strings. Two
    /// keys can differ and still collide in the stage cache (they are also the stage-hash input
    /// and the manifest filename), and it is the collision, not the string, that skips a full
    /// regeneration; an assertion on the strings alone would pass while the defect survived. ~keep
    #[test]
    fn scoped_e2e_stage_cache_does_not_satisfy_a_later_full_run() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cwd = crate::test_support::CwdGuard::enter(tmp.path());

        let ir_json = r#"{"crate_name":"sample_crate"}"#;
        let config_toml = "[e2e]\nlanguages = [\"python\", \"node\"]\n";
        let scoped_langs = vec!["python".to_string()];
        let scoped_key = e2e_stage_cache_key(false, Some(&scoped_langs));
        let full_key = e2e_stage_cache_key(false, None);

        let python_output = tmp.path().join("e2e/python/test_smoke.py");
        std::fs::create_dir_all(python_output.parent().expect("output parent")).expect("create output dir");
        std::fs::write(&python_output, "# generated\n").expect("write the scoped run's only output");
        let scoped_hash = cache::compute_stage_hash(ir_json, &scoped_key, config_toml, &[]);
        cache::write_stage_hash("sample-crate", &scoped_key, &scoped_hash, &[python_output])
            .expect("record the scoped run");

        assert!(
            cache::is_stage_cached("sample-crate", &scoped_key, &scoped_hash),
            "a repeat of the same scoped run must still hit its own cache"
        );
        let full_hash = cache::compute_stage_hash(ir_json, &full_key, config_toml, &[]);
        assert!(
            !cache::is_stage_cached("sample-crate", &full_key, &full_hash),
            "an unscoped run must regenerate rather than inherit a --lang-scoped run's partial output"
        );
        assert!(
            cache::read_stage_paths("sample-crate", &full_key).is_empty(),
            "no unscoped stage has been generated, so the unscoped manifest must not exist"
        );
    }

    #[test]
    fn e2e_stage_cache_key_separates_registry_mode_and_language_selections() {
        let python = vec!["python".to_string()];
        let node = vec!["node".to_string()];

        assert_eq!(e2e_stage_cache_key(false, None), "e2e-all");
        assert_eq!(e2e_stage_cache_key(true, None), "e2e-registry-all");
        assert_eq!(e2e_stage_cache_key(false, Some(&python)), "e2e-python");
        assert_eq!(e2e_stage_cache_key(true, Some(&python)), "e2e-registry-python");
        assert_ne!(
            e2e_stage_cache_key(false, Some(&python)),
            e2e_stage_cache_key(false, Some(&node))
        );
    }
}
