//! A configured directory that does not exist must be reported as exactly that, on every
//! command that walks one.
//!
//! `discover_snippets` already refuses one (`src/snippets/discovery.rs`), but three surfaces
//! were left behind, all with the same shape: a check that passes, or misreports, because it
//! examined nothing.
//!
//! * `alef snippets audit` / `gaps` did fail on a missing `--snippets` root, but only by
//!   accident -- the first thing they do is walk it for coverage ledgers, so the user was told
//!   "reading generated snippet coverage: ... IO error" for a path that simply is not there.
//! * A missing `--docs` root was worse: nothing walks it eagerly, so `audit` printed "Audit
//!   clean: no issues found" over a documentation tree it never opened.
//! * `alef verify` had no opinion at all -- it checks generated-file hashes and coverage-ledger
//!   freshness, so a `docs.snippets.dirs`/`inline_dirs` entry pointing at a path that was
//!   renamed or never created passed as "All bindings and versions are up to date".
//!
//! Driven through the real binary rather than the library functions, because in every case the
//! defect was in what the command reports, not in what the library computes. ~keep

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn alef_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_alef"))
}

fn run_alef(root: &Path, args: &[&str]) -> Output {
    Command::new(alef_binary())
        .current_dir(root)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run alef {args:?}: {error}"))
}

fn context(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// One real snippet root holding one valid snippet, so a failure can only come from the
/// missing directory under test and never from an empty corpus.
fn write_snippet_root(root: &Path) -> PathBuf {
    let snippets = root.join("snippets");
    fs::create_dir_all(&snippets).expect("create snippets directory");
    fs::write(snippets.join("hello.md"), "```json\n{\"hello\": \"world\"}\n```\n").expect("write snippet fixture");
    snippets
}

#[test]
fn audit_names_the_missing_snippets_root_instead_of_a_coverage_ledger_walk() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let missing = dir.path().join("snippets-never-created");

    let output = run_alef(
        dir.path(),
        &["snippets", "audit", "--snippets", &missing.to_string_lossy()],
    );

    assert!(
        !output.status.success(),
        "a snippets root that does not exist must fail the audit.\n{}",
        context(&output)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("configured snippet directory does not exist"),
        "the diagnostic must name the real cause -- a configured directory that is not there -- \
         not the coverage-ledger walk that happened to trip over it first.\n{}",
        context(&output)
    );
    assert!(
        stderr.contains(&missing.display().to_string()),
        "the diagnostic must name the missing path.\n{}",
        context(&output)
    );
}

#[test]
fn audit_fails_on_a_missing_docs_root_instead_of_reporting_it_clean() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let snippets = write_snippet_root(dir.path());
    let missing = dir.path().join("docs-never-created");

    let output = run_alef(
        dir.path(),
        &[
            "snippets",
            "audit",
            "--snippets",
            &snippets.to_string_lossy(),
            "--docs",
            &missing.to_string_lossy(),
        ],
    );

    assert!(
        !output.status.success(),
        "a documentation root that does not exist must fail the audit -- reporting it clean \
         means claiming a tree was audited when not one file in it was opened.\n{}",
        context(&output)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("configured documentation directory does not exist"),
        "the diagnostic must name the missing documentation root.\n{}",
        context(&output)
    );
    assert!(
        stderr.contains(&missing.display().to_string()),
        "the diagnostic must name the missing path.\n{}",
        context(&output)
    );
}

#[test]
fn gaps_names_the_missing_docs_root_instead_of_reporting_every_snippet_unreferenced() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let snippets = write_snippet_root(dir.path());
    let missing = dir.path().join("docs-never-created");

    let output = run_alef(
        dir.path(),
        &[
            "snippets",
            "gaps",
            "--snippets",
            &snippets.to_string_lossy(),
            "--docs",
            &missing.to_string_lossy(),
        ],
    );

    assert!(
        !output.status.success(),
        "a documentation root that does not exist must fail the gap report.\n{}",
        context(&output)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("configured documentation directory does not exist"),
        "the gap report used to fail for the wrong reason -- with no docs tree to walk, every \
         snippet reads as unreferenced -- which points the user at their snippets instead of at \
         the path that is not there.\n{}",
        context(&output)
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("Unreferenced snippets"),
        "the misleading unreferenced-snippet finding must not be reported for a docs root that \
         was never walked.\n{}",
        context(&output)
    );
}

