// This module is a sanctioned stdout reporting surface. Every function here formats and prints a
// report table (validation summary, snippet listing) that is the primary deliverable of its
// `alef snippets` subcommand, not a diagnostic. Routing each of the many `println!` calls below
// through `crate::bin_cli::output::line` would be pure churn for no behavioral difference, so the
// whole module carries the allow instead of one per call site. ~keep
#![allow(clippy::print_stdout)]

use crate::snippets::error::Result;
use crate::snippets::types::{RunSummary, Snippet, SnippetStatus, ValidationResult};
use std::collections::BTreeMap;
use std::path::Path;

pub fn print_summary(summary: &RunSummary, show_code: bool) {
    println!();
    println!(
        "{:<60} {:<12} {:<10} {:<8} TIME",
        "SNIPPET", "LANGUAGE", "STATUS", "LEVEL"
    );
    println!("{}", "-".repeat(100));

    for result in &summary.results {
        let file_name = result
            .snippet
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("?");

        let status = match result.status {
            SnippetStatus::Pass => "PASS",
            SnippetStatus::Downgraded => "DOWNGRADE",
            SnippetStatus::Fail => "FAIL",
            SnippetStatus::Skip => "SKIP",
            SnippetStatus::Error => "ERROR",
            SnippetStatus::Unavailable => "N/A",
        };

        println!(
            "{:<60} {:<12} {:<10} {:<8} {}ms",
            truncate(file_name, 58),
            display_language(&result.snippet),
            status,
            result.effective_level,
            result.duration_ms
        );

        if matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error) {
            let title_info = result
                .snippet
                .title
                .as_deref()
                .map(|title| format!(" (title: {title})"))
                .unwrap_or_default();
            println!(
                "  Source: {}:{}{}",
                result.snippet.path.display(),
                result.snippet.start_line,
                title_info
            );

            if let Some(message) = &result.message {
                let trimmed = message.trim();
                if !trimmed.is_empty() {
                    println!("  Error:");
                    for line in trimmed.lines() {
                        println!("    {line}");
                    }
                }
            }

            if show_code {
                println!("  Code:");
                for (index, line) in result.snippet.code.lines().enumerate() {
                    println!("    {:>3} | {line}", index + 1);
                }
            }

            println!();
        } else if let Some(line) = reason_line(result) {
            println!("{line}");
        }
    }

    println!("{}", "-".repeat(100));
    println!(
        "Total: {}  Passed: {}  Downgraded: {}  Failed: {}  Skipped: {}  Errors: {}  Unavailable: {}",
        summary.total,
        summary.passed,
        summary.downgraded,
        summary.failed,
        summary.skipped,
        summary.errors,
        summary.unavailable
    );
    if let Some(line) = unresolved_dependency_rollup(summary) {
        println!("{line}");
    }
    println!();
}

/// One line naming the languages whose results were reclassified as `unresolved_dependency`, or
/// `None` when there were none.
///
/// The per-row reclassification landed in `ValidationResult::unresolved_dependency`, but the
/// report still presented those rows one at a time — and when a language's package was never
/// built, *every* snippet in it reclassifies, so the reader sees hundreds of rows with a single
/// upstream cause and no statement of that cause anywhere. Rolling them up per language is what
/// turns "376 typescript results" into "the typescript package was not built": the count is
/// evidence of one environmental fact, not of 376 problems. ~keep
fn unresolved_dependency_rollup(summary: &RunSummary) -> Option<String> {
    if summary.unresolved_dependency == 0 {
        return None;
    }
    let mut per_language: BTreeMap<String, usize> = BTreeMap::new();
    for result in summary.results.iter().filter(|result| result.unresolved_dependency) {
        *per_language.entry(display_language(&result.snippet)).or_default() += 1;
    }
    let breakdown = per_language
        .iter()
        .map(|(language, count)| format!("{language} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "Unresolved dependencies: {} of {} ({breakdown}) -- these languages' packages were not built, so their \
         snippets were never really validated. Run `alef build` before validating; the counts above measure the \
         environment, not the snippets.",
        summary.unresolved_dependency, summary.total
    ))
}

/// The `  Reason: ...` line for a row whose effective level differs from what was requested, or
/// `None` for a clean result. Covers every `downgrade_reason` the runner sets, not just
/// `Downgraded`/`capability_capped` — a `Pass` clamped by a snippet's own declared `level:`
/// (`DowngradeReason::Declared`) carries a reason too, and leaving it out of this table is what
/// let `docs.snippets.validation_level = "run"` clamp silently down to a fixture's stamped
/// `typecheck` ceiling with no visible trace in the human-readable report. ~keep
fn reason_line(result: &ValidationResult) -> Option<String> {
    result.downgrade_reason?;
    let message = result.message.as_deref()?.trim();
    (!message.is_empty()).then(|| format!("  Reason: {message}"))
}

