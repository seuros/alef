// This module is a sanctioned stdout reporting surface. Every function here formats and prints a
// report table (validation summary, snippet listing) that is the primary deliverable of its
// `alef snippets` subcommand, not a diagnostic. Routing each of the many `println!` calls below
// through `crate::bin_cli::output::line` would be pure churn for no behavioral difference, so the
// whole module carries the allow instead of one per call site. ~keep
#![allow(clippy::print_stdout)]

use crate::snippets::error::Result;
use crate::snippets::types::{RunSummary, Snippet, SnippetStatus};
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
        } else if result.status == SnippetStatus::Downgraded || result.capability_capped {
            // `finalize_result` computes a `message` naming the requested/effective level and, for
            // a capability ceiling, which validator caps it — but until now that message was only
            // ever printed for `Fail`/`Error`, so a `DOWNGRADE` row (or a capability-capped `Pass`)
            // gave a reader the level gap with no reason attached, indistinguishable in the
            // human-readable report from an unexplained regression. Printed as its own one-line
            // reason rather than joining the `Source`/`Error`/`Code` block above, which is about
            // showing a failing snippet's content — nothing failed here. ~keep
            if let Some(message) = &result.message {
                let trimmed = message.trim();
                if !trimmed.is_empty() {
                    println!("  Reason: {trimmed}");
                }
            }
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
    println!();
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
