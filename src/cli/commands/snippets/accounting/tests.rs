//! Tests for `alef snippets audit`'s curated-versus-generated accounting.
//!
//! Every fixture below builds the shape a real consumer has: a generated tree with a coverage
//! ledger in it, and hand-authored snippets sitting BESIDE that tree rather than inside it.

use super::{accounting_scope_line, audit_outcome, configured_curated_paths};
use crate::snippets::audit::{AuditIssueKind, AuditSeverity};
use std::path::{Path, PathBuf};

const SNIPPET_BODY: &str = "```python\nprint(\"ok\")\n```\n";

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("path has a parent")).expect("create parent directory");
    std::fs::write(path, content).expect("write fixture file");
}

/// A project tree with one alef-generated snippet (recorded by a real coverage ledger) and one
/// hand-authored snippet outside the generated tree.
fn project(curated_globs: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path();
    write(root, "docs/snippets/generated/python/example.md", SNIPPET_BODY);
    write(root, "docs/snippets/cli/quickstart.md", SNIPPET_BODY);
    write(
        root,
        "docs/snippets/generated/.alef-snippet-coverage.json",
        r#"{
  "format_version": 2,
  "generated_paths": ["python/example.md"],
  "generated_metadata": [
    {
      "key": { "fixture_id": "sample", "language": "python" },
      "path": "python/example.md",
      "language": "python",
      "target": "python",
      "session": "python",
      "requires": [],
      "side_effect": "safe"
    }
  ],
  "expected": [{ "fixture_id": "sample", "language": "python" }],
  "generated": [{ "fixture_id": "sample", "language": "python" }],
  "missing": [],
  "documented_exceptions": []
}
"#,
    );
    write(
        root,
        "alef.toml",
        &format!(
            r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "e2e/fixtures"
output = "e2e"

[crates.e2e.call]
function = "example"
module = "sample"
result_var = "result"
args = []

[crates.e2e.snippets]
output = "docs/snippets/generated"
curated_snippets = [{curated_globs}]
"#
        ),
    );
    directory
}

fn snippet_roots(root: &Path) -> Vec<PathBuf> {
    vec![root.join("docs/snippets")]
}

/// The defect this feature exists to close: with the declaration configured, the hand-authored
/// file must come back as CURATED, and must not be reported as an unaccounted coverage gap.
/// The undeclared sibling must still be reported, or the check would prove nothing.
#[test]
fn a_configured_curated_file_reports_as_curated_and_an_undeclared_one_stays_a_gap() {
    let directory = project("\"docs/snippets/cli/*.md\"");
    let root = directory.path();
    write(root, "docs/snippets/legacy/leftover.md", SNIPPET_BODY);

    let outcome = audit_outcome(&snippet_roots(root), &[], false, Some(&root.join("alef.toml")))
        .expect("audit succeeds over a configured project");

    assert!(
        outcome.accounting_enabled,
        "a ledger and a config together must enable the accounting pass"
    );
    assert_eq!(
        outcome.report.curated,
        vec![root.join("docs/snippets/cli/quickstart.md")],
        "the declared hand-authored snippet must be reported as curated"
    );
    let unaccounted: Vec<&PathBuf> = outcome
        .report
        .issues
        .iter()
        .filter(|issue| issue.kind == AuditIssueKind::UnaccountedSnippet)
        .map(|issue| &issue.path)
        .collect();
    assert_eq!(
        unaccounted,
        vec![&root.join("docs/snippets/legacy/leftover.md")],
        "only the undeclared hand-authored snippet may be reported as an unaccounted gap"
    );
}

/// The curated file in every fixture here lives outside `output`, which is where hand-authored
/// snippets are actually found. Pinned separately so a regression to `output`-relative
/// resolution fails on the semantics rather than on some incidental assertion.
#[test]
fn a_curated_file_outside_the_output_tree_is_declarable_and_recognised() {
    let directory = project("\"docs/snippets/cli/*.md\"");
    let root = directory.path();

    let curated = configured_curated_paths(&root.join("alef.toml")).expect("the declaration resolves");

    assert_eq!(curated, vec![root.join("docs/snippets/cli/quickstart.md")]);
    assert!(
        !curated[0].starts_with(root.join("docs/snippets/generated")),
        "the declared file must be outside the configured output tree, or this proves nothing"
    );
}

