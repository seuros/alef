//! `alef snippets` subcommand — discover, validate, audit, and gap-check documentation snippets.

use crate::snippets::audit::{AuditConfig, AuditSeverity, audit};
use crate::snippets::discovery;
use crate::snippets::gaps::{GapConfig, detect_gaps};
use crate::snippets::output;
use crate::snippets::runner::{RunnerConfig, run_validation};
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

    /// Validate snippet syntax (and optionally compilation / execution).
    Validate {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short = 'L', long, default_value = "syntax")]
        level: ValidationLevel,

        #[arg(short, long, value_delimiter = ',')]
        languages: Option<Vec<String>>,

        #[arg(short, long)]
        output: Option<PathBuf>,

        #[arg(short = 'j', long, default_value = "4")]
        jobs: usize,

        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,

        #[arg(long)]
        fail_fast: bool,

        #[arg(long)]
        include: Option<String>,

        #[arg(long)]
        show_code: bool,

        #[arg(long)]
        strict: bool,

        #[arg(long)]
        changed_only: bool,
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
        SnippetsAction::Validate {
            snippets,
            level,
            languages,
            output: output_path,
            jobs,
            timeout,
            fail_fast,
            include,
            show_code,
            strict,
            changed_only,
        } => run_validate(
            &snippets,
            level,
            languages.as_ref(),
            output_path,
            jobs,
            timeout,
            fail_fast,
            include.as_ref(),
            show_code,
            strict,
            changed_only,
        ),
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

#[allow(clippy::too_many_arguments)]
fn run_validate(
    snippets: &[PathBuf],
    level: ValidationLevel,
    languages: Option<&Vec<String>>,
    output_path: Option<PathBuf>,
    jobs: usize,
    timeout: u64,
    fail_fast: bool,
    include: Option<&String>,
    show_code: bool,
    strict: bool,
    changed_only: bool,
) -> ExitCode {
    let filter = parse_language_filter(languages.map(Vec::as_slice));
    let mut found = match discovery::discover_snippets(snippets, filter.as_deref()) {
        Ok(found) => found,
        Err(err) => {
            tracing::error!("discovering snippets: {err}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(pattern) = &include {
        found.retain(|snippet| snippet.path.to_string_lossy().contains(pattern.as_str()));
    }

    if found.is_empty() {
        tracing::error!("no snippets found");
        return ExitCode::FAILURE;
    }

    tracing::info!("Validating {} snippets at level '{level}'...", found.len());
    let registry = ValidatorRegistry::new();
    let config = RunnerConfig {
        level,
        parallelism: jobs,
        timeout_secs: timeout,
        fail_fast,
        deny_unclassified: strict,
        allowed_side_effects: Vec::new(),
        cache_dir: Some(PathBuf::from(".alef/snippets")),
        changed_only,
    };

    match run_validation(&found, &registry, &config) {
        Ok(summary) => {
            output::print_summary(&summary, show_code);

            if let Some(path) = output_path {
                if let Err(err) = output::write_report(&summary, &path, show_code) {
                    tracing::error!("writing JSON output: {err}");
                    return ExitCode::FAILURE;
                } else {
                    tracing::info!("Results written to {}", path.display());
                }
            }

            if summary.has_failures() || strict && has_incomplete_coverage(&summary) {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            tracing::error!("running validation: {err}");
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
    let Some(config) = resolved.iter().find_map(|krate| krate.docs.as_ref()?.snippets.as_ref()) else {
        tracing::error!("no [workspace.docs.snippets] or [crates.docs.snippets] configuration found");
        return ExitCode::FAILURE;
    };
    let root = config_path.parent().unwrap_or_else(|| Path::new("."));
    let mut directories: Vec<PathBuf> = config
        .dirs
        .iter()
        .chain(&config.inline_dirs)
        .map(|path| root.join(path))
        .collect();
    directories.retain(|path| {
        !config
            .exclude
            .iter()
            .any(|excluded| path.starts_with(root.join(excluded)))
    });
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
    if summary.has_failures() || strict_failure {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
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
    let config = AuditConfig {
        docs_dirs: docs_dirs.to_vec(),
        snippet_dirs: snippet_dirs.to_vec(),
        require_frontmatter,
        include_base_paths: docs_dirs.to_vec(),
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
    let config = GapConfig {
        docs_dirs: docs_dirs.to_vec(),
        snippet_dirs: snippet_dirs.to_vec(),
        required_languages: required,
        include_base_paths: resolved_base_paths,
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
}
