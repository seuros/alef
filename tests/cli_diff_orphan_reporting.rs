//! Regression for `alef diff` never reporting orphaned generated files, even though `alef
//! verify` does (`verify_orphans::find_orphaned_generated_files`,
//! `src/bin_cli/verify_orphans.rs`). A user previewing `alef diff` before committing to a real
//! `alef generate` could not see that regeneration would also leave a stale, alef-marked file
//! behind -- `alef diff` only ever unioned `pipeline::diff_files` over bindings/stubs/scaffold
//! (`Commands::Diff`, `src/bin_cli/core_commands.rs`), never the orphan sweep `alef verify`
//! already runs. Drives the real dispatch path through the real `alef` binary, not a direct
//! call into `find_orphaned_generated_files` (already unit-tested in `verify_orphans::tests`),
//! so this proves the CLI wiring, not just the diff logic.
//!
//! The assertions here deliberately do not require a byte-for-byte clean `alef diff` baseline:
//! `pipeline::diff_files` (`src/cli/pipeline/generate/diff.rs`) compares each in-memory
//! regenerated file against on-disk content that a prior `alef all` already ran through `poly
//! fmt`, and `diff_files` itself never re-applies that formatter before comparing TOML scaffold
//! files -- so a handful of scaffold entries (`poly.toml`, `rustfmt.toml`, the umbrella crate's
//! `Cargo.toml`) show up as "changed" on every run regardless of this fix. That is pre-existing,
//! unrelated `alef diff` noise, not something this test's fix touches, so causation here is
//! pinned on the specific orphan path string and the section header appearing/disappearing
//! around the plant/remove of one file, not on the overall diff being empty. ~keep

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn alef_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_alef"))
}

const FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";
const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"diff-orphan-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "diff-orphan-fixture"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.python]
module_name = "diff_orphan_fixture"

[crates.python.stubs]
output = "packages/python/diff_orphan_fixture"
"#;

fn write_fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

fn run_alef(root: &Path, args: &[&str]) -> Output {
    Command::new(alef_binary())
        .current_dir(root)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run alef {args:?}: {error}"))
}

#[test]
fn diff_reports_an_orphaned_generated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture(&root);

    let generated = run_alef(&root, &["all", "--skip-frb"]);
    assert!(
        generated.status.success(),
        "alef all must succeed against the fixture: {}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let baseline_diff = run_alef(&root, &["diff"]);
    assert!(
        baseline_diff.status.success(),
        "alef diff must succeed (exit 0) without --exit-code even when scaffold entries are \
         reported: {}",
        String::from_utf8_lossy(&baseline_diff.stderr)
    );
    let baseline_stdout = String::from_utf8_lossy(&baseline_diff.stdout);
    assert!(
        !baseline_stdout.contains("legacy_visitor.py"),
        "sanity: the orphan path must not appear before it is planted, got:\n{baseline_stdout}"
    );
    assert!(
        !baseline_stdout.contains("would be removed"),
        "sanity: no orphan section should be present before any orphan is planted, got:\n{baseline_stdout}"
    );

    // Simulate a backend that stopped emitting a file it used to (the same scenario
    // `verify_command_reports_and_fails_on_a_real_orphaned_generated_file` in
    // `src/bin_cli/core_commands/tests.rs` plants for `alef verify`): copy an existing
    // alef-marked file's real bytes -- header and hash intact -- to a path no current
    // backend's output would include.
    let current = root.join("packages/python/diff_orphan_fixture/api.py");
    let stale = root.join("packages/python/diff_orphan_fixture/legacy_visitor.py");
    assert!(
        current.is_file(),
        "sanity: fixture must have produced api.py for the orphan copy source"
    );
    fs::copy(&current, &stale).expect("plant a stale alef-marked file");

    let diff_with_orphan = run_alef(&root, &["diff"]);
    assert!(
        diff_with_orphan.status.success(),
        "alef diff without --exit-code must still exit 0 even when it finds an orphan: {}",
        String::from_utf8_lossy(&diff_with_orphan.stderr)
    );
    let stdout = String::from_utf8_lossy(&diff_with_orphan.stdout);
    assert!(
        stdout.contains("legacy_visitor.py"),
        "alef diff must name the orphaned generated file it found, got:\n{stdout}"
    );
    assert!(
        stdout.contains("would be removed"),
        "alef diff must report the orphan under its own section, distinct from \"Files that \
         would change\", got:\n{stdout}"
    );

    // `--exit-code` is diff's documented "fail if there would be changes" flag; its stdout must
    // include the same orphan finding the plain invocation above reported, proving the
    // exit-code path reads from the same orphan set rather than a separate, narrower one.
    let diff_exit_code = run_alef(&root, &["diff", "--exit-code"]);
    assert_eq!(
        diff_exit_code.status.code(),
        Some(1),
        "alef diff --exit-code must fail when an orphan is present: stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&diff_exit_code.stdout),
        String::from_utf8_lossy(&diff_exit_code.stderr)
    );
    let exit_code_stdout = String::from_utf8_lossy(&diff_exit_code.stdout);
    assert!(
        exit_code_stdout.contains("legacy_visitor.py"),
        "the --exit-code path must print the same orphan finding before exiting nonzero, got:\n{exit_code_stdout}"
    );

    fs::remove_file(&stale).expect("remove the planted orphan");
    let diff_after_cleanup = run_alef(&root, &["diff"]);
    assert!(
        diff_after_cleanup.status.success(),
        "alef diff must succeed once the orphan is removed: {}",
        String::from_utf8_lossy(&diff_after_cleanup.stderr)
    );
    let cleanup_stdout = String::from_utf8_lossy(&diff_after_cleanup.stdout);
    assert!(
        !cleanup_stdout.contains("legacy_visitor.py"),
        "alef diff must stop reporting the orphan once it is removed from disk, got:\n{cleanup_stdout}"
    );
    assert!(
        !cleanup_stdout.contains("would be removed"),
        "alef diff must drop the orphan section entirely once no orphan remains, got:\n{cleanup_stdout}"
    );
}