/// Anti-vacuity at the command boundary: a declaration that matches nothing must fail the run.
/// Degrading to "nothing is curated" would be indistinguishable from a project that declared
/// nothing, and would leave every file the glob was meant to cover reported as a gap.
#[test]
fn a_declaration_matching_zero_files_fails_the_audit() {
    let directory = project("\"docs/snippets/clu/*.md\"");
    let root = directory.path();

    let error = audit_outcome(&snippet_roots(root), &[], false, Some(&root.join("alef.toml")))
        .expect_err("a glob matching zero files must fail the audit");

    assert!(error.to_string().contains("matches no file"), "{error:#}");
}

/// A curated declaration must never annex alef's own output: claiming a generated path would
/// let the declaration silently retire a real coverage cell.
#[test]
fn a_declaration_claiming_a_generated_path_is_an_audit_error() {
    let directory = project("\"docs/snippets/generated/python/*.md\"");
    let root = directory.path();

    let outcome = audit_outcome(&snippet_roots(root), &[], false, Some(&root.join("alef.toml")))
        .expect("resolution succeeds; the collision is reported as an audit issue");

    let claimed: Vec<&AuditIssueKind> = outcome
        .report
        .issues
        .iter()
        .filter(|issue| issue.kind == AuditIssueKind::CuratedGeneratedSnippet)
        .map(|issue| &issue.kind)
        .collect();
    assert_eq!(claimed.len(), 1, "the collision must be reported: {:?}", outcome.report);
    assert!(
        outcome.report.has_errors(),
        "a declaration claiming alef's own output must fail the audit"
    );
}

/// An unaccounted snippet is a coverage observation, not a structural defect: reporting it as
/// an error would turn every project that has not yet declared its curated files red on
/// upgrade. It must still be reported, so the severity is warning, not silence.
#[test]
fn an_unaccounted_snippet_is_reported_as_a_warning_and_does_not_fail_the_audit() {
    let directory = project("\"docs/snippets/cli/*.md\"");
    let root = directory.path();
    write(root, "docs/snippets/legacy/leftover.md", SNIPPET_BODY);

    let outcome = audit_outcome(&snippet_roots(root), &[], false, Some(&root.join("alef.toml")))
        .expect("audit succeeds over a configured project");

    let issue = outcome
        .report
        .issues
        .iter()
        .find(|issue| issue.kind == AuditIssueKind::UnaccountedSnippet)
        .expect("the undeclared snippet is reported");
    assert_eq!(issue.severity, AuditSeverity::Warning);
    assert!(
        !outcome.report.has_errors(),
        "an unaccounted snippet must not fail the audit: {:?}",
        outcome.report.issues
    );
}

/// Without `--config` the accounting pass has no curated side, so it must not run -- and must
/// say so. A run that skipped the classification entirely cannot be allowed to read like one
/// that classified everything cleanly.
#[test]
fn without_a_config_the_accounting_pass_is_skipped_and_named() {
    let directory = project("\"docs/snippets/cli/*.md\"");
    let root = directory.path();

    let outcome = audit_outcome(&snippet_roots(root), &[], false, None).expect("audit succeeds without a config");

    assert!(!outcome.accounting_enabled);
    assert!(
        outcome.report.curated.is_empty(),
        "no config means no curated verdict may be claimed"
    );
    assert!(
        !outcome
            .report
            .issues
            .iter()
            .any(|issue| issue.kind == AuditIssueKind::UnaccountedSnippet),
        "an unconfigured run must not report gaps it had no declaration to check against"
    );
    let line = accounting_scope_line(None, false, 0);
    assert!(line.contains("NOT run"), "{line}");
    assert!(line.contains("--config"), "{line}");
}

/// The other half of the skip: a configured project whose snippet roots hold no coverage
/// ledger has no generated side either, so every file would read as unaccounted. Skipping is
/// right; skipping silently is not.
#[test]
fn without_a_coverage_ledger_the_accounting_pass_is_skipped_and_named() {
    let directory = project("\"docs/snippets/cli/*.md\"");
    let root = directory.path();
    std::fs::remove_file(root.join("docs/snippets/generated/.alef-snippet-coverage.json"))
        .expect("remove the coverage ledger");

    let outcome = audit_outcome(&snippet_roots(root), &[], false, Some(&root.join("alef.toml")))
        .expect("audit succeeds without a ledger");

    assert!(!outcome.accounting_enabled);
    assert!(
        !outcome
            .report
            .issues
            .iter()
            .any(|issue| issue.kind == AuditIssueKind::UnaccountedSnippet),
        "with nothing recorded as generated, every file would read as a gap; the pass must be skipped"
    );
    let line = accounting_scope_line(Some(Path::new("alef.toml")), false, 0);
    assert!(line.contains("NOT run"), "{line}");
    assert!(line.contains("coverage ledger"), "{line}");
}
