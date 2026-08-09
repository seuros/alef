//! API reference documentation generator for alef polyglot bindings.
//!
//! Generates per-language `api-{lang}.md` files plus shared `configuration.md`
//! and `errors.md` files from the alef IR (`ApiSurface`).

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use anyhow::Context;
use heck::ToPascalCase;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod context;
mod descriptions;
pub mod doc_cleaning;
mod examples;
mod formatting;
mod language_pages;
pub(crate) mod naming;
mod render;
mod rust_static;
mod shared_pages;
mod signatures;
mod sorting;
pub(crate) mod template_env;
#[cfg(test)]
mod tests;
mod type_mapping;
mod version_labels;

#[cfg(test)]
pub(crate) mod test_helpers;

pub use doc_cleaning::clean_doc;
pub use type_mapping::doc_type;

pub use context::{CliSurface, DocsRenderContext, McpSurface};

/// Generate API reference documentation for the given languages.
///
/// Produces one `api-{lang}.md` per language, plus shared `configuration.md`,
/// `types.md`, and `errors.md` files written into `output_dir`.
pub fn generate_docs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_dir: &str,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();
    let ffi_prefix = &config.ffi_prefix().to_pascal_case();

    for &lang in languages {
        files.push(language_pages::generate_lang_doc(
            api, config, lang, output_dir, ffi_prefix,
        )?);
    }

    files.push(shared_pages::generate_configuration_doc(api, config, output_dir)?);
    files.push(shared_pages::generate_types_doc(api, output_dir)?);
    files.push(shared_pages::generate_errors_doc(api, output_dir)?);

    for file in &mut files {
        file.content = doc_cleaning::wrap_bare_urls(&file.content);
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    Ok(files)
}

/// Generate the complete docs stage: API reference, optional CLI/MCP reference,
/// optional template-rendered llms.txt and skills, and configured snippet checks.
/// The reference-docs output directory for `config` — `[docs].reference_output`
/// or the `docs/reference` default. Relative to the workspace root; callers join
/// it under the base directory.
///
/// Exposed so the generate pipeline can protect committed reference pages from
/// orphan cleanup: the page set `generate_docs_stage` emits depends on host
/// state (CLI/MCP source presence, doc languages), so a host that produces fewer
/// pages must not delete the committed ones it simply did not regenerate (#184).
pub fn reference_output_dir(config: &ResolvedCrateConfig) -> PathBuf {
    config
        .docs
        .as_ref()
        .and_then(|docs| docs.reference_output.clone())
        .unwrap_or_else(|| PathBuf::from("docs/reference"))
}