/// Write validation results to a JSON file.
///
/// # Errors
///
/// Returns an error when serialization fails or the destination cannot be written.
pub fn write_json(summary: &RunSummary, path: &Path, show_code: bool) -> Result<()> {
    let mut value = serde_json::to_value(summary)?;
    if !show_code && let Some(results) = value.get_mut("results").and_then(serde_json::Value::as_array_mut) {
        for result in results {
            if let Some(snippet) = result.get_mut("snippet").and_then(serde_json::Value::as_object_mut) {
                snippet.remove("code");
            }
        }
    }
    let json = serde_json::to_string_pretty(&value)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Write a versioned summary as JSON or TOON, selected by the destination extension.
///
/// # Errors
///
/// Returns an error when serialization or writing fails.
pub fn write_report(summary: &RunSummary, path: &Path, show_code: bool) -> Result<()> {
    if path.extension().and_then(|value| value.to_str()) != Some("toon") {
        return write_json(summary, path, show_code);
    }
    let mut output = format!(
        "schema_version: {}\ntotal: {}\npassed: {}\ndowngraded: {}\nfailed: {}\nskipped: {}\nerrors: {}\nunavailable: {}\nresults[{}]:\n",
        summary.schema_version,
        summary.total,
        summary.passed,
        summary.downgraded,
        summary.failed,
        summary.skipped,
        summary.errors,
        summary.unavailable,
        summary.results.len()
    );
    for result in &summary.results {
        output.push_str(&format!(
            "  - path: {}\n    line: {}\n    language: {}\n    status: {}\n    requested_level: {}\n    effective_level: {}\n",
            result.snippet.source_origin.path.display(),
            result.snippet.source_origin.line,
            display_language(&result.snippet),
            result.status,
            result.requested_level,
            result.effective_level
        ));
        if show_code {
            output.push_str("    code: |\n");
            for line in result.snippet.code.lines() {
                output.push_str(&format!("      {line}\n"));
            }
        }
    }
    std::fs::write(path, output)?;
    Ok(())
}

fn display_language(snippet: &Snippet) -> String {
    snippet.metadata.target.as_ref().map_or_else(
        || snippet.language.to_string(),
        |target| format!("{}/{target}", snippet.language),
    )
}

pub fn print_snippet_list(snippets: &[Snippet]) {
    println!("{:<60} {:<12} {:<8} TITLE", "FILE", "LANGUAGE", "LINE");
    println!("{}", "-".repeat(95));

    for snippet in snippets {
        let file_name = snippet.path.file_name().and_then(|name| name.to_str()).unwrap_or("?");

        println!(
            "{:<60} {:<12} {:<8} {}",
            truncate(file_name, 58),
            snippet.language,
            snippet.start_line,
            snippet.title.as_deref().unwrap_or("-")
        );
    }

    println!("{}", "-".repeat(95));
    println!("Total: {} snippets", snippets.len());
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        value.to_string()
    } else {
        format!("{}...", &value[..max.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{DowngradeReason, Language, SnippetMetadata, SourceOrigin, ValidationLevel};

    fn sample_result(
        status: SnippetStatus,
        downgrade_reason: Option<DowngradeReason>,
        message: Option<&str>,
    ) -> ValidationResult {
        ValidationResult {
            snippet: Snippet {
                id: None,
                path: "example.md".into(),
                language: Language::Rust,
                title: None,
                code: "fn main() {}".into(),
                start_line: 1,
                block_index: 0,
                annotation: None,
                metadata: SnippetMetadata::default(),
                source_origin: SourceOrigin {
                    path: "example.md".into(),
                    line: 1,
                    block_index: 0,
                },
            },
            status,
            level: ValidationLevel::TypeCheck,
            requested_level: ValidationLevel::Run,
            effective_level: ValidationLevel::TypeCheck,
            message: message.map(str::to_owned),
            duration_ms: 0,
            capability_capped: false,
            downgrade_reason,
            unresolved_dependency: false,
        }
    }

    /// The regression this exists for: a `Pass` clamped by a snippet's own declared `level:`
    /// front matter used to print nothing at all — `print_summary`'s reason line only fired for
    /// `Downgraded` or `capability_capped` rows, so `docs.snippets.validation_level = "run"`
    /// clamped to a fixture's stamped `typecheck` ceiling with no visible trace anywhere in the
    /// human-readable report. ~keep
    #[test]
    fn declared_downgrade_reason_produces_a_reason_line() {
        let result = sample_result(
            SnippetStatus::Pass,
            Some(DowngradeReason::Declared),
            Some("requested run, validated at declared level typecheck"),
        );

        assert_eq!(
            reason_line(&result),
            Some("  Reason: requested run, validated at declared level typecheck".to_string())
        );
    }

    /// Negative control: an ordinary `Pass` for a legitimately-configured level with nothing
    /// clamped carries no `downgrade_reason` and must produce no reason line at all. ~keep
    #[test]
    fn ordinary_pass_with_no_downgrade_reason_has_no_reason_line() {
        let result = sample_result(SnippetStatus::Pass, None, None);

        assert_eq!(reason_line(&result), None);
    }

    fn unresolved_in(language: Language) -> ValidationResult {
        let mut result = sample_result(SnippetStatus::Unavailable, None, Some("cannot find module"));
        result.snippet.language = language;
        result.unresolved_dependency = true;
        result
    }

    /// The reader's takeaway has to be "two packages were not built", not "446 snippets are
    /// broken" — so the rollup names the languages and says the counts describe the environment.
    /// ~keep
    #[test]
    fn unresolved_dependency_rollup_names_each_language_and_its_count() {
        let summary = RunSummary::from_results(vec![
            unresolved_in(Language::TypeScript),
            unresolved_in(Language::TypeScript),
            unresolved_in(Language::Python),
            sample_result(SnippetStatus::Fail, None, Some("syntax error")),
        ]);

        let line = unresolved_dependency_rollup(&summary).expect("rollup for reclassified results");

        assert!(line.contains("Unresolved dependencies: 3 of 4"), "{line}");
        assert!(line.contains("python 1"), "{line}");
        assert!(line.contains("typescript 2"), "{line}");
        assert!(line.contains("alef build"), "{line}");
    }

    /// Negative control: a clean run must not grow an extra line implying anything was
    /// unavailable. ~keep
    #[test]
    fn unresolved_dependency_rollup_is_absent_when_nothing_was_reclassified() {
        let summary = RunSummary::from_results(vec![sample_result(SnippetStatus::Pass, None, None)]);

        assert_eq!(unresolved_dependency_rollup(&summary), None);
    }
}
