//! `alef snippets` subcommand — discover, validate, audit, and gap-check documentation snippets.

use crate::snippets::audit::{AuditConfig, AuditSeverity, audit};
use crate::snippets::discovery;
use crate::snippets::gaps::{GapConfig, detect_gaps};
use crate::snippets::output;
use crate::snippets::runner::{RunnerConfig, run_validation};
use crate::snippets::session::SessionSpec;
use crate::snippets::types::{Language, SideEffectClass, SnippetStatus, ValidationLevel};
use crate::snippets::validators::ValidatorRegistry;
use clap::Subcommand;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Subcommand)]
pub enum SnippetsAction {
    /// List discovered snippets and a per-language count summary.
    List {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short, long, value_delimiter = ',')]
        languages: Option<Vec<String>>,
    },

    /// Run the configured snippet discovery, validation, audit, and gap checks.
    Check {
        #[arg(short, long, default_value = "alef.toml")]
        config: PathBuf,
        #[arg(long)]
        strict: bool,
        #[arg(long, default_value = "on", value_parser = ["on", "off"])]
        cache: String,
    },

    /// Parse a single file and print its code blocks.
    Parse { file: PathBuf },

    /// Structural integrity audit (frontmatter, fences, include targets).
    Audit {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short, long, num_args = 0..)]
        docs: Vec<PathBuf>,

        #[arg(long)]
        require_frontmatter: bool,
    },

    /// Coverage gap report (unreferenced snippets, missing language variants).
    Gaps {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short, long, num_args = 0..)]
        docs: Vec<PathBuf>,

        #[arg(short = 'L', long, value_delimiter = ',')]
        required_languages: Option<Vec<String>>,

        /// Additional base paths to search when resolving `--8<--` include targets.
        ///
        /// Mirrors the `pymdownx.snippets` `base_path` list. Each target is
        /// resolved against these paths in order; the first match wins. When
        /// unset, only the docs root is searched (preserving the prior behaviour).
        #[arg(long = "include-base-path", num_args = 0..)]
        include_base_paths: Vec<PathBuf>,
    },
}

pub fn run(action: SnippetsAction) -> ExitCode {
    match action {
        SnippetsAction::List { snippets, languages } => run_list(&snippets, languages.as_ref()),
        SnippetsAction::Check { config, strict, cache } => run_check(&config, strict, cache != "off"),
        SnippetsAction::Parse { file } => run_parse(&file),
        SnippetsAction::Audit {
            snippets,
            docs,
            require_frontmatter,
        } => run_audit(&snippets, &docs, require_frontmatter),
        SnippetsAction::Gaps {
            snippets,
            docs,
            required_languages,
            include_base_paths,
        } => run_gaps(&snippets, &docs, required_languages.as_ref(), &include_base_paths),
    }
}

fn parse_language_filter(languages: Option<&[String]>) -> Option<Vec<Language>> {
    let languages = languages?;
    Some(
        languages
            .iter()
            .map(|language| Language::from_fence_tag(language))
            .filter(|language| *language != Language::Unknown)
            .collect(),
    )
}

