//! API reference documentation generator for alef polyglot bindings.
//!
//! Generates per-language `api-{lang}.md` files plus shared `configuration.md`
//! and `errors.md` files from the alef IR (`ApiSurface`).

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use heck::ToPascalCase;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod context;
mod descriptions;
pub mod doc_cleaning;
mod examples;
mod formatting;
pub(crate) mod language_pages;
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
/// Shared with `crate::readme` so README output gets the same self-embedded
/// HTML-comment marker as generated docs pages -- see `render::with_html_header`'s
/// own doc for why. ~keep
pub(crate) use render::with_html_header;

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
        // `Language::C` is an e2e consumer target, not a generated binding, and
        // `Language::Jni` is an internal Rust shim crate paired with `kotlin_android`
        // (see the `Language` variant docs in core/config/extras.rs) -- neither owns a
        // public API reference page of its own. `readme::generate_readme` already skips
        // both for the same reason. Both slug to `"c"` in `naming::lang_slug`, so
        // rendering a page for them here raced `Language::Ffi` -- the actual generated C
        // binding -- for `api-c.md`, and for `Jni` specifically the content was never
        // even the right ABI to begin with (see the long comment on
        // `examples::sample_param_value`'s Jni arm). ~keep
        if matches!(lang, Language::C | Language::Jni) {
            continue;
        }
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

/// The reference-docs output directory for `config` — `[docs].reference_output`
/// or the `docs/reference` default. Relative to the workspace root; callers join
/// it under the base directory.
///
/// Exposed so the generate pipeline can protect committed reference pages from
/// orphan cleanup: the page set `generate_docs_stage` emits depends on host
/// state (CLI/MCP source presence, doc languages), so a host that produces fewer
/// pages must not delete the committed ones it simply did not regenerate (#184). ~keep
pub fn reference_output_dir(config: &ResolvedCrateConfig) -> PathBuf {
    config
        .docs
        .as_ref()
        .and_then(|docs| docs.reference_output.clone())
        .unwrap_or_else(|| PathBuf::from("docs/reference"))
}

