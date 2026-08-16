use anyhow::Result;
use std::path::PathBuf;
use std::process;

use crate::cli::{cache, dispatch, pipeline, version_pin};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::Extract { output } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let effective_output = if multi {
                    output
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join(format!("{}.ir.json", resolved_cfg.name))
                } else {
                    output.clone()
                };
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                if let Some(parent) = effective_output.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&effective_output, serde_json::to_string_pretty(&api)?)?;
                if multi {
                    tracing::info!("[{}] Wrote IR to {}", resolved_cfg.name, effective_output.display());
                } else {
                    tracing::info!("Wrote IR to {}", effective_output.display());
                }
            }
            Ok(None)
        }
        Commands::Generate { lang, clean, skip_frb } => {
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
            let (workspace, resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;

            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

            let mut grand_total_generated: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                pipeline::warn_missing_formatters(&languages);
                if multi {
                    tracing::info!(
                        "[{}] Generating bindings for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Generating bindings for: {}", format_languages(&languages));
                }
                let api = pipeline::extract(resolved_cfg, config_path, clean)?;
                let files = pipeline::generate(&api, resolved_cfg, &languages, clean, config_path, true)?;
                let regenerated_languages: std::collections::HashSet<_> =
                    files.iter().map(|(language, _)| *language).collect();
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

                let mut current_gen_paths = std::collections::HashSet::new();
                let mut language_output_paths: std::collections::HashMap<_, std::collections::HashSet<_>> = files
                    .iter()
                    .map(|(language, generated)| {
                        (
                            *language,
                            generated
                                .iter()
                                .filter(|file| file.carries_alef_marker())
                                .map(|file| base_dir.join(&file.path))
                                .collect(),
                        )
                    })
                    .collect();
                let mut generation_owned_paths: std::collections::HashMap<_, std::collections::HashSet<_>> = files
                    .iter()
                    .map(|(language, generated)| {
                        (
                            *language,
                            generated.iter().map(|file| base_dir.join(&file.path)).collect(),
                        )
                    })
                    .collect();
                for language in languages
                    .iter()
                    .filter(|language| !regenerated_languages.contains(language))
                {
                    let cached_paths = cache::read_lang_manifest(&resolved_cfg.name, &language.to_string());
                    current_gen_paths.extend(cached_paths.iter().cloned());
                    language_output_paths
                        .entry(*language)
                        .or_default()
                        .extend(cached_paths.iter().cloned());
                    generation_owned_paths
                        .entry(*language)
                        .or_default()
                        .extend(cached_paths);
                }
                let mut changed_languages: std::collections::HashSet<crate::core::config::Language> =
                    std::collections::HashSet::new();

                let mut generated_paths = std::collections::HashSet::new();
                let mut any_written = false;
                for (lang, lang_files) in &files {
                    generated_paths.extend(lang_files.iter().map(|file| base_dir.join(&file.path)));
                    let lang_str = lang.to_string();
                    for file in lang_files.iter().filter(|file| file.carries_alef_marker()) {
                        current_gen_paths.insert(base_dir.join(&file.path));
                    }

                    let hashes: Vec<(String, String)> = lang_files
                        .iter()
                        .map(|f| {
                            let normalized = pipeline::normalize_content(&f.path, &f.content);
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&normalized),
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
                    if report.changed_count() > 0 {
                        any_written = true;
                        changed_languages.insert(*lang);
                    }
                    let _ = cache::write_generation_hashes(&cache_key, &hashes);
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                if !api.services.is_empty() {
                    let svc_files = pipeline::generate_service_api(&api, resolved_cfg, &languages)?;
                    if !svc_files.is_empty() {
                        generated_paths.extend(
                            svc_files
                                .iter()
                                .flat_map(|(_, generated)| generated.iter())
                                .map(|file| base_dir.join(&file.path)),
                        );
                        for (_, files) in &svc_files {
                            for file in files.iter().filter(|file| file.carries_alef_marker()) {
                                current_gen_paths.insert(base_dir.join(&file.path));
                            }
                        }
                        for (language, generated) in &svc_files {
                            generation_owned_paths
                                .entry(*language)
                                .or_default()
                                .extend(generated.iter().map(|file| base_dir.join(&file.path)));
                            language_output_paths.entry(*language).or_default().extend(
                                generated
                                    .iter()
                                    .filter(|file| file.carries_alef_marker())
                                    .map(|file| base_dir.join(&file.path)),
                            );
                        }
                        let report = pipeline::write_files_report(&svc_files, &base_dir)?;
                        let svc_count = report.changed_count();
                        tracing::info!("Generated {svc_count} service API files");
                        if svc_count > 0 {
                            any_written = true;
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
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                if resolved_cfg.generate.public_api {
                    let public_api_files = pipeline::generate_public_api(&api, resolved_cfg, &languages, config_path)?;
                    if !public_api_files.is_empty() {
                        generated_paths.extend(
                            public_api_files
                                .iter()
                                .flat_map(|(_, generated)| generated.iter())
                                .map(|file| base_dir.join(&file.path)),
                        );
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
                        for (language, generated) in &public_api_files {
                            generation_owned_paths
                                .entry(*language)
                                .or_default()
                                .extend(generated.iter().map(|file| base_dir.join(&file.path)));
                            language_output_paths.entry(*language).or_default().extend(
                                generated
                                    .iter()
                                    .filter(|file| file.carries_alef_marker())
                                    .map(|file| base_dir.join(&file.path)),
                            );
                        }

                        if !api_match || clean {
                            let report = pipeline::write_files_report(&public_api_files, &base_dir)?;
                            let api_count = report.changed_count();
                            tracing::info!("Generated {api_count} public API files");
                            any_written |= api_count > 0;
                            let _ = cache::write_generation_hashes(&api_cache_key, &api_hashes);
                            for (lang, generated) in &public_api_files {
                                if generated
                                    .iter()
                                    .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                                {
                                    changed_languages.insert(*lang);
                                }
                            }
                        } else {
                            tracing::info!("  [public_api] up to date (skipping)");
                        }
                    }
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                let stub_files = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
                if !stub_files.is_empty() {
                    generated_paths.extend(
                        stub_files
                            .iter()
                            .flat_map(|(_, generated)| generated.iter())
                            .map(|file| base_dir.join(&file.path)),
                    );
                    let stub_hashes: Vec<(String, String)> = stub_files
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

                    for (_, files) in &stub_files {
                        for file in files.iter().filter(|file| file.carries_alef_marker()) {
                            current_gen_paths.insert(base_dir.join(&file.path));
                        }
                    }
                    for (language, generated) in &stub_files {
                        generation_owned_paths
                            .entry(*language)
                            .or_default()
                            .extend(generated.iter().map(|file| base_dir.join(&file.path)));
                        language_output_paths.entry(*language).or_default().extend(
                            generated
                                .iter()
                                .filter(|file| file.carries_alef_marker())
                                .map(|file| base_dir.join(&file.path)),
                        );
                    }

                    if !stubs_match || clean {
                        let report = pipeline::write_files_report(&stub_files, &base_dir)?;
                        let stub_count = report.changed_count();
                        tracing::info!("Generated {stub_count} type stub files");
                        any_written |= stub_count > 0;
                        let _ = cache::write_generation_hashes(&stubs_cache_key, &stub_hashes);

                        for (lang, generated) in &stub_files {
                            if generated
                                .iter()
                                .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                            {
                                changed_languages.insert(*lang);
                            }
                        }
                    } else {
                        tracing::info!("  [stubs] up to date (skipping)");
                    }
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                let report = pipeline::reconcile_managed_scaffold_manifests(&scaffold_files, &base_dir)?;
                if report.changed_count() > 0 {
                    any_written = true;
                }
                for file in &scaffold_files {
                    let path = base_dir.join(&file.path);
                    if file.carries_alef_marker() {
                        current_gen_paths.insert(path);
                    }
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                if any_written && !changed_languages.is_empty() {
                    tracing::info!("Formatting generated files...");
                    let mut files_to_format = files.clone();
                    files_to_format.extend(stub_files.clone());
                    pipeline::format_generated(&files_to_format, resolved_cfg, &base_dir, Some(&changed_languages));
                }
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                tracing::info!("Running post-build processing...");
                complete_generated_artifacts(&languages, resolved_cfg, &base_dir)?;

                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                let previous_generation_owned: std::collections::HashMap<_, _> = languages
                    .iter()
                    .map(|language| {
                        (
                            *language,
                            cache::read_stage_paths(&resolved_cfg.name, &format!("generate-{language}-ownership")),
                        )
                    })
                    .collect();
                for (language, previous_paths) in &previous_generation_owned {
                    if !regenerated_languages.contains(language) {
                        generation_owned_paths
                            .entry(*language)
                            .or_default()
                            .extend(previous_paths.iter().cloned());
                    }
                }
                let cleanup_keep_paths: std::collections::HashSet<_> = generation_owned_paths
                    .values()
                    .flat_map(|paths| paths.iter().cloned())
                    .collect();
                let cleanup_roots = pipeline::generate_sweep_roots(&languages, lang.is_some(), resolved_cfg, &base_dir);
                let previous_paths: Vec<_> = previous_generation_owned.into_values().flatten().collect();
                pipeline::sweep_manifest_orphans(&previous_paths, &cleanup_keep_paths, &cleanup_roots)?;
                for (language, paths) in &generation_owned_paths {
                    let paths: Vec<_> = paths.iter().cloned().collect();
                    cache::write_stage_hash(
                        &resolved_cfg.name,
                        &format!("generate-{language}-ownership"),
                        &sources_hash,
                        &paths,
                    )?;
                }
                for (language, paths) in language_output_paths {
                    let paths: Vec<_> = paths.into_iter().collect();
                    cache::write_lang_manifest(&resolved_cfg.name, &language.to_string(), &paths)?;
                }

                if let Err(e) = pipeline::sync_versions(resolved_cfg, config_path, None, true, true, None) {
                    tracing::warn!("version sync failed: {e}");
                }

                if resolved_cfg.e2e.is_some() {
                    tracing::warn!("[e2e] block detected — run 'alef e2e generate' to regenerate e2e test suites");
                }

                grand_total_generated += generated_paths.len();
            }
            tracing::info!("Generated {grand_total_generated} files");
            Ok(None)
        }
        Commands::Stubs { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    tracing::info!(
                        "[{}] Generating type stubs for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Generating type stubs for: {}", format_languages(&languages));
                }
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let files = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

                let hashes: Vec<(String, String)> = files
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

                let cache_key = format!("{}.stubs", resolved_cfg.name);
                let stored = cache::read_generation_hashes(&cache_key).unwrap_or_default();
                let all_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                if all_match {
                    if multi {
                        tracing::info!("[{}] Stubs up to date (skipping)", resolved_cfg.name);
                    } else {
                        tracing::info!("Stubs up to date (skipping)");
                    }
                    continue;
                }

                let count = pipeline::write_files(&files, &base_dir)?;
                let _ = cache::write_generation_hashes(&cache_key, &hashes);

                pipeline::format_generated(&files, resolved_cfg, &base_dir, None);

                let stub_paths: std::collections::HashSet<PathBuf> = files
                    .iter()
                    .flat_map(|(_, fs)| {
                        fs.iter()
                            .filter(|file| file.carries_alef_marker())
                            .map(|file| base_dir.join(&file.path))
                    })
                    .collect();
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                pipeline::finalize_hashes(&stub_paths, &sources_hash, &alef_toml_bytes)?;
                grand_total += count;
            }
            tracing::info!("Generated {grand_total} stub files");
            Ok(None)
        }
        Commands::Scaffold { lang } => {
            let (workspace, resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;

            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let ir_json = serde_json::to_string(&api)?;
                let stage_hash = cache::compute_stage_hash(&ir_json, "scaffold", &config_toml, &[]);
                if cache::is_stage_cached(&resolved_cfg.name, "scaffold", &stage_hash) {
                    if multi {
                        tracing::info!("[{}] Scaffold up to date (cached)", resolved_cfg.name);
                    } else {
                        tracing::info!("Scaffold up to date (cached)");
                    }
                    continue;
                }
                if multi {
                    tracing::info!(
                        "[{}] Generating scaffolding for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Generating scaffolding for: {}", format_languages(&languages));
                }
                let files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                let count = pipeline::write_scaffold_files(&files, &base_dir)?;
                let output_paths: Vec<PathBuf> = files
                    .iter()
                    .filter(|file| file.carries_alef_marker())
                    .map(|file| base_dir.join(&file.path))
                    .collect();
                let scaffold_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&scaffold_paths, &sources_hash, &alef_toml_bytes)?;
                cache::write_stage_hash(&resolved_cfg.name, "scaffold", &stage_hash, &output_paths)?;
                grand_total += count;
            }

            pipeline::install_poly_hooks(&base_dir);

            // downstream crates can use `#[cfg_attr(feature = "alef-meta", alef(since = "..."))]`
            match pipeline::ensure_workspace_alef_meta_check_cfg() {
                Ok(true) => tracing::info!(
                    "Patched Cargo.toml: added [workspace.lints.rust] unexpected_cfgs allowlist for alef-meta"
                ),
                Ok(false) => {}
                Err(e) => tracing::warn!("could not patch workspace lints for alef-meta: {e}"),
            }

            tracing::info!("Generated {grand_total} scaffold files");
            Ok(None)
        }
        Commands::Readme { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = crate::readme::expand_configured_readme_languages(
                    resolved_cfg,
                    &resolve_readme_languages(resolved_cfg, lang.as_deref())?,
                );
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let ir_json = serde_json::to_string(&api)?;
                let stage_hash = cache::compute_stage_hash(&ir_json, "readme", &config_toml, &[]);
                if cache::is_stage_cached(&resolved_cfg.name, "readme", &stage_hash) {
                    if multi {
                        tracing::info!("[{}] READMEs up to date (cached)", resolved_cfg.name);
                    } else {
                        tracing::info!("READMEs up to date (cached)");
                    }
                    continue;
                }
                if multi {
                    tracing::info!(
                        "[{}] Generating READMEs for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Generating READMEs for: {}", format_languages(&languages));
                }
                let files = pipeline::readme(&api, resolved_cfg, &languages)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                let output_paths: Vec<PathBuf> = files
                    .iter()
                    .filter(|file| file.carries_alef_marker())
                    .map(|file| base_dir.join(&file.path))
                    .collect();
                let readme_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&readme_paths, &sources_hash, &alef_toml_bytes)?;
                cache::write_stage_hash(&resolved_cfg.name, "readme", &stage_hash, &output_paths)?;
                grand_total += count;
            }
            tracing::info!("Generated {grand_total} README files");
            Ok(None)
        }
        Commands::Docs { lang, output } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_doc_languages(resolved_cfg, lang.as_deref())?;
                let selector = languages.iter().map(ToString::to_string).collect::<Vec<_>>().join("-");
                let output_key = output.as_deref().unwrap_or("default");
                let docs_stage_key = format!("docs-{}", cache::hash_content(&format!("{selector}:{output_key}")));
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let ir_json = serde_json::to_string(&api)?;
                let stage_hash = cache::compute_stage_hash(&ir_json, &docs_stage_key, &config_toml, &[]);
                let use_stage_cache = resolved_cfg.docs.is_none();
                if use_stage_cache && cache::is_stage_cached(&resolved_cfg.name, &docs_stage_key, &stage_hash) {
                    if multi {
                        tracing::info!("[{}] Docs up to date (cached)", resolved_cfg.name);
                    } else {
                        tracing::info!("Docs up to date (cached)");
                    }
                    continue;
                }
                if multi {
                    tracing::info!(
                        "[{}] Generating docs for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Generating docs for: {}", format_languages(&languages));
                }
                // `generate_docs_stage` hands back every page it rendered even when a later step
                // (snippet validation, CLI/MCP adoption, llms/skills) fails, specifically so a
                // strict-mode bail never discards already-rendered API reference pages. Write
                // `files` before propagating `docs_result`, not after. ~keep
                let (files, docs_result) =
                    crate::docs::generate_docs_stage(&api, resolved_cfg, &languages, output.as_deref(), &base_dir);
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                let report = pipeline::write_scaffold_files_report(&files, &base_dir, true)?;
                docs_result?;
                let count = report.changed_count();
                let output_paths: Vec<PathBuf> = files
                    .iter()
                    .filter(|file| file.carries_alef_marker())
                    .map(|file| base_dir.join(&file.path))
                    .collect();
                let doc_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&doc_paths, &sources_hash, &alef_toml_bytes)?;
                if use_stage_cache {
                    cache::write_stage_hash(&resolved_cfg.name, &docs_stage_key, &stage_hash, &output_paths)?;
                }
                grand_total += count;
            }
            tracing::info!("Generated {grand_total} doc files");
            Ok(None)
        }
        Commands::SyncVersions {
            bump,
            set,
            regen,
            skip_swift_checksum,
            release_date,
        } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                if let Some(version) = &set {
                    if multi {
                        tracing::info!("[{}] Setting version to {version}", resolved_cfg.name);
                    } else {
                        tracing::info!("Setting version to {version}");
                    }
                    pipeline::set_version(resolved_cfg, version)?;
                }
                if multi {
                    tracing::info!("[{}] Syncing versions from Cargo.toml", resolved_cfg.name);
                } else {
                    tracing::info!("Syncing versions from Cargo.toml");
                }
                pipeline::sync_versions(
                    resolved_cfg,
                    config_path,
                    bump.as_deref(),
                    !regen,
                    skip_swift_checksum,
                    release_date.as_deref(),
                )?;
            }
            tracing::info!("Version sync complete");
            Ok(None)
        }
        Commands::Build { lang, release } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                let profile = if release { "release" } else { "dev" };
                if multi {
                    tracing::info!(
                        "[{}] Building bindings ({profile}) for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Building bindings ({profile}) for: {}", format_languages(&languages));
                }
                pipeline::build(resolved_cfg, &languages, release)?;
            }
            tracing::info!("Build complete");
            Ok(None)
        }
        Commands::Fmt { lang: _ } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            for resolved_cfg in &crates_to_process {
                if multi {
                    tracing::info!("[{}] Formatting generated output...", resolved_cfg.name);
                } else {
                    tracing::info!("Formatting generated output...");
                }
                pipeline::fmt(resolved_cfg, &base_dir)?;
            }
            tracing::info!("Format complete");
            Ok(None)
        }
        Commands::Lint { lang: _ } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            for resolved_cfg in &crates_to_process {
                if multi {
                    tracing::info!("[{}] Linting generated output...", resolved_cfg.name);
                } else {
                    tracing::info!("Linting generated output...");
                }
                pipeline::lint(resolved_cfg, &base_dir)?;
            }
            tracing::info!("Lint complete");
            Ok(None)
        }
        Commands::Test { lang, e2e, coverage } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_test_languages(resolved_cfg, lang.as_deref(), e2e)?;
                if multi {
                    tracing::info!(
                        "[{}] Running tests for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Running tests for: {}", format_languages(&languages));
                }
                if e2e {
                    tracing::info!("  (with e2e tests)");
                }
                if coverage {
                    tracing::info!("  (with coverage)");
                }
                pipeline::test(resolved_cfg, &languages, e2e, coverage)?;
            }
            tracing::info!("Tests complete");
            Ok(None)
        }
        Commands::Setup { lang, timeout } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    tracing::info!(
                        "[{}] Setting up dependencies for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Setting up dependencies for: {}", format_languages(&languages));
                }
                pipeline::setup(resolved_cfg, &languages, timeout)?;
            }
            tracing::info!("Setup complete");
            Ok(None)
        }
        Commands::Clean { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    tracing::info!(
                        "[{}] Cleaning build artifacts for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Cleaning build artifacts for: {}", format_languages(&languages));
                }
                pipeline::clean(resolved_cfg, &languages)?;
            }
            tracing::info!("Clean complete");
            Ok(None)
        }
        Commands::Update { lang, latest } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                let mode = if latest { "latest" } else { "compatible" };
                if multi {
                    tracing::info!(
                        "[{}] Updating dependencies ({mode}) for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Updating dependencies ({mode}) for: {}", format_languages(&languages));
                }
                pipeline::update(resolved_cfg, &languages, latest)?;
            }
            tracing::info!("Update complete");
            Ok(None)
        }
        Commands::Verify {
            exit_code: _,
            report_only,
            compile: _,
            lint: _,
            lang: _,
        } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            tracing::info!("Verifying alef-generated files (inputs-hash mode)");
            let base_dir = std::env::current_dir()?;

            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

            let all_inputs_hashes: Vec<String> = crates_to_process
                .iter()
                .filter_map(|c| cache::sources_hash(&c.sources).ok())
                .map(|sh| crate::core::hash::compute_inputs_hash(&sh, &alef_toml_bytes))
                .collect();

            let stale = verify_walk_multi(&base_dir, &all_inputs_hashes)?;

            let mut snippet_coverage_issues = Vec::new();
            // `verify_walk_multi` only sees files that already exist on disk; a file
            // generation would now produce but that was never written (a backend
            // that emits one file per public type, an item added since the last
            // regen) is invisible to it. Closing that requires knowing what
            // generation would produce, so every crate pays a regeneration pass
            // here (mirrors `alef diff`) to find files entirely absent from disk, and
            // -- in the same pass -- files that exist but were never marked and so
            // can never be written by a plain `alef generate` (frozen; see
            // `FrozenFile`). ~keep
            let mut missing_generated_files: Vec<String> = Vec::new();
            let mut frozen_generated_files: Vec<FrozenFile> = Vec::new();
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let found =
                    find_missing_and_frozen_generated_files(&languages, &api, resolved_cfg, config_path, &base_dir)?;
                missing_generated_files.extend(found.missing);
                frozen_generated_files.extend(found.frozen);

                let Some(e2e_config) = &resolved_cfg.e2e else {
                    continue;
                };
                if let Err(error) = crate::e2e::verify_fresh_snippet_coverage(
                    &base_dir,
                    resolved_cfg,
                    e2e_config,
                    &api.types,
                    &api.enums,
                    &api.functions,
                ) {
                    snippet_coverage_issues.push(format!("[{}] {error:#}", resolved_cfg.name));
                }
            }
            missing_generated_files.sort();
            missing_generated_files.dedup();
            frozen_generated_files.sort_by(|a, b| a.path.cmp(&b.path));
            frozen_generated_files.dedup_by(|a, b| a.path == b.path);
            let has_missing_files = !missing_generated_files.is_empty();
            let has_frozen_files = !frozen_generated_files.is_empty();

            // Catches the cross-artifact ABI straddle a per-file hash check cannot
            // see: an FFI header and a binding backend's opaque-handle file each
            // individually fresh against current inputs, but stamped by two
            // different handle-ABI generations because only one side was
            // regenerated. See `crate::core::hash::HANDLE_ABI_STAMP_KEY` and
            // `find_stamp_disagreement` for why 0/1 distinct values is silently
            // fine and only 2+ is reported. ~keep
            let abi_disagreement = find_stamp_disagreement(&base_dir, crate::core::hash::HANDLE_ABI_STAMP_KEY);
            let has_abi_disagreement = abi_disagreement.is_some();
            if let Some(disagreement) = &abi_disagreement {
                crate::bin_cli::output::line(format_args!(
                    "ABI generation disagreement detected for `{}`:",
                    disagreement.key
                ));
                for (path, value) in &disagreement.examples {
                    crate::bin_cli::output::line(format_args!("  {path} -> {value}"));
                }
            }

            let mut all_version_mismatches: Vec<String> = Vec::new();
            for resolved_cfg in &crates_to_process {
                let mismatches = pipeline::verify_versions(resolved_cfg)?;
                all_version_mismatches.extend(mismatches);
            }
            let has_version_issues = !all_version_mismatches.is_empty();
            if has_version_issues {
                crate::bin_cli::output::line("Version mismatches detected:");
                for mismatch in &all_version_mismatches {
                    crate::bin_cli::output::line(format_args!("  {mismatch}"));
                }
            }
            if !snippet_coverage_issues.is_empty() {
                crate::bin_cli::output::line("Snippet coverage issues detected:");
                for issue in &snippet_coverage_issues {
                    crate::bin_cli::output::line(format_args!("  {issue}"));
                }
            }

            if stale.is_empty()
                && !has_missing_files
                && !has_frozen_files
                && !has_abi_disagreement
                && !has_version_issues
                && snippet_coverage_issues.is_empty()
            {
                crate::bin_cli::output::line("All bindings and versions are up to date.");
            } else {
                if !stale.is_empty() {
                    crate::bin_cli::output::line("Stale bindings detected:");
                    for s in &stale {
                        crate::bin_cli::output::line(format_args!("  {}", s.path));
                        if tracing::enabled!(tracing::Level::DEBUG) {
                            crate::bin_cli::output::line(format_args!("    embedded:  {}", s.embedded));
                            let computed_str = s.computed.join(", ");
                            crate::bin_cli::output::line(format_args!("    computed:  {computed_str}"));
                        }
                    }
                }
                if has_missing_files {
                    crate::bin_cli::output::line("Missing generated files detected:");
                    for path in &missing_generated_files {
                        crate::bin_cli::output::line(format_args!("  {path}"));
                    }
                }
                // Reported separately from stale/missing, never folded into either
                // count: the remedy is different (a human must review and adopt or
                // delete the file -- `alef generate` alone cannot fix it) and folding
                // it in would make a frozen file look like ordinary drift. ~keep
                if has_frozen_files {
                    crate::bin_cli::output::line(
                        "Frozen generated files detected (alef owns these paths but the files carry no \
                         provenance marker, so alef refuses to write them -- review each file, then either \
                         add the marker shown and rerun `alef generate`, or delete the file so generation \
                         can write it cleanly):",
                    );
                    for frozen in &frozen_generated_files {
                        crate::bin_cli::output::line(format_args!("  {}", frozen.path));
                        match &frozen.remedy {
                            Some(remedy) => crate::bin_cli::output::line(format_args!("    add marker: {remedy}")),
                            None => crate::bin_cli::output::line(
                                "    this format has no comment syntax to carry a marker -- adopt or delete it",
                            ),
                        }
                    }
                }
            }
            super::verify_outcome::ensure_success(
                !stale.is_empty() || has_missing_files || has_frozen_files || has_abi_disagreement,
                has_version_issues,
                snippet_coverage_issues.len(),
                report_only,
            )?;
            Ok(None)
        }
        Commands::Diff { exit_code } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            tracing::info!("Computing diff of generated bindings...");
            let base_dir = std::env::current_dir()?;
            let mut all_diffs: Vec<String> = Vec::new();
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, true, config_path, true)?;
                let stubs = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
                let scaffold = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                all_diffs.extend(pipeline::diff_files(&bindings, &base_dir)?);
                all_diffs.extend(pipeline::diff_files(&stubs, &base_dir)?);
                all_diffs.extend(pipeline::diff_files(
                    &[(crate::core::config::Language::Rust, scaffold)],
                    &base_dir,
                )?);
            }

            if all_diffs.is_empty() {
                crate::bin_cli::output::line("No changes detected.");
            } else {
                crate::bin_cli::output::line("Files that would change:");
                for diff in &all_diffs {
                    crate::bin_cli::output::line(format_args!("  {diff}"));
                }
                if exit_code {
                    process::exit(1);
                }
            }
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}