pub fn generate_docs_stage(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_override: Option<&str>,
    workspace_root: &Path,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let reference_output = output_override
        .map(PathBuf::from)
        .unwrap_or_else(|| reference_output_dir(config));
    let reference_output_str = reference_output.to_string_lossy().to_string();

    let mut files = generate_docs(api, config, languages, &reference_output_str)?;
    for file in &mut files {
        file.content = with_markdown_alef_header(&file.content);
        file.generated_header = true;
    }

    let mut context = build_base_context(api, config, languages, &files);
    let snippet_dirs = build_snippet_context(config, workspace_root, &mut context)?;

    if let Some(docs_cfg) = &config.docs {
        if let Some(cli_cfg) = &docs_cfg.cli
            && cli_cfg.is_enabled()
        {
            let explicit_sources = !cli_cfg.sources.is_empty();
            let sources = docs_sources(config, &cli_cfg.sources, workspace_root);
            warn_missing_explicit_sources("CLI", &cli_cfg.sources, workspace_root);
            let surface = rust_static::extract_cli_surface(&sources)?;
            if surface.commands.is_empty() {
                if explicit_sources {
                    tracing::warn!("docs.cli was configured but no clap commands were discovered");
                }
            } else {
                let path = cli_cfg
                    .output
                    .clone()
                    .unwrap_or_else(|| reference_output.join("cli.md"));
                render::ensure_managed_or_adopted(workspace_root, &path, cli_cfg.adopt_existing)?;
                files.push(render::generate_cli_doc(&surface, path.clone()));
                context.references.push(context::ReferenceDoc {
                    kind: "cli".to_string(),
                    title: "CLI Reference".to_string(),
                    path: path.to_string_lossy().to_string(),
                });
                context.cli = surface;
            }
        }

        if let Some(mcp_cfg) = &docs_cfg.mcp
            && mcp_cfg.is_enabled()
        {
            let explicit_sources = !mcp_cfg.sources.is_empty();
            let sources = docs_sources(config, &mcp_cfg.sources, workspace_root);
            warn_missing_explicit_sources("MCP", &mcp_cfg.sources, workspace_root);
            let surface = rust_static::extract_mcp_surface(&sources)?;
            if surface.tools.is_empty() && surface.prompts.is_empty() && surface.resources.is_empty() {
                if explicit_sources {
                    tracing::warn!("docs.mcp was configured but no rmcp tools, prompts, or resources were discovered");
                }
            } else {
                let path = mcp_cfg
                    .output
                    .clone()
                    .unwrap_or_else(|| reference_output.join("mcp.md"));
                render::ensure_managed_or_adopted(workspace_root, &path, mcp_cfg.adopt_existing)?;
                files.push(render::generate_mcp_doc(&surface, path.clone()));
                context.references.push(context::ReferenceDoc {
                    kind: "mcp".to_string(),
                    title: "MCP Reference".to_string(),
                    path: path.to_string_lossy().to_string(),
                });
                context.mcp = surface;
            }
        }

        if let Some(llms_cfg) = &docs_cfg.llms {
            files.push(render::render_llms(llms_cfg, &context, workspace_root, &snippet_dirs)?);
        }

        if let Some(skills_cfg) = &docs_cfg.skills {
            files.extend(render::render_skills(
                skills_cfg,
                &context,
                workspace_root,
                &snippet_dirs,
            )?);
        }
    }

    for file in &mut files {
        file.content = doc_cleaning::wrap_bare_urls(&file.content);
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    Ok(files)
}

fn build_base_context(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    api_files: &[GeneratedFile],
) -> DocsRenderContext {
    let description = config
        .scaffold
        .as_ref()
        .and_then(|scaffold| scaffold.description.clone())
        .unwrap_or_else(|| format!("Bindings for {}", config.name));
    let license = config
        .scaffold
        .as_ref()
        .and_then(|scaffold| scaffold.license.clone())
        .unwrap_or_else(|| "MIT".to_string());
    let api_references = api_files
        .iter()
        .map(|file| {
            let path = file.path.to_string_lossy().to_string();
            context::ReferenceDoc {
                kind: "api".to_string(),
                title: path
                    .rsplit('/')
                    .next()
                    .unwrap_or(path.as_str())
                    .trim_end_matches(".md")
                    .replace('-', " "),
                path,
            }
        })
        .collect::<Vec<_>>();

    DocsRenderContext {
        krate: context::CrateDocsContext {
            name: config.name.clone(),
            version: api.version.clone(),
            description,
            repository: config.github_repo(),
            license,
        },
        languages: languages.iter().map(ToString::to_string).collect(),
        references: api_references.clone(),
        api_references,
        ..DocsRenderContext::default()
    }
}

fn build_snippet_context(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    context: &mut DocsRenderContext,
) -> anyhow::Result<Vec<PathBuf>> {
    let Some(snippet_cfg) = config.docs.as_ref().and_then(|docs| docs.snippets.as_ref()) else {
        return Ok(Vec::new());
    };

    for dir in snippet_cfg.dirs.iter().chain(&snippet_cfg.inline_dirs) {
        let abs_dir = workspace_root.join(dir);
        if !abs_dir.exists() {
            anyhow::bail!(
                "configured docs.snippets.dirs root '{}' (resolved to '{}') does not exist",
                dir.display(),
                abs_dir.display()
            );
        }
    }
    let snippet_dirs = snippet_cfg.dirs.clone();
    let discovery_dirs = snippet_cfg
        .dirs
        .iter()
        .chain(&snippet_cfg.inline_dirs)
        .cloned()
        .collect::<Vec<_>>();
    if discovery_dirs.is_empty() {
        if snippet_cfg.validation_level.is_some() || !snippet_cfg.required_languages.is_empty() {
            tracing::warn!("docs.snippets is configured for validation but docs.snippets.dirs is empty");
        }
        return Ok(Vec::new());
    }

    let absolute_snippet_dirs = snippet_dirs
        .iter()
        .map(|dir| workspace_root.join(dir))
        .collect::<Vec<_>>();
    let absolute_discovery_dirs = discovery_dirs
        .iter()
        .map(|dir| workspace_root.join(dir))
        .collect::<Vec<_>>();
    let excluded = snippet_cfg
        .exclude
        .iter()
        .map(|path| workspace_root.join(path))
        .collect::<Vec<_>>();
    let snippets = crate::snippets::discovery::discover_snippets(&absolute_discovery_dirs, None)?
        .into_iter()
        .filter(|snippet| !excluded.iter().any(|prefix| snippet.path.starts_with(prefix)))
        .collect::<Vec<_>>();
    let mut counts_by_language = BTreeMap::new();
    for snippet in &snippets {
        *counts_by_language.entry(snippet.language.to_string()).or_insert(0) += 1;
    }
    context.snippets = context::SnippetIndexContext {
        dirs: snippet_dirs
            .iter()
            .map(|dir| dir.to_string_lossy().to_string())
            .collect(),
        snippets: snippets
            .iter()
            .map(|snippet| context::SnippetContext {
                id: snippet.id.clone(),
                path: snippet.path.to_string_lossy().to_string(),
                language: snippet.language.to_string(),
                title: snippet.title.clone(),
                tags: snippet.metadata.tags.clone(),
            })
            .collect(),
        counts_by_language,
    };

    validate_snippets(config, workspace_root, snippet_cfg, &absolute_snippet_dirs, &snippets)?;
    Ok(snippet_dirs)
}

fn validate_snippets(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    snippet_cfg: &crate::core::config::DocsSnippetsConfig,
    absolute_snippet_dirs: &[PathBuf],
    snippets: &[crate::snippets::types::Snippet],
) -> anyhow::Result<()> {
    let docs_dirs = if snippet_cfg.docs_dirs.is_empty() {
        Vec::new()
    } else {
        snippet_cfg
            .docs_dirs
            .iter()
            .map(|dir| workspace_root.join(dir))
            .collect::<Vec<_>>()
    };
    let include_base_paths = if snippet_cfg.include_base_paths.is_empty() {
        docs_dirs.clone()
    } else {
        snippet_cfg
            .include_base_paths
            .iter()
            .map(|dir| workspace_root.join(dir))
            .collect::<Vec<_>>()
    };
    let exclude = snippet_cfg
        .exclude
        .iter()
        .map(|path| workspace_root.join(path))
        .collect::<Vec<_>>();
    let mut configured_references =
        crate::snippets::gaps::readme_snippet_references(workspace_root, config.readme.as_ref());
    configured_references.extend(ledger_generated_references(
        workspace_root,
        config,
        absolute_snippet_dirs,
    )?);

    if !docs_dirs.is_empty() {
        let audit_report = crate::snippets::audit::audit(&crate::snippets::audit::AuditConfig {
            docs_dirs: docs_dirs.clone(),
            snippet_dirs: absolute_snippet_dirs.to_vec(),
            include_base_paths: include_base_paths.clone(),
            configured_references: configured_references.clone(),
            exclude: exclude.clone(),
            require_frontmatter: snippet_cfg.require_frontmatter,
        });
        if audit_report.has_errors() {
            let summary = audit_report
                .issues
                .iter()
                .take(8)
                .map(|issue| format!("{}:{}: {}", issue.path.display(), issue.line, issue.message))
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!("snippet audit failed for crate `{}`:\n{summary}", config.name);
        }
    }

    let required_languages = snippet_cfg
        .required_languages
        .iter()
        .map(|lang| lang.parse::<crate::snippets::types::Language>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| anyhow::anyhow!("invalid docs.snippets.required_languages entry: {err}"))?;

    if !docs_dirs.is_empty() || !required_languages.is_empty() {
        let report = crate::snippets::gaps::detect_gaps(&crate::snippets::gaps::GapConfig {
            docs_dirs,
            snippet_dirs: absolute_snippet_dirs.to_vec(),
            required_languages,
            include_base_paths,
            configured_references,
            exclude,
        })?;
        if !report.unreferenced_snippets.is_empty() && snippet_cfg.strict {
            anyhow::bail!(
                "strict snippet coverage failed for crate `{}`: {} unreferenced snippet file(s)",
                config.name,
                report.unreferenced_snippets.len()
            );
        }
        if !report.unreferenced_snippets.is_empty() {
            tracing::warn!(
                "docs.snippets found {} unreferenced snippet file(s); not failing because extra examples can be intentional",
                report.unreferenced_snippets.len()
            );
        }
        if !report.missing_references.is_empty()
            || !report.missing_language_variants.is_empty()
            || !report.skips_without_reason.is_empty()
            || !report.unknown_languages.is_empty()
        {
            anyhow::bail!("snippet gap validation failed for crate `{}`", config.name);
        }
    }

    if let Some(level) = &snippet_cfg.validation_level {
        let level = level
            .parse::<crate::snippets::types::ValidationLevel>()
            .map_err(|err| anyhow::anyhow!("invalid docs.snippets.validation_level: {err}"))?;
        let mut runner_cfg = crate::snippets::runner::RunnerConfig {
            level,
            fail_fast: snippet_cfg.fail_fast,
            deny_unclassified: snippet_cfg.deny_unclassified,
            allowed_side_effects: parse_allowed_side_effects(&snippet_cfg.allowed_side_effects)?,
            cache_dir: Some(workspace_root.join(snippet_cfg.cache_dir())),
            sessions: snippet_cfg
                .sessions
                .iter()
                .map(|(target, session)| {
                    let normalized = crate::snippets::types::Language::normalize_session_target(target);
                    let language = crate::snippets::types::Language::from_session_target(&normalized);
                    if language == crate::snippets::types::Language::Unknown {
                        anyhow::bail!("unknown docs.snippets session target `{target}`");
                    }
                    Ok((
                        normalized,
                        crate::snippets::session::SessionSpec {
                            language,
                            working_directory: workspace_root.join(&session.cwd),
                            manifest: session.manifest.as_ref().map(|path| workspace_root.join(path)),
                            before: session.before.clone(),
                            env: session.env.clone(),
                        },
                    ))
                })
                .collect::<anyhow::Result<_>>()?,
            ..crate::snippets::runner::RunnerConfig::default()
        };
        if let Some(timeout_secs) = snippet_cfg.timeout_secs {
            runner_cfg.timeout_secs = timeout_secs;
        }
        let registry = crate::snippets::validators::ValidatorRegistry::default();
        let summary = crate::snippets::runner::run_validation(snippets, &registry, &runner_cfg)?;
        if summary.unavailable > 0 && snippet_cfg.strict {
            anyhow::bail!(
                "strict snippet validation failed for crate `{}`: {} validation(s) unavailable",
                config.name,
                summary.unavailable
            );
        }
        if summary.downgraded > 0 && snippet_cfg.strict {
            anyhow::bail!(
                "strict snippet validation failed for crate `{}`: {} validation(s) downgraded",
                config.name,
                summary.downgraded
            );
        }
        if summary.unavailable > 0 {
            tracing::warn!(
                "docs.snippets skipped {} snippet validation(s) because required toolchains were unavailable",
                summary.unavailable
            );
        }
        if summary.has_failures() {
            anyhow::bail!(
                "snippet validation failed for crate `{}`: {} failed, {} errors",
                config.name,
                summary.failed,
                summary.errors
            );
        }
        if let Some(path) = &snippet_cfg.report_output {
            let report_path = workspace_root.join(path);
            crate::snippets::output::write_report(&summary, &report_path, false).map_err(|err| {
                anyhow::anyhow!(
                    "writing snippet validation report to '{}': {err}",
                    report_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn ledger_generated_references(
    workspace_root: &std::path::Path,
    config: &crate::core::config::ResolvedCrateConfig,
    snippet_dirs: &[std::path::PathBuf],
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let Some(snippets) = config.e2e.as_ref().and_then(|e2e| e2e.snippets.as_ref()) else {
        return Ok(Vec::new());
    };
    let output_root = workspace_root.join(&snippets.output);
    if !snippet_dirs.iter().any(|directory| directory == &output_root) {
        return Ok(Vec::new());
    }
    let manifest = output_root.join(crate::e2e::snippets::COVERAGE_MANIFEST);
    let content = std::fs::read_to_string(&manifest).with_context(|| {
        format!(
            "strict fixture-snippet coverage manifest is missing: {}",
            manifest.display()
        )
    })?;
    let ledger: crate::e2e::snippets::SnippetCoverageLedger = serde_json::from_str(&content).with_context(|| {
        format!(
            "failed to parse fixture-snippet coverage manifest: {}",
            manifest.display()
        )
    })?;
    if ledger.format_version != crate::e2e::snippets::COVERAGE_MANIFEST_VERSION {
        anyhow::bail!(
            "stale fixture-snippet coverage manifest version {} at {}; expected {}",
            ledger.format_version,
            manifest.display(),
            crate::e2e::snippets::COVERAGE_MANIFEST_VERSION
        );
    }
    if !ledger.missing.is_empty()
        || ledger.expected.len() != ledger.generated.len() + ledger.documented_exceptions.len()
    {
        anyhow::bail!("incomplete fixture-snippet coverage manifest at {}", manifest.display());
    }
    if ledger.generated_paths.len() != ledger.generated.len()
        || ledger.generated_metadata.len() != ledger.generated_paths.len()
    {
        anyhow::bail!("stale fixture-snippet path ledger at {}", manifest.display());
    }
    let metadata_paths = ledger
        .generated_metadata
        .iter()
        .map(|metadata| metadata.path.as_path())
        .collect::<std::collections::BTreeSet<_>>();
    let generated_paths = ledger
        .generated_paths
        .iter()
        .map(PathBuf::as_path)
        .collect::<std::collections::BTreeSet<_>>();
    if metadata_paths != generated_paths {
        anyhow::bail!("stale fixture-snippet metadata ledger at {}", manifest.display());
    }
    ledger
        .generated_paths
        .into_iter()
        .map(|relative| {
            validate_ledger_path(&relative)?;
            let path = output_root.join(relative);
            if !path.is_file() {
                anyhow::bail!(
                    "fixture snippet recorded by the coverage ledger is missing: {}",
                    path.display()
                );
            }
            Ok(path)
        })
        .collect()
}

fn validate_ledger_path(path: &std::path::Path) -> anyhow::Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        anyhow::bail!(
            "fixture snippet ledger path must be a non-empty relative path: {}",
            path.display()
        );
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_)
        )
    }) {
        anyhow::bail!(
            "fixture snippet ledger path escapes its output root: {}",
            path.display()
        );
    }
    Ok(())
}

fn parse_allowed_side_effects(configured: &[String]) -> anyhow::Result<Vec<crate::snippets::types::SideEffectClass>> {
    configured
        .iter()
        .map(|value| match value.as_str() {
            "safe" => Ok(crate::snippets::types::SideEffectClass::Safe),
            "network" => Ok(crate::snippets::types::SideEffectClass::Network),
            "process" => Ok(crate::snippets::types::SideEffectClass::Process),
            "install" => Ok(crate::snippets::types::SideEffectClass::Install),
            "server" => Ok(crate::snippets::types::SideEffectClass::Server),
            _ => anyhow::bail!("invalid docs.snippets.allowed_side_effects entry: `{value}`"),
        })
        .collect()
}

fn docs_sources(config: &ResolvedCrateConfig, configured_sources: &[PathBuf], workspace_root: &Path) -> Vec<PathBuf> {
    let sources = if configured_sources.is_empty() {
        config.source_hash_paths()
    } else {
        configured_sources.to_vec()
    };
    sources
        .into_iter()
        .map(|source| {
            if source.is_absolute() {
                source
            } else {
                workspace_root.join(source)
            }
        })
        .collect()
}

fn warn_missing_explicit_sources(kind: &str, sources: &[PathBuf], workspace_root: &Path) {
    let kind = kind.to_ascii_lowercase();
    for source in sources {
        if !workspace_root.join(source).exists() {
            tracing::warn!("docs.{kind} source does not exist, skipping: {}", source.display());
        }
    }
}

fn with_markdown_alef_header(content: &str) -> String {
    render::with_html_header(content.to_string())
}

#[cfg(test)]
mod coverage_manifest_tests {
    use super::*;
    use crate::core::config::e2e::{E2eConfig, SnippetConfig};
    use crate::e2e::fixture::SideEffectClass;
    use crate::e2e::snippets::{
        COVERAGE_MANIFEST_VERSION, GeneratedSnippetMetadata, SnippetCoverageKey, SnippetCoverageLedger,
    };

    #[test]
    fn current_ledger_paths_are_authoritative_documentation_references() {
        let root = tempfile::tempdir().expect("temporary workspace");
        let output = root.path().join("docs/snippets");
        let generated = output.join("python/topic/example.md");
        std::fs::create_dir_all(generated.parent().expect("generated parent")).expect("snippet directory");
        std::fs::write(&generated, "```python\nvalue = 1\n```\n").expect("generated snippet");
        let ledger = ledger(COVERAGE_MANIFEST_VERSION);
        std::fs::write(
            output.join(crate::e2e::snippets::COVERAGE_MANIFEST),
            serde_json::to_vec_pretty(&ledger).expect("coverage serializes"),
        )
        .expect("coverage manifest");
        let config = config();

        let references = ledger_generated_references(root.path(), &config, std::slice::from_ref(&output))
            .expect("ledger is current");

        assert_eq!(references, [generated]);
    }

    #[test]
    fn stale_ledger_version_is_rejected() {
        let root = tempfile::tempdir().expect("temporary workspace");
        let output = root.path().join("docs/snippets");
        std::fs::create_dir_all(&output).expect("snippet directory");
        std::fs::write(
            output.join(crate::e2e::snippets::COVERAGE_MANIFEST),
            serde_json::to_vec_pretty(&ledger(0)).expect("coverage serializes"),
        )
        .expect("coverage manifest");

        let error = ledger_generated_references(root.path(), &config(), std::slice::from_ref(&output))
            .expect_err("stale ledger must fail");

        assert!(
            error
                .to_string()
                .contains("stale fixture-snippet coverage manifest version")
        );
    }

    fn config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            e2e: Some(E2eConfig {
                snippets: Some(SnippetConfig {
                    output: "docs/snippets".into(),
                    languages: vec!["python".into()],
                    ..SnippetConfig::default()
                }),
                ..E2eConfig::default()
            }),
            ..ResolvedCrateConfig::default()
        }
    }

    fn ledger(format_version: u32) -> SnippetCoverageLedger {
        let key = SnippetCoverageKey {
            fixture_id: "example".into(),
            language: "python".into(),
        };
        SnippetCoverageLedger {
            format_version,
            generated_paths: vec![PathBuf::from("python/topic/example.md")],
            generated_metadata: vec![GeneratedSnippetMetadata {
                key: key.clone(),
                path: PathBuf::from("python/topic/example.md"),
                language: "python".into(),
                target: "python".into(),
                session: "python".into(),
                requires: Vec::new(),
                side_effect: SideEffectClass::Safe,
            }],
            expected: vec![key.clone()],
            generated: vec![key],
            missing: Vec::new(),
            documented_exceptions: Vec::new(),
        }
    }
}
