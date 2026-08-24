//! The `alef e2e snippets-migrate` comparison and its report rendering.
//!
//! Split out of `aux_commands` so the wiring between `[crates.e2e.snippets]` and the
//! curated-aware comparison is unit-testable: `aux_commands`'s command arms are reachable
//! only through a full extract/generate pipeline, which is not a seam a regression test can
//! stand on.

use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::SnippetConfig;
use crate::e2e::snippets::migration::{self, MigrationEntry, MigrationStatus};
use anyhow::Result;
use std::path::Path;

/// Compare hand-authored snippets under `existing_root` against what alef would generate,
/// carrying the project's `curated_snippets` declaration into the comparison.
///
/// ~keep The curated globs are the whole point of routing through
/// [`migration::compare_root_curated`] rather than the plain `compare_root`: without them
/// every hand-authored file reports as `no_generated_equivalent`, so a project with hundreds
/// of intentionally curated snippets cannot tell a declared file from a real migration gap.
///
/// `project_root` is the directory holding `alef.toml` -- the base both `snippet_config.output`
/// and the curated globs are written in.
///
/// # Errors
///
/// Returns an error when the comparison cannot be computed or a curated glob is unusable.
pub(crate) fn compare(
    project_root: &Path,
    existing_root: &Path,
    snippet_config: &SnippetConfig,
    generated: &[GeneratedFile],
) -> Result<Vec<MigrationEntry>> {
    migration::compare_root_curated(&migration::CuratedComparison {
        project_root,
        existing_root,
        generated_root: Path::new(&snippet_config.output),
        generated,
        curated_globs: &snippet_config.curated_snippets,
    })
}

/// The single token the human-readable report prints for an entry.
///
/// ~keep A curated entry gets its own label rather than `no_generated_equivalent` so the
/// text surface can be filtered on the same distinction the JSON surface carries in
/// [`MigrationEntry::curated`]; the JSON `status` stays structural.
fn status_label(entry: &MigrationEntry) -> &'static str {
    match (entry.status, entry.curated) {
        (MigrationStatus::Identical, _) => "identical",
        (MigrationStatus::Different, _) => "different",
        (MigrationStatus::NoGeneratedEquivalent, true) => "curated",
        (MigrationStatus::NoGeneratedEquivalent, false) => "no_generated_equivalent",
    }
}