fn run_list(snippets: &[PathBuf], languages: Option<&Vec<String>>) -> ExitCode {
    let filter = parse_language_filter(languages.map(Vec::as_slice));
    match discovery::discover_snippets(snippets, filter.as_deref()) {
        Ok(found) => {
            output::print_snippet_list(&found);
            crate::bin_cli::output::blank();
            for (language, count) in &discovery::count_by_language(&found) {
                crate::bin_cli::output::line(format!("  {language:<12} {count}"));
            }
            crate::bin_cli::output::blank();
            ExitCode::SUCCESS
        }
        Err(err) => {
            tracing::error!("discovering snippets: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_check(config_path: &Path, force_strict: bool, use_cache: bool) -> ExitCode {
    let (_, resolved) = match crate::bin_cli::helpers::load_config(config_path) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!("loading snippet config: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some((crate_config, config)) = resolved
        .iter()
        .find_map(|krate| Some((krate, krate.docs.as_ref()?.snippets.as_ref()?)))
    else {
        tracing::error!("no [workspace.docs.snippets] or [crates.docs.snippets] configuration found");
        return ExitCode::FAILURE;
    };
    let root = config_path.parent().unwrap_or_else(|| Path::new("."));
    let excluded_paths: Vec<PathBuf> = config.exclude.iter().map(|excluded| root.join(excluded)).collect();
    let snippet_directories = resolved_roots(root, &config.dirs, &excluded_paths);
    let mut directories = snippet_directories.clone();
    directories.extend(resolved_roots(root, &config.inline_dirs, &excluded_paths));
    let docs_directories: Vec<PathBuf> = config.docs_dirs.iter().map(|path| root.join(path)).collect();
    let include_base_paths: Vec<PathBuf> = if config.include_base_paths.is_empty() {
        docs_directories.clone()
    } else {
        config.include_base_paths.iter().map(|path| root.join(path)).collect()
    };
    let required_languages = match config
        .required_languages
        .iter()
        .map(|language| language.parse::<Language>())
        .collect::<Result<Vec<Language>, _>>()
    {
        Ok(languages) => languages,
        Err(error) => {
            tracing::error!("invalid docs.snippets.required_languages entry: {error}");
            return ExitCode::FAILURE;
        }
    };
    let level = config
        .validation_level
        .as_deref()
        .unwrap_or("syntax")
        .parse::<ValidationLevel>()
        .unwrap_or(ValidationLevel::Syntax);
    let strict = force_strict || config.strict;
    let found = match discovery::discover_snippets(&directories, None) {
        Ok(found) if !found.is_empty() => found,
        Ok(_) => {
            tracing::error!("snippet discovery returned no snippets");
            return ExitCode::FAILURE;
        }
        Err(error) => {
            tracing::error!("discovering configured snippets: {error}");
            return ExitCode::FAILURE;
        }
    };
    let allowed_side_effects = config
        .allowed_side_effects
        .iter()
        .filter_map(|value| parse_side_effect(value))
        .collect();
    let runner = RunnerConfig {
        level,
        parallelism: std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get),
        timeout_secs: config.timeout_secs.unwrap_or(120),
        fail_fast: config.fail_fast,
        deny_unclassified: config.deny_unclassified || force_strict,
        allowed_side_effects,
        cache_dir: use_cache.then(|| root.join(config.cache_dir())),
        changed_only: use_cache,
        sessions: match configured_sessions(config, root, &crate_config.features) {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::error!("{error}");
                return ExitCode::FAILURE;
            }
        },
    };
    let summary = match run_validation(&found, &ValidatorRegistry::new(), &runner) {
        Ok(summary) => summary,
        Err(error) => {
            tracing::error!("running configured snippet validation: {error}");
            return ExitCode::FAILURE;
        }
    };
    output::print_summary(&summary, false);
    if let Some(path) = &config.report_output
        && let Err(error) = output::write_report(&summary, &root.join(path), false)
    {
        tracing::error!("writing snippet report: {error}");
        return ExitCode::FAILURE;
    }
    let strict_failure = strict && has_incomplete_coverage(&summary);
    let missing_generated = match missing_generated_snippets(&directories) {
        Ok(missing) => missing,
        Err(error) => {
            tracing::error!("reading generated snippet coverage: {error}");
            return ExitCode::FAILURE;
        }
    };
    for missing in &missing_generated {
        tracing::warn!(
            "generated snippet missing for fixture `{}` language `{}`: {}",
            missing.key.fixture_id,
            missing.key.language,
            missing.reason
        );
    }
    let content_collections: std::collections::BTreeMap<String, PathBuf> = config
        .content_collections
        .iter()
        .map(|(name, collection_root)| (name.clone(), root.join(collection_root)))
        .collect();
    let (audit_failure, gap_failure) = match run_configured_audit_and_gaps(&ConfiguredCheckInputs {
        snippet_directories: &snippet_directories,
        docs_directories: &docs_directories,
        include_base_paths: &include_base_paths,
        required_languages: &required_languages,
        exclude: &excluded_paths,
        readme: crate_config.readme.as_ref(),
        content_collections: &content_collections,
        workspace_root: root,
        require_frontmatter: config.require_frontmatter,
        strict,
    }) {
        Ok(result) => result,
        Err(error) => {
            tracing::error!("running configured snippet audit and gap checks: {error}");
            return ExitCode::FAILURE;
        }
    };
    if summary.has_failures()
        || strict_failure
        || strict && !missing_generated.is_empty()
        || audit_failure
        || gap_failure
    {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Resolve configured snippet roots against `root`, dropping any that fall
/// under an excluded prefix.
fn resolved_roots(root: &Path, dirs: &[PathBuf], excluded: &[PathBuf]) -> Vec<PathBuf> {
    dirs.iter()
        .map(|path| root.join(path))
        .filter(|path| !excluded.iter().any(|prefix| path.starts_with(prefix)))
        .collect()
}

/// Inputs for `check`'s configured audit and gap pass, grouped into one
/// struct so the call stays under clippy's argument threshold.
struct ConfiguredCheckInputs<'a> {
    /// Snippet roots proper — `docs.snippets.dirs` only. `inline_dirs` are
    /// deliberately absent: they are prose documentation pages whose fenced
    /// blocks are validated as snippets, and they are never `--8<--` include
    /// targets, so gap-checking them would report every documentation page as
    /// an unreferenced snippet. Mirrors `docs/mod.rs::validate_snippets`,
    /// which likewise audits and gap-checks only the `dirs`-derived list.
    snippet_directories: &'a [PathBuf],
    docs_directories: &'a [PathBuf],
    include_base_paths: &'a [PathBuf],
    required_languages: &'a [Language],
    exclude: &'a [PathBuf],
    readme: Option<&'a crate::core::config::ReadmeConfig>,
    /// Astro content collection names mapped to their already-resolved roots.
    content_collections: &'a std::collections::BTreeMap<String, PathBuf>,
    workspace_root: &'a Path,
    require_frontmatter: bool,
    strict: bool,
}

/// Run the configured audit and gap checks against the configured snippet
/// roots, so `check` cannot disagree with `alef validate` about which files
/// are in scope.
///
/// References a snippet can legitimately have without any `--8<--` include
/// are collected from the same three sources as
/// `docs/mod.rs::validate_snippets`: `[crates.readme]` snippet mappings,
/// generated-snippet coverage ledgers, and Astro content collections queried
/// by a documentation page.
///
/// Audit issues of `AuditSeverity::Error` always fail the gate, and audit is
/// skipped entirely without a configured docs surface (matching the
/// precedent) so a snippets-only config is not failed by fence tags no
/// documentation ever renders. Gap findings split the same way `alef
/// validate`'s snippet gate already treats them: unreferenced snippets are
/// only a failure under `strict` (extra examples can be intentional), while
/// missing include targets, missing required language variants, undocumented
/// skips, and unknown fence languages always fail. Gaps are skipped entirely
/// when neither `docs_dirs` nor `required_languages` is configured —
/// otherwise every discovered snippet would read as "unreferenced" and a
/// `strict` config with no docs surface configured would flip from green to
/// red for a check that was never meaningful for it.
///
/// Coverage ledgers are read with missing fixture/language cells tolerated:
/// `run_check` already reports those through `missing_generated_snippets` and
/// only fails on them under `strict`, so rejecting them here would both
/// override that gate and misattribute the failure.
///
/// # Errors
///
/// Returns an error when a coverage ledger is broken, an Astro collection
/// root cannot be walked, or a documentation file cannot be read.
fn run_configured_audit_and_gaps(inputs: &ConfiguredCheckInputs<'_>) -> anyhow::Result<(bool, bool)> {
    let mut configured_references =
        crate::snippets::gaps::readme_snippet_references(inputs.workspace_root, inputs.readme);
    configured_references
        .extend(crate::snippets::gaps::coverage_ledger_references_allowing_missing_cells(inputs.snippet_directories)?);
    configured_references.extend(crate::snippets::gaps::astro_collection_references(
        inputs.docs_directories,
        inputs.content_collections,
    )?);

    let audit_failure = if inputs.docs_directories.is_empty() {
        false
    } else {
        report_audit(&audit(&AuditConfig {
            docs_dirs: inputs.docs_directories.to_vec(),
            snippet_dirs: inputs.snippet_directories.to_vec(),
            require_frontmatter: inputs.require_frontmatter,
            include_base_paths: inputs.include_base_paths.to_vec(),
            configured_references: configured_references.clone(),
            exclude: inputs.exclude.to_vec(),
        }))
    };

    if inputs.docs_directories.is_empty() && inputs.required_languages.is_empty() {
        return Ok((audit_failure, false));
    }
    let gap_report = detect_gaps(&GapConfig {
        docs_dirs: inputs.docs_directories.to_vec(),
        snippet_dirs: inputs.snippet_directories.to_vec(),
        required_languages: inputs.required_languages.to_vec(),
        include_base_paths: inputs.include_base_paths.to_vec(),
        configured_references,
        exclude: inputs.exclude.to_vec(),
    })?;
    let (gap_structural_failure, gap_has_unreferenced) = report_gaps(&gap_report);
    Ok((
        audit_failure,
        gap_structural_failure || (inputs.strict && gap_has_unreferenced),
    ))
}

fn report_audit(report: &crate::snippets::audit::AuditReport) -> bool {
    for issue in &report.issues {
        let message = format!(
            "snippet audit: {}:{} ({:?}) {}",
            issue.path.display(),
            issue.line,
            issue.kind,
            issue.message
        );
        match issue.severity {
            AuditSeverity::Error => tracing::error!("{message}"),
            AuditSeverity::Warning => tracing::warn!("{message}"),
        }
    }
    report.has_errors()
}

/// Logs every gap finding and returns `(structural_failure, has_unreferenced_snippets)`.
///
/// `structural_failure` covers missing include targets, missing required
/// language variants, undocumented skips, and unknown fence languages —
/// findings that are never intentional. Unreferenced snippets are reported
/// separately because they are only a failure under `strict`.
fn report_gaps(report: &crate::snippets::gaps::GapReport) -> (bool, bool) {
    for reference in &report.missing_references {
        tracing::error!(
            "snippet gap: missing include target {}:{} -> {}",
            reference.source.display(),
            reference.line,
            reference.target.display()
        );
    }
    for path in &report.unreferenced_snippets {
        tracing::warn!("snippet gap: unreferenced snippet {}", path.display());
    }
    for variant in &report.missing_language_variants {
        tracing::error!(
            "snippet gap: missing required language variant `{}` for {}",
            variant.language,
            variant.group.display()
        );
    }
    for location in &report.skips_without_reason {
        tracing::error!(
            "snippet gap: skip without reason {}:{} (block {})",
            location.path.display(),
            location.line,
            location.block_index
        );
    }
    for unknown in &report.unknown_languages {
        tracing::error!(
            "snippet gap: unknown fence language {}:{} tag=`{}`",
            unknown.path.display(),
            unknown.line,
            unknown.tag
        );
    }
    let structural_failure = !report.missing_references.is_empty()
        || !report.missing_language_variants.is_empty()
        || !report.skips_without_reason.is_empty()
        || !report.unknown_languages.is_empty();
    (structural_failure, !report.unreferenced_snippets.is_empty())
}

fn configured_sessions(
    config: &crate::core::config::DocsSnippetsConfig,
    root: &std::path::Path,
    crate_features: &[String],
) -> Result<std::collections::HashMap<String, SessionSpec>, String> {
    let root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("resolving current directory for snippet sessions: {error}"))?
            .join(root)
    };
    let mut sessions = std::collections::HashMap::new();
    for (target, session) in &config.sessions {
        let normalized = Language::normalize_session_target(target);
        let language = Language::from_session_target(&normalized);
        if language == Language::Unknown {
            return Err(format!("unknown docs.snippets session target `{target}`"));
        }
        let mut rust_features = session.rust_features.clone();
        if language == Language::Rust {
            rust_features.extend(crate_features.iter().cloned());
            rust_features.sort();
            rust_features.dedup();
        }
        let spec = SessionSpec {
            language,
            working_directory: root.join(&session.cwd),
            manifest: session.manifest.as_ref().map(|path| root.join(path)),
            before: session.before.clone(),
            env: session.env.clone(),
            include_paths: session.include_paths.iter().map(|path| root.join(path)).collect(),
            rust_features,
            rust_dependencies: session.rust_dependencies.clone(),
        };
        if sessions.insert(normalized.clone(), spec).is_some() {
            return Err(format!("duplicate docs.snippets session target `{normalized}`"));
        }
    }
    Ok(sessions)
}

fn missing_generated_snippets(directories: &[PathBuf]) -> anyhow::Result<Vec<crate::e2e::snippets::MissingSnippet>> {
    let mut missing = Vec::new();
    for directory in directories {
        let path = directory.join(crate::e2e::snippets::COVERAGE_MANIFEST);
        if !path.is_file() {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
        let ledger: crate::e2e::snippets::SnippetCoverageLedger = serde_json::from_str(&content)
            .map_err(|error| anyhow::anyhow!("failed to parse {}: {error}", path.display()))?;
        missing.extend(ledger.missing);
    }
    missing.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(missing)
}

fn has_incomplete_coverage(summary: &crate::snippets::types::RunSummary) -> bool {
    summary.results.iter().any(|result| is_incomplete_status(result.status))
}

fn is_incomplete_status(status: SnippetStatus) -> bool {
    matches!(
        status,
        SnippetStatus::Skip | SnippetStatus::Unavailable | SnippetStatus::Downgraded
    )
}

fn parse_side_effect(value: &str) -> Option<SideEffectClass> {
    match value.trim().to_ascii_lowercase().as_str() {
        "safe" => Some(SideEffectClass::Safe),
        "network" => Some(SideEffectClass::Network),
        "process" => Some(SideEffectClass::Process),
        "install" => Some(SideEffectClass::Install),
        "server" => Some(SideEffectClass::Server),
        _ => None,
    }
}

fn run_parse(file: &Path) -> ExitCode {
    match crate::snippets::parser::parse_code_blocks(file) {
        Ok(blocks) => {
            if blocks.is_empty() {
                crate::bin_cli::output::line(format!("No code blocks found in {}", file.display()));
            } else {
                for (index, block) in blocks.iter().enumerate() {
                    crate::bin_cli::output::line(format!("--- Block {} (line {}) ---", index + 1, block.start_line));
                    crate::bin_cli::output::line(format!("Language: {}", block.lang));
                    if let Some(title) = &block.title {
                        crate::bin_cli::output::line(format!("Title: {title}"));
                    }
                    if let Some(comment) = &block.preceding_comment {
                        crate::bin_cli::output::line(format!("Annotation: {comment}"));
                    }
                    crate::bin_cli::output::line(format!("Code ({} lines):", block.code.lines().count()));
                    crate::bin_cli::output::line(&block.code);
                    crate::bin_cli::output::blank();
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            tracing::error!("parsing {}: {err}", file.display());
            ExitCode::FAILURE
        }
    }
}

fn run_audit(snippet_dirs: &[PathBuf], docs_dirs: &[PathBuf], require_frontmatter: bool) -> ExitCode {
    let configured_references = match crate::snippets::gaps::coverage_ledger_references(snippet_dirs) {
        Ok(references) => references,
        Err(error) => {
            tracing::error!("reading generated snippet coverage: {error}");
            return ExitCode::FAILURE;
        }
    };
    let config = AuditConfig {
        docs_dirs: docs_dirs.to_vec(),
        snippet_dirs: snippet_dirs.to_vec(),
        require_frontmatter,
        include_base_paths: docs_dirs.to_vec(),
        configured_references,
        exclude: Vec::new(),
    };
    let report = audit(&config);
    if report.issues.is_empty() {
        crate::bin_cli::output::line("Audit clean: no issues found.");
        return ExitCode::SUCCESS;
    }
    crate::bin_cli::output::line(format!("Audit found {} issue(s):", report.issues.len()));
    for issue in &report.issues {
        let severity = match issue.severity {
            AuditSeverity::Error => "ERROR",
            AuditSeverity::Warning => "WARN",
        };
        crate::bin_cli::output::line(format!(
            "  [{severity}] {}:{} ({:?}) {}",
            issue.path.display(),
            issue.line,
            issue.kind,
            issue.message
        ));
    }
    if report.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run_gaps(
    snippet_dirs: &[PathBuf],
    docs_dirs: &[PathBuf],
    required_languages: Option<&Vec<String>>,
    include_base_paths: &[PathBuf],
) -> ExitCode {
    let required = required_languages
        .map(|languages| {
            languages
                .iter()
                .map(|language| Language::from_fence_tag(language))
                .filter(|language| *language != Language::Unknown)
                .collect()
        })
        .unwrap_or_default();
    let resolved_base_paths: Vec<PathBuf> = if include_base_paths.is_empty() {
        docs_dirs.to_vec()
    } else {
        include_base_paths.to_vec()
    };
    let configured_references = match crate::snippets::gaps::coverage_ledger_references(snippet_dirs) {
        Ok(references) => references,
        Err(error) => {
            tracing::error!("reading generated snippet coverage: {error}");
            return ExitCode::FAILURE;
        }
    };
    let config = GapConfig {
        docs_dirs: docs_dirs.to_vec(),
        snippet_dirs: snippet_dirs.to_vec(),
        required_languages: required,
        include_base_paths: resolved_base_paths,
        configured_references,
        exclude: Vec::new(),
    };
    let report = match detect_gaps(&config) {
        Ok(report) => report,
        Err(err) => {
            tracing::error!("detecting gaps: {err}");
            return ExitCode::FAILURE;
        }
    };
    if !report.has_gaps() {
        crate::bin_cli::output::line("No gaps found.");
        return ExitCode::SUCCESS;
    }
    if !report.missing_references.is_empty() {
        crate::bin_cli::output::line(format!(
            "Missing include targets ({}):",
            report.missing_references.len()
        ));
        for reference in &report.missing_references {
            crate::bin_cli::output::line(format!(
                "  {}:{} → {}",
                reference.source.display(),
                reference.line,
                reference.target.display()
            ));
        }
    }
    if !report.unreferenced_snippets.is_empty() {
        crate::bin_cli::output::line(format!(
            "Unreferenced snippets ({}):",
            report.unreferenced_snippets.len()
        ));
        for path in &report.unreferenced_snippets {
            crate::bin_cli::output::line(format!("  {}", path.display()));
        }
    }
    if !report.missing_language_variants.is_empty() {
        crate::bin_cli::output::line(format!(
            "Missing language variants ({}):",
            report.missing_language_variants.len()
        ));
        for variant in &report.missing_language_variants {
            crate::bin_cli::output::line(format!("  {} — {}", variant.group.display(), variant.language));
        }
    }
    if !report.skips_without_reason.is_empty() {
        crate::bin_cli::output::line(format!("Skips without reason ({}):", report.skips_without_reason.len()));
        for location in &report.skips_without_reason {
            crate::bin_cli::output::line(format!(
                "  {}:{} (block {})",
                location.path.display(),
                location.line,
                location.block_index
            ));
        }
    }
    if !report.unknown_languages.is_empty() {
        crate::bin_cli::output::line(format!("Unknown languages ({}):", report.unknown_languages.len()));
        for unknown in &report.unknown_languages {
            crate::bin_cli::output::line(format!(
                "  {}:{} tag={}",
                unknown.path.display(),
                unknown.line,
                unknown.tag
            ));
        }
    }
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_coverage_rejects_every_non_validation_status() {
        assert!(is_incomplete_status(SnippetStatus::Skip));
        assert!(is_incomplete_status(SnippetStatus::Unavailable));
        assert!(is_incomplete_status(SnippetStatus::Downgraded));
        assert!(!is_incomplete_status(SnippetStatus::Pass));
    }

    #[test]
    fn configured_audit_is_skipped_without_a_docs_surface() {
        let directory = tempfile::tempdir().expect("temp directory");
        let snippets = directory.path().join("snippets");
        std::fs::create_dir_all(&snippets).expect("snippet directory");
        std::fs::write(snippets.join("weird.md"), "```gibberish\nvalue\n```\n").expect("write snippet");
        let snippet_directories = [snippets];

        let (audit_failure, gap_failure) = run_configured_audit_and_gaps(&ConfiguredCheckInputs {
            snippet_directories: &snippet_directories,
            docs_directories: &[],
            include_base_paths: &[],
            required_languages: &[],
            exclude: &[],
            readme: None,
            content_collections: &std::collections::BTreeMap::new(),
            workspace_root: directory.path(),
            require_frontmatter: false,
            strict: true,
        })
        .expect("audit and gap pass");

        assert!(
            !audit_failure,
            "a snippets-only config has no documentation surface to audit, so an unknown fence tag \
             must not fail the gate — `docs/mod.rs::validate_snippets` skips audit the same way"
        );
        assert!(
            !gap_failure,
            "gaps are meaningless without docs dirs or required languages"
        );
    }

    #[test]
    fn readme_snippet_mappings_count_as_references_for_the_strict_gate() {
        let directory = tempfile::tempdir().expect("temp directory");
        let snippets = directory.path().join("snippets");
        let docs = directory.path().join("docs");
        std::fs::create_dir_all(snippets.join("python")).expect("snippet directory");
        std::fs::create_dir_all(&docs).expect("docs directory");
        std::fs::write(snippets.join("python/hello.md"), "```python\nvalue = 1\n```\n").expect("write snippet");
        let snippet_directories = [snippets];
        let docs_directories = [docs];
        let content_collections = std::collections::BTreeMap::new();
        let readme = crate::core::config::ReadmeConfig {
            template_dir: None,
            snippets_dir: Some(PathBuf::from("snippets")),
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: std::collections::HashMap::from([(
                "python".to_string(),
                serde_json::json!({ "snippets": ["hello.md"] }),
            )]),
            targets: std::collections::HashMap::new(),
        };

        let (audit_failure, gap_failure) = run_configured_audit_and_gaps(&ConfiguredCheckInputs {
            snippet_directories: &snippet_directories,
            docs_directories: &docs_directories,
            include_base_paths: &docs_directories,
            required_languages: &[],
            exclude: &[],
            readme: Some(&readme),
            content_collections: &content_collections,
            workspace_root: directory.path(),
            require_frontmatter: false,
            strict: true,
        })
        .expect("audit and gap pass");

        assert!(!audit_failure);
        assert!(
            !gap_failure,
            "a snippet named by [crates.readme.languages.*].snippets is referenced even though no \
             documentation page `--8<--`-includes it"
        );

        let (_, gap_failure_without_readme) = run_configured_audit_and_gaps(&ConfiguredCheckInputs {
            snippet_directories: &snippet_directories,
            docs_directories: &docs_directories,
            include_base_paths: &docs_directories,
            required_languages: &[],
            exclude: &[],
            readme: None,
            content_collections: &content_collections,
            workspace_root: directory.path(),
            require_frontmatter: false,
            strict: true,
        })
        .expect("audit and gap pass");

        assert!(
            gap_failure_without_readme,
            "without the README source the same snippet reads as unreferenced, so this test would \
             pass vacuously if the reference sources were dropped"
        );
    }

    #[test]
    fn resolved_roots_drop_excluded_prefixes() {
        let root = Path::new("/workspace");
        let excluded = [root.join("snippets/vendored")];

        let resolved = resolved_roots(
            root,
            &[PathBuf::from("snippets"), PathBuf::from("snippets/vendored")],
            &excluded,
        );

        assert_eq!(resolved, vec![root.join("snippets")]);
    }

    #[test]
    fn generated_coverage_manifest_exposes_missing_cells() {
        let directory = tempfile::tempdir().expect("temp directory");
        let ledger = crate::e2e::snippets::SnippetCoverageLedger {
            expected: vec![crate::e2e::snippets::SnippetCoverageKey {
                fixture_id: "extension_only".into(),
                language: "python".into(),
            }],
            missing: vec![crate::e2e::snippets::MissingSnippet {
                key: crate::e2e::snippets::SnippetCoverageKey {
                    fixture_id: "extension_only".into(),
                    language: "python".into(),
                },
                reason: "no compatible recipe".into(),
            }],
            ..Default::default()
        };
        std::fs::write(
            directory.path().join(crate::e2e::snippets::COVERAGE_MANIFEST),
            serde_json::to_vec(&ledger).expect("serialize ledger"),
        )
        .expect("write ledger");

        let missing = missing_generated_snippets(&[directory.path().to_path_buf()]).expect("read ledger");
        assert_eq!(missing, ledger.missing);
    }

    #[test]
    fn configured_sessions_accept_binding_targets_and_reject_unknown_keys() {
        let root = std::env::current_dir()
            .expect("current directory")
            .join("neutral-workspace");
        let mut config = crate::core::config::DocsSnippetsConfig::default();
        config.sessions.insert(
            "wasm".into(),
            crate::core::config::output::DocsSnippetSessionConfig {
                cwd: "bindings/wasm".into(),
                ..Default::default()
            },
        );
        let sessions = configured_sessions(&config, &root, &[]).expect("known target");
        assert_eq!(sessions["wasm"].language, Language::TypeScript);
        assert_eq!(sessions["wasm"].working_directory, root.join("bindings/wasm"));

        config.sessions.insert(
            "unsupported-runtime".into(),
            crate::core::config::output::DocsSnippetSessionConfig::default(),
        );
        assert!(configured_sessions(&config, &root, &[]).is_err());
    }

    #[test]
    fn configured_rust_session_enables_crate_features_so_gated_modules_resolve() {
        let root = std::env::current_dir()
            .expect("current directory")
            .join("neutral-workspace");
        let mut config = crate::core::config::DocsSnippetsConfig::default();
        config.sessions.insert(
            "rust".into(),
            crate::core::config::output::DocsSnippetSessionConfig {
                cwd: "crates/sample-core".into(),
                rust_features: vec!["telemetry".into()],
                ..Default::default()
            },
        );
        config.sessions.insert(
            "wasm".into(),
            crate::core::config::output::DocsSnippetSessionConfig {
                cwd: "bindings/wasm".into(),
                ..Default::default()
            },
        );
        let crate_features = vec!["plugins".to_string(), "telemetry".to_string()];

        let sessions = configured_sessions(&config, &root, &crate_features).expect("known targets");

        assert_eq!(sessions["rust"].language, Language::Rust);
        assert_eq!(
            sessions["rust"].rust_features,
            vec!["plugins".to_string(), "telemetry".to_string()],
            "a Rust snippet session must build the path dependency with the crate's declared features, \
             otherwise snippets importing a feature-gated module fail with `unresolved import`"
        );
        assert!(
            sessions["wasm"].rust_features.is_empty(),
            "crate features must not leak into non-Rust sessions"
        );
    }
}