/// Generate the complete docs stage and hand back everything rendered, paired with whatever
/// error (if any) stopped it going further.
///
/// The `Vec<GeneratedFile>` is never dropped on failure: the 15+ API reference pages plus
/// `configuration.md`/`types.md`/`errors.md` are fully rendered before snippet discovery,
/// snippet validation, CLI/MCP adoption checks, or llms/skills rendering ever run, and every one
/// of those can fail for reasons that have nothing to do with the reference pages themselves (a
/// strict snippet-validation bail, an unmanaged `llms.txt`, a missing `docs.snippets.dirs` root).
/// Discarding the whole `Vec` on any of those used to mean a single strict-mode snippet failure
/// silently froze the *entire* published API reference at whatever version last validated
/// cleanly — worse than the failure it was trying to report, and with no signal to the caller
/// that anything was skipped. Callers must write `.0` unconditionally and only then propagate
/// `.1`.
///
/// That guarantee is not limited to the API reference pages: `generate_docs_stage_extras` orders
/// its steps so `cli.md`, `mcp.md`, `llms.txt` and every `SKILL.md` are emitted *before* snippet
/// validation runs, precisely so a strict bail cannot amputate the returned set. See that
/// function's doc for the false-orphan residue that ordering fixes. ~keep
pub fn generate_docs_stage(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_override: Option<&str>,
    workspace_root: &Path,
) -> (Vec<GeneratedFile>, anyhow::Result<()>) {
    let reference_output = output_override
        .map(PathBuf::from)
        .unwrap_or_else(|| reference_output_dir(config));
    let reference_output_str = reference_output.to_string_lossy().to_string();

    let mut files = match generate_docs(api, config, languages, &reference_output_str) {
        Ok(files) => files,
        Err(err) => return (Vec::new(), Err(err)),
    };
    for file in &mut files {
        file.content = with_markdown_alef_header(&file.content);
        file.generated_header = true;
    }

    let result = generate_docs_stage_extras(api, config, languages, workspace_root, &reference_output, &mut files);

    for file in &mut files {
        file.content = doc_cleaning::wrap_bare_urls(&file.content);
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    (files, result)
}

/// The one way a page enters the docs stage's emitted set.
///
/// `files` is not a convenience buffer, it is the record every downstream consumer of this stage
/// reads to decide what alef owns: `write_scaffold_files_report` writes it and records each
/// unmarkable path in `.alef-ownership.toml`, `alef all`/`alef docs` derive the orphan sweep's
/// `keep` set from it, and `bin_cli::helpers::collect_managed_surface` hands it to `alef verify`'s
/// frozen report and `alef adopt`'s candidate set. A page missing from it is indistinguishable, to
/// all of them, from a file alef has stopped emitting — an orphan.
///
/// Pairing the two appends in one function, instead of leaving each caller to remember a separate
/// `context.references.push`, is the point: the separate call is exactly the shape that left
/// `llms.txt` and every `SKILL.md` out of `references` while `cli.md`/`mcp.md` were in it. Passing
/// `None` is now a visible decision at the call site rather than a forgotten line. ~keep
fn emit_page(
    files: &mut Vec<GeneratedFile>,
    context: &mut DocsRenderContext,
    file: GeneratedFile,
    reference: Option<(&str, &str)>,
) {
    if let Some((kind, title)) = reference {
        context.references.push(context::ReferenceDoc {
            kind: kind.to_string(),
            title: title.to_string(),
            path: file.path.to_string_lossy().to_string(),
        });
    }
    files.push(file);
}

/// Everything past the API reference pages: CLI/MCP extraction and adoption checks, snippet
/// discovery, llms/skills rendering, and snippet validation. Takes `files` by mutable reference
/// specifically so an early `?` return here only unwinds this function — `generate_docs_stage`'s
/// `files` keeps every page pushed onto it before the failure. ~keep
///
/// THE ORDER OF THE STEPS BELOW IS LOAD-BEARING, and is the whole reason this function reads the
/// way it does: every step that *emits* a page runs before the fallible step that emits none.
/// Snippet validation used to run first (it was fused into `build_snippet_context`), so any
/// strict bail, gap failure or audit failure — none of which say anything about the CLI or MCP
/// surface — returned before `cli.md`, `mcp.md`, `llms.txt` and every `SKILL.md` were ever pushed.
/// `generate_docs_stage`'s "the Vec is never dropped on failure" guarantee then covered only the
/// API reference pages, and the pages that vanished were *silently* absent rather than reported:
/// downstream, absence from this Vec is read as "alef no longer emits this", so a validation
/// failure in one part of the docs stage manufactured false orphans in another. Two consumer
/// repos show exactly that residue — `cli.md`/`mcp.md`/`SKILL.md` present on disk, alef-marked,
/// and absent from `.alef-ownership.toml` while every version-bearing API page is listed.
///
/// Anything fallible added here must go after the last `emit_page`, or be an emitter itself. ~keep
fn generate_docs_stage_extras(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    workspace_root: &Path,
    reference_output: &Path,
    files: &mut Vec<GeneratedFile>,
) -> anyhow::Result<()> {
    let mut context = build_base_context(api, config, languages, files.as_slice());
    let Some(docs_cfg) = &config.docs else {
        return Ok(());
    };

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
            let page = render::generate_cli_doc(&surface, path);
            emit_page(files, &mut context, page, Some(("cli", "CLI Reference")));
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
            let page = render::generate_mcp_doc(&surface, path);
            emit_page(files, &mut context, page, Some(("mcp", "MCP Reference")));
            context.mcp = surface;
        }
    }

    let snippets = build_snippet_context(config, workspace_root, &mut context)?;
    let snippet_dirs: &[PathBuf] = snippets.as_ref().map_or(&[], |stage| stage.dirs.as_slice());

    if let Some(llms_cfg) = &docs_cfg.llms {
        let page = render::render_llms(llms_cfg, &context, workspace_root, snippet_dirs)?;
        emit_page(files, &mut context, page, None);
    }

    if let Some(skills_cfg) = &docs_cfg.skills {
        let pages = render::render_skills(skills_cfg, &context, workspace_root, snippet_dirs)?;
        for page in pages {
            emit_page(files, &mut context, page, None);
        }
    }

    if let Some(stage) = &snippets {
        validate_snippets(
            config,
            workspace_root,
            stage.config,
            &stage.absolute_dirs,
            &stage.snippets,
        )?;
    }

    Ok(())
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