pub(crate) fn write_report(entries: &[MigrationEntry], json: bool) -> Result<()> {
    if json {
        crate::bin_cli::output::payload(serde_json::to_string_pretty(entries)?);
        return Ok(());
    }
    for entry in entries {
        crate::bin_cli::output::line(format_args!("{}\t{}", status_label(entry), entry.path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry_for<'a>(entries: &'a [MigrationEntry], path: &str) -> &'a MigrationEntry {
        entries
            .iter()
            .find(|entry| entry.path == Path::new(path))
            .unwrap_or_else(|| panic!("comparison must report {path}: {entries:?}"))
    }

    /// The CLI wiring under test: `alef e2e snippets-migrate` reads `curated_snippets` off the
    /// project's `[crates.e2e.snippets]` table and must report a declared file as curated
    /// rather than as a coverage gap. Routing the comparison through the curated-*unaware*
    /// `migration::compare_root` — what the command did before this seam existed — makes both
    /// files below indistinguishable `no_generated_equivalent` entries.
    ///
    /// Shaped the way a consumer's tree actually is: the migrated root is a subdirectory of the
    /// project, and the curated file sits BESIDE the generated tree rather than within it, so
    /// the declaring glob is project-root-relative and reaches outside `output`.
    #[test]
    fn a_declared_curated_file_reports_as_curated_and_an_undeclared_one_stays_a_gap() {
        let directory = tempfile::tempdir().expect("tempdir");
        let existing_root = directory.path().join("docs/snippets");
        std::fs::create_dir_all(existing_root.join("cli")).expect("create curated directory");
        std::fs::write(existing_root.join("cli/quick-start.md"), "hand authored")
            .expect("write declared curated snippet");
        std::fs::write(existing_root.join("orphan.md"), "hand authored").expect("write undeclared snippet");
        let snippet_config = SnippetConfig {
            output: "docs/snippets/generated".into(),
            curated_snippets: vec!["docs/snippets/cli/*.md".to_string()],
            ..SnippetConfig::default()
        };

        let entries = compare(directory.path(), &existing_root, &snippet_config, &[]).expect("comparison succeeds");

        let curated = entry_for(&entries, "cli/quick-start.md");
        let gap = entry_for(&entries, "orphan.md");
        assert_eq!(curated.status, MigrationStatus::NoGeneratedEquivalent);
        assert!(
            curated.curated,
            "a path a configured curated_snippets glob claims must reach the CLI as curated"
        );
        assert!(
            !gap.curated,
            "a hand-authored path no glob claims must stay a genuine migration gap"
        );
        assert_eq!(status_label(curated), "curated");
        assert_eq!(status_label(gap), "no_generated_equivalent");
    }

    /// The machine-readable surface a consumer acts on: `--json` must carry the curated flag
    /// per entry, so a repo with hundreds of hand-authored snippets can partition them without
    /// re-deriving the globs itself.
    #[test]
    fn the_json_report_carries_the_curated_flag_for_each_entry() {
        let entries = vec![
            MigrationEntry {
                path: PathBuf::from("docker/quick-start.md"),
                status: MigrationStatus::NoGeneratedEquivalent,
                curated: true,
            },
            MigrationEntry {
                path: PathBuf::from("orphan.md"),
                status: MigrationStatus::NoGeneratedEquivalent,
                curated: false,
            },
        ];

        let rendered = serde_json::to_string_pretty(&entries).expect("entries serialize");

        let parsed: serde_json::Value = serde_json::from_str(&rendered).expect("report is valid JSON");
        assert_eq!(parsed[0]["curated"], serde_json::Value::Bool(true));
        assert_eq!(parsed[0]["status"], serde_json::json!("no_generated_equivalent"));
        assert_eq!(parsed[1]["curated"], serde_json::Value::Bool(false));
    }

    /// A curated declaration must not be able to hide a stale snippet: `curated` annotates the
    /// declared *absence* of a generated equivalent, so a file alef does generate keeps its
    /// `different` verdict even when a glob also names it.
    #[test]
    fn a_curated_glob_does_not_mask_a_snippet_that_differs_from_its_generated_equivalent() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(directory.path().join("docker")).expect("create directory");
        std::fs::write(directory.path().join("docker/quick-start.md"), "stale").expect("write stale snippet");
        let snippet_config = SnippetConfig {
            output: "docs/snippets-generated".into(),
            curated_snippets: vec!["docker/*.md".to_string()],
            ..SnippetConfig::default()
        };
        let generated = vec![GeneratedFile {
            path: PathBuf::from("docs/snippets-generated/docker/quick-start.md"),
            content: "fresh".into(),
            generated_header: false,
        }];

        let entries =
            compare(directory.path(), directory.path(), &snippet_config, &generated).expect("comparison succeeds");

        let entry = entry_for(&entries, "docker/quick-start.md");
        assert_eq!(entry.status, MigrationStatus::Different);
        assert!(!entry.curated);
        assert_eq!(status_label(entry), "different");
    }

    /// A typo'd glob must fail the command rather than silently reclassifying nothing —
    /// the same anti-vacuity posture the coverage ledger takes.
    #[test]
    fn an_invalid_curated_glob_fails_the_command_rather_than_reporting_nothing_curated() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("orphan.md"), "hand authored").expect("write snippet");
        let snippet_config = SnippetConfig {
            output: "docs/snippets-generated".into(),
            curated_snippets: vec!["[unterminated".to_string()],
            ..SnippetConfig::default()
        };

        let error =
            compare(directory.path(), directory.path(), &snippet_config, &[]).expect_err("an invalid glob must fail");

        assert!(error.to_string().contains("invalid curated snippet glob"), "{error}");
    }

    /// Anti-vacuity for the migration path's own key space: a migrated root that does not lie
    /// beneath the project root leaves every project-root-relative glob unmatchable, so the
    /// command must refuse rather than report "nothing curated" -- which is precisely what a
    /// genuinely empty declaration also looks like.
    #[test]
    fn a_migrated_root_outside_the_project_refuses_rather_than_reporting_nothing_curated() {
        let project = tempfile::tempdir().expect("project tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        std::fs::write(outside.path().join("orphan.md"), "hand authored").expect("write snippet");
        let snippet_config = SnippetConfig {
            output: "docs/snippets/generated".into(),
            curated_snippets: vec!["docs/snippets/cli/*.md".to_string()],
            ..SnippetConfig::default()
        };

        let error = compare(project.path(), outside.path(), &snippet_config, &[])
            .expect_err("an unrelatable migrated root must fail rather than silently match nothing");

        assert!(error.to_string().contains("does not lie beneath it"), "{error}");
    }
}