/// An existing but empty root is a different, legitimate case and must stay silent: it is not a
/// typo, it is a corpus that is genuinely empty. Guards the fix against being over-applied.
#[test]
fn an_existing_empty_docs_root_still_audits_clean() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let snippets = write_snippet_root(dir.path());
    let docs = dir.path().join("docs");
    fs::create_dir_all(&docs).expect("create empty docs directory");

    let output = run_alef(
        dir.path(),
        &[
            "snippets",
            "audit",
            "--snippets",
            &snippets.to_string_lossy(),
            "--docs",
            &docs.to_string_lossy(),
        ],
    );

    assert!(
        output.status.success(),
        "an existing but empty documentation root is not a misconfiguration.\n{}",
        context(&output)
    );
}

const FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";
const FIXTURE_CARGO_TOML: &str =
    "[package]\nname = \"verify-missing-dir-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[workspace.docs.snippets]
dirs = ["snippets"]
inline_dirs = ["docs/guides-removed-after-generation"]

[[crates]]
name = "verify-missing-dir-fixture"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.python]
module_name = "verify_missing_dir_fixture"

[crates.python.stubs]
output = "packages/python/verify_missing_dir_fixture"
"#;

/// `alef verify` must report a `docs.snippets` root that is not on disk.
///
/// The fixture is generated with `alef all` first so that the missing `inline_dirs` entry is
/// the only finding left: before the fix this run reported "All bindings and versions are up to
/// date" and exited 0, even though `alef all` on the very same configuration prints an error
/// for that root (`docs::build_snippet_context`). `verify` reaches that same docs stage through
/// `find_missing_and_frozen_generated_files`, where a stage error is deliberately downgraded to
/// a debug log -- so the condition was already being detected and then discarded. ~keep
#[test]
fn verify_reports_a_configured_snippet_root_that_does_not_exist() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
    write_snippet_root(&root);
    // Generated with the root in place, then removed: this is the drift `verify` exists to
    // catch -- a directory that was renamed or deleted after the last successful generation.
    // `alef all` refuses to run at all while the root is missing (`build_snippet_context`),
    // which is precisely why `verify` -- the command a user runs against an already-generated
    // tree -- is the one that has to notice afterwards. ~keep
    let removed_root = root.join("docs/guides-removed-after-generation");
    fs::create_dir_all(&removed_root).expect("create the root that generation requires");

    let generated = run_alef(&root, &["all", "--skip-frb"]);
    assert!(
        generated.status.success(),
        "alef all must succeed against the fixture.\n{}",
        context(&generated)
    );

    fs::remove_dir_all(&removed_root).expect("remove the configured root after generation");

    let output = run_alef(&root, &["verify"]);

    assert!(
        !output.status.success(),
        "verify must fail on a configured docs.snippets root that does not exist -- passing \
         means every snippet check that walks it was graded against nothing.\n{}",
        context(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("docs/guides-removed-after-generation"),
        "verify must name the configured entry that does not resolve.\n{}",
        context(&output)
    );
    assert!(
        stdout.contains("Configured docs.snippets roots that do not exist"),
        "the finding must have its own section, distinct from coverage-ledger staleness.\n{}",
        context(&output)
    );

    let report_only = run_alef(&root, &["verify", "--report-only"]);
    assert!(
        report_only.status.success(),
        "--report-only must keep a successful exit status, as it does for every other verify \
         finding.\n{}",
        context(&report_only)
    );
    assert!(
        String::from_utf8_lossy(&report_only.stdout).contains("docs/guides-removed-after-generation"),
        "--report-only must still print the finding.\n{}",
        context(&report_only)
    );
}