/// The discovered snippet corpus, carried from discovery to validation.
///
/// Discovery has to happen before `llms.txt`/`SKILL.md` render (it populates `context.snippets`
/// and supplies the roots the `include_snippet` filter searches); validation must happen after
/// they have been emitted. Holding the corpus in one value is what lets the two sit on opposite
/// sides of the emitting steps without discovering twice. ~keep
struct SnippetStage<'cfg> {
    config: &'cfg crate::core::config::DocsSnippetsConfig,
    dirs: Vec<PathBuf>,
    absolute_dirs: Vec<PathBuf>,
    snippets: Vec<crate::snippets::types::Snippet>,
}

/// Discover the configured snippet corpus and record it in `context`.
///
/// `Ok(None)` means there is nothing to validate later either — no `docs.snippets` section, or a
/// section with no discovery roots at all. Both cases previously returned an empty dir list and
/// skipped validation; returning `None` keeps that coupling explicit instead of leaving the
/// deferred `validate_snippets` call to re-derive it from an empty vector. ~keep
fn build_snippet_context<'cfg>(
    config: &'cfg ResolvedCrateConfig,
    workspace_root: &Path,
    context: &mut DocsRenderContext,
) -> anyhow::Result<Option<SnippetStage<'cfg>>> {
    let Some(snippet_cfg) = config.docs.as_ref().and_then(|docs| docs.snippets.as_ref()) else {
        return Ok(None);
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
        return Ok(None);
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

    Ok(Some(SnippetStage {
        config: snippet_cfg,
        dirs: snippet_dirs,
        absolute_dirs: absolute_snippet_dirs,
        snippets,
    }))
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
    configured_references.extend(crate::snippets::gaps::coverage_ledger_references(
        absolute_snippet_dirs,
    )?);
    let content_collections = snippet_cfg
        .content_collections
        .iter()
        .map(|(name, root)| (name.clone(), workspace_root.join(root)))
        .collect();
    configured_references.extend(crate::snippets::gaps::astro_collection_references(
        &docs_dirs,
        &content_collections,
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
                    let mut rust_features = session.rust_features.clone();
                    if language == crate::snippets::types::Language::Rust {
                        rust_features.extend(config.features.iter().cloned());
                        rust_features.sort();
                        rust_features.dedup();
                    }
                    Ok((
                        normalized,
                        crate::snippets::session::SessionSpec {
                            language,
                            working_directory: workspace_root.join(&session.cwd),
                            manifest: session.manifest.as_ref().map(|path| workspace_root.join(path)),
                            before: session.before.clone(),
                            env: session.env.clone(),
                            include_paths: session
                                .include_paths
                                .iter()
                                .map(|path| workspace_root.join(path))
                                .collect(),
                            rust_features,
                            rust_dependencies: session.rust_dependencies.clone(),
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
        // Write the report before any strict bail. A run that fails strict mode is precisely the
        // run whose report a consumer needs, and emitting it afterwards meant the artifact was
        // never produced in that case. ~keep
        if let Some(path) = &snippet_cfg.report_output {
            let report_path = workspace_root.join(path);
            crate::snippets::output::write_report(&summary, &report_path, false).map_err(|err| {
                anyhow::anyhow!(
                    "writing snippet validation report to '{}': {err}",
                    report_path.display()
                )
            })?;
        }
        enforce_snippet_summary(&config.name, snippet_cfg.strict, &summary)?;
    }

    Ok(())
}

/// Decides whether a completed snippet validation run fails the crate, and how. Order matters:
/// hard failures (`Fail`/`Error`, which includes session/preparation errors — see
/// `session::prepare_sessions_isolated`) bail before a strict downgraded bail. A run can carry
/// both hundreds of downgrades and hundreds of real failures at once, and the strict downgraded
/// check used to run — and bail — before the failure check further down was ever reached, so a
/// consumer investigating "N downgraded" never learned the run had failed outright. A user must
/// never be told "downgraded" when something actually failed. Factored out of `validate_snippets`
/// so this ordering is directly testable against a constructed `RunSummary`, without a real
/// toolchain or filesystem — see `docs::tests::strict_bail_order`. ~keep
fn enforce_snippet_summary(
    crate_name: &str,
    strict: bool,
    summary: &crate::snippets::types::RunSummary,
) -> anyhow::Result<()> {
    if summary.unavailable > 0 && strict {
        let toolchain_missing = summary.unavailable - summary.unresolved_dependency;
        anyhow::bail!(
            "strict snippet validation failed for crate `{}`: {} unavailable ({} unresolved dependency, {} \
             toolchain missing){}",
            crate_name,
            summary.unavailable,
            summary.unresolved_dependency,
            toolchain_missing,
            attribute_unavailable(summary)
        );
    }
    if summary.capability_capped > 0 {
        tracing::warn!(
            capped = summary.capability_capped,
            "docs.snippets validated {} snippet(s) below the requested level because their validator caps lower; \
             these pass strict mode, because the level is unreachable for that language rather than degraded{}",
            summary.capability_capped,
            attribute_capability_capped(summary)
        );
    }
    // `declared_capped` results pass exactly like `capability_capped` ones do, and for the same
    // structural reason this function already warns about the latter: a bare "N passed" leaves a
    // consumer with no way to learn that `docs.snippets.validation_level` was not actually applied
    // to every snippet. The gap this closes is concrete, not hypothetical: `alef e2e generate`
    // stamps every fixture snippet's front matter with `level: typecheck` (see
    // `e2e::snippets::render_snippet_markdown`), which silently caps `validation_level = "run"`
    // down to typecheck for all of them — previously reported as an unqualified `Pass` with no
    // trace anywhere in this pipeline's output. ~keep
    if summary.declared_capped > 0 {
        tracing::warn!(
            declared_capped = summary.declared_capped,
            "docs.snippets validated {} snippet(s) below the requested level because their own front-matter \
             `level:` declares a lower ceiling; these pass strict mode as a satisfied per-snippet contract, \
             but the requested level was not actually applied to them{}",
            summary.declared_capped,
            attribute_declared_capped(summary)
        );
    }
    if summary.unavailable > 0 {
        let toolchain_missing = summary.unavailable - summary.unresolved_dependency;
        tracing::warn!(
            unavailable = summary.unavailable,
            unresolved_dependency = summary.unresolved_dependency,
            toolchain_missing,
            "docs.snippets skipped {} snippet validation(s) because required toolchains were unavailable ({} \
             unresolved dependency, {} toolchain missing){}",
            summary.unavailable,
            summary.unresolved_dependency,
            toolchain_missing,
            attribute_unavailable(summary)
        );
    }
    if summary.has_failures() {
        anyhow::bail!(
            "snippet validation failed for crate `{}`: {} failed, {} errors{}{}",
            crate_name,
            summary.failed,
            summary.errors,
            attribute_results(summary, crate::snippets::types::SnippetStatus::Fail),
            attribute_results(summary, crate::snippets::types::SnippetStatus::Error)
        );
    }
    if summary.downgraded > 0 && strict {
        anyhow::bail!(
            "strict snippet validation failed for crate `{}`: {} validation(s) downgraded{}",
            crate_name,
            summary.downgraded,
            attribute_results(summary, crate::snippets::types::SnippetStatus::Downgraded)
        );
    }
    Ok(())
}

/// Human label for why a result's effective level differs from what was requested. Kept
/// alongside the attribution formatting it feeds — `DowngradeReason` itself stays a plain data
/// enum with no presentation concerns. ~keep
fn downgrade_reason_label(reason: crate::snippets::types::DowngradeReason) -> &'static str {
    use crate::snippets::types::DowngradeReason;
    match reason {
        DowngradeReason::Declared => "author declared this level via front matter",
        DowngradeReason::Annotation => "author suppressed via annotation",
        DowngradeReason::ValidatorCapability => "validator cannot reach this level",
        DowngradeReason::Environment => "environment could not reach this level",
    }
}

/// Name the snippets behind a strict-mode count so the failure is actionable.
///
/// A bare total ("261 validation(s) downgraded") gives a consumer no entry point: the achieved
/// level is not recorded in the emitted snippet frontmatter, so there is no other way to learn
/// which snippets regressed or from what level. Groups by language and bounds the per-language
/// sample, so a large run stays readable while still naming concrete ids to start from. Within a
/// language, also breaks the count down by `downgrade_reason` — "author declared this level" and
/// "environment could not reach this level" call for entirely different fixes, and collapsing
/// them into one count told a consumer nothing about which one applies. ~keep
fn attribute_results(
    summary: &crate::snippets::types::RunSummary,
    status: crate::snippets::types::SnippetStatus,
) -> String {
    attribute(summary, |result| result.status == status)
}

/// Same attribution, for the `Pass` results a validator's declared or structural ceiling capped
/// below the requested level. These never fail strict, but a consumer watching the summary count
/// climb still needs to know which snippets and why. ~keep
fn attribute_capability_capped(summary: &crate::snippets::types::RunSummary) -> String {
    attribute(summary, |result| result.capability_capped)
}

/// Same attribution, for the `Pass` results a snippet's own front-matter `level:` contract capped
/// below the requested level (`DowngradeReason::Declared`). These never fail strict either, but
/// unlike `capability_capped` this ceiling came from the snippet's content, not its language — a
/// consumer needs to know that too, especially when the "author" is a code generator that stamped
/// the same declared level onto every snippet it emitted rather than a person choosing it. ~keep
fn attribute_declared_capped(summary: &crate::snippets::types::RunSummary) -> String {
    attribute(summary, |result| {
        result.downgrade_reason == Some(crate::snippets::types::DowngradeReason::Declared)
    })
}

/// Per-language counts for `Unavailable` results, split by cause. Deliberately not
/// `attribute_results`: the remediation for `unresolved_dependency` is "run `alef build`" and for
/// a plain toolchain gap is "install the toolchain", and both apply to the whole language batch,
/// not to one snippet — three sample ids and a `+N more` told a consumer nothing a count didn't
/// already, since the fix is the same for every snippet in the batch. This also sidesteps two bugs
/// `attribute_results` had for this status: its `[reasons]` bracket reads `downgrade_reason`,
/// which is `None` for every `Unavailable` result by construction (see the `debug_assert!` in
/// `runner::finalize_result`), so the bracket was always empty; and its `(requested -> effective)`
/// arrow implied a level downgrade that never happened here — `Unavailable` results carry no
/// downgrade at all. ~keep
fn attribute_unavailable(summary: &crate::snippets::types::RunSummary) -> String {
    #[derive(Default)]
    struct LanguageCounts {
        unresolved_dependency: usize,
        toolchain_missing: usize,
    }

    let mut by_language: BTreeMap<String, LanguageCounts> = BTreeMap::new();
    for result in summary
        .results
        .iter()
        .filter(|result| result.status == crate::snippets::types::SnippetStatus::Unavailable)
    {
        let entry = by_language.entry(result.snippet.language.to_string()).or_default();
        if result.unresolved_dependency {
            entry.unresolved_dependency += 1;
        } else {
            entry.toolchain_missing += 1;
        }
    }
    if by_language.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for (language, counts) in by_language {
        out.push_str(&format!(
            "\n  {language}: {} unresolved dependency, {} toolchain missing",
            counts.unresolved_dependency, counts.toolchain_missing
        ));
    }
    out
}

fn attribute(
    summary: &crate::snippets::types::RunSummary,
    matches: impl Fn(&crate::snippets::types::ValidationResult) -> bool,
) -> String {
    const SAMPLE_PER_LANGUAGE: usize = 3;

    #[derive(Default)]
    struct LanguageGroup {
        count: usize,
        reasons: BTreeMap<&'static str, usize>,
        sample: Vec<String>,
    }

    let mut by_language: BTreeMap<String, LanguageGroup> = BTreeMap::new();
    for result in summary.results.iter().filter(|result| matches(result)) {
        let entry = by_language.entry(result.snippet.language.to_string()).or_default();
        entry.count += 1;
        if let Some(reason) = result.downgrade_reason {
            *entry.reasons.entry(downgrade_reason_label(reason)).or_default() += 1;
        }
        if entry.sample.len() < SAMPLE_PER_LANGUAGE {
            let id = result.snippet.id.clone().unwrap_or_else(|| {
                format!(
                    "{}:{}",
                    result.snippet.source_origin.path.display(),
                    result.snippet.source_origin.line
                )
            });
            entry.sample.push(format!(
                "{id} ({} -> {})",
                result.requested_level, result.effective_level
            ));
        }
    }
    if by_language.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for (language, group) in by_language {
        let remainder = group.count.saturating_sub(group.sample.len());
        let suffix = if remainder > 0 {
            format!(", +{remainder} more")
        } else {
            String::new()
        };
        let reasons = group
            .reasons
            .iter()
            .map(|(label, count)| format!("{label}: {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let reason_suffix = if reasons.is_empty() {
            String::new()
        } else {
            format!(" [{reasons}]")
        };
        out.push_str(&format!(
            "\n  {language}: {}{reason_suffix} -- {}{suffix}",
            group.count,
            group.sample.join(", ")
        ));
    }
    out
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
    render::with_html_header(content.to_string(), "alef docs")
}
