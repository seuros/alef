//! Curated-versus-generated accounting for `alef snippets audit` / `alef snippets check`.
//!
//! Split from the command module so the classification decision is reachable from a test
//! without going through a process exit code, and so the command module stays under the
//! repository's file-size cap.

use crate::snippets::audit::{AuditConfig, AuditReport, SnippetAccounting, audit};
use std::path::{Path, PathBuf};

/// Resolve the project's curated-snippet declaration, in the path spelling the audit's own
/// snippet-root walk produces.
///
/// Every resolved crate contributes: a workspace can configure `[crates.e2e.snippets]` per
/// crate, and an audit that silently read only the first one would report the rest of the
/// project's declared files as gaps.
///
/// # Errors
///
/// Returns an error when the config cannot be loaded or a curated glob is unusable -- a
/// declaration that cannot be resolved must fail the run rather than degrade into "nothing is
/// curated", which is indistinguishable from a project that declared nothing. ~keep
pub(crate) fn configured_curated_paths(config_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let (_, resolved) = crate::bin_cli::helpers::load_config(config_path)?;
    let root = config_path.parent().unwrap_or_else(|| Path::new("."));
    let patterns: Vec<String> = resolved
        .iter()
        .filter_map(|krate| krate.e2e.as_ref()?.snippets.as_ref())
        .flat_map(|snippets| snippets.curated_snippets.iter().cloned())
        .collect();
    Ok(
        crate::e2e::snippets::coverage::resolve_curated_snippet_paths(root, &patterns)?
            .into_iter()
            .map(|relative| root.join(relative))
            .collect(),
    )
}

/// Name the accounting pass's scope, so a run that never classified anything cannot read as
/// one that classified everything cleanly. ~keep
pub(crate) fn accounting_scope_line(config_path: Option<&Path>, enabled: bool, curated: usize) -> String {
    match (config_path, enabled) {
        (None, _) => "Curated accounting was NOT run - pass --config to tell a declared \
                      hand-authored snippet apart from an unaccounted coverage gap."
            .to_string(),
        (Some(_), false) => "Curated accounting was NOT run - no coverage ledger under the snippet roots records \
                             anything as alef-generated, so every file would read as unaccounted."
            .to_string(),
        (Some(_), true) => format!("Curated accounting: {curated} snippet(s) declared curated."),
    }
}

/// One `alef snippets audit` verdict, computed without printing anything.
#[derive(Debug)]
pub(crate) struct AuditOutcome {
    pub report: AuditReport,
    pub accounting_enabled: bool,
}

/// Compute the audit verdict for one invocation.
///
/// # Errors
///
/// Returns an error when a coverage ledger is unusable or a curated declaration cannot be
/// resolved.
pub(crate) fn audit_outcome(
    snippet_dirs: &[PathBuf],
    docs_dirs: &[PathBuf],
    require_frontmatter: bool,
    config_path: Option<&Path>,
) -> anyhow::Result<AuditOutcome> {
    let generated_paths = crate::snippets::gaps::coverage_ledger_references(snippet_dirs)?;
    let curated_paths = config_path
        .map(configured_curated_paths)
        .transpose()?
        .unwrap_or_default();
    // Accounting needs both halves of the classification to mean something: without a
    // configuration there is no curated side, and without a ledger recording generated output
    // there is no generated side, so every file would report as an unaccounted gap. ~keep
    let accounting_enabled = config_path.is_some() && !generated_paths.is_empty();
    let config = AuditConfig {
        docs_dirs: docs_dirs.to_vec(),
        snippet_dirs: snippet_dirs.to_vec(),
        require_frontmatter,
        include_base_paths: docs_dirs.to_vec(),
        configured_references: generated_paths.clone(),
        exclude: Vec::new(),
        accounting: SnippetAccounting {
            generated_paths,
            curated_paths,
            enabled: accounting_enabled,
        },
    };
    Ok(AuditOutcome {
        report: audit(&config),
        accounting_enabled,
    })
}

#[cfg(test)]
mod tests;
