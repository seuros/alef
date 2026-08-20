use super::*;
use crate::core::backend::GeneratedFile;
use std::collections::HashSet;
use std::path::PathBuf;

/// Regression for `alef test-apps generate` reporting drift on every run even though
/// the emitted content never changes: it writes through `write_scaffold_files_report`
/// (not `write_files_report`, which already has this coverage via
/// `test_finalize_hashes_is_idempotent_with_inputs_hash`) and then calls
/// `finalize_hashes` over the marker-carrying paths, exactly as
/// `bin_cli::aux_commands::TestAppsAction::Generate` does. That combination had no
/// dedicated test, so a regression in either half's idempotency could land unnoticed.
///
/// Simulates two full "clean-tree" runs of the same generator output: run the
/// write+finalize pair once to bootstrap the on-disk hash, then run the exact same
/// pair again over the unchanged generated content. The second pass must write
/// nothing at all -- neither a body rewrite (`write_scaffold_files_report`'s
/// `changed_count`) nor a hash-only rewrite (`finalize_hashes`'s return count) --
/// and the file bytes must be byte-for-byte identical before and after.
#[test]
fn write_then_finalize_is_idempotent_across_two_full_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();

    let file = GeneratedFile {
        path: PathBuf::from("test_apps/python/conftest.py"),
        content: "\"\"\"Pytest configuration for e2e tests.\"\"\"\n".to_string(),
        generated_header: true,
    };
    let full_path = base.join(&file.path);
    let sources_hash = "stable_sources_hash";
    let alef_toml_bytes = b"[workspace]\nlanguages = [\"python\"]\nalef_version = \"0.62.0\"\n";

    // Run 1: bootstrap. Body write, then hash finalize -- mirrors
    // `TestAppsAction::Generate`'s `write_scaffold_files_report` + `finalize_hashes` pair.
    let first_write = write_scaffold_files_report(&[file.clone()], base, true).expect("first write");
    assert_eq!(first_write.changed_count(), 1, "sanity: run 1 must create the file");

    let mut paths: HashSet<PathBuf> = HashSet::new();
    paths.insert(full_path.clone());
    let first_finalize = finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("first finalize");
    assert_eq!(first_finalize, 1, "sanity: run 1 must stamp the hash line");

    let after_run_one = std::fs::read_to_string(&full_path).expect("read after run 1");

    // Run 2: identical generated content, identical inputs, no cache -- exactly what a
    // second clean-tree `alef test-apps generate` invocation produces.
    let second_write = write_scaffold_files_report(&[file], base, true).expect("second write");
    assert_eq!(
        second_write.changed_count(),
        0,
        "run 2 must not rewrite a file whose body is unchanged"
    );

    let second_finalize = finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("second finalize");
    assert_eq!(
        second_finalize, 0,
        "run 2 must not rewrite the hash line when inputs and body are both unchanged"
    );

    let after_run_two = std::fs::read_to_string(&full_path).expect("read after run 2");
    assert_eq!(
        after_run_one, after_run_two,
        "two clean-tree runs over unchanged inputs must produce byte-identical output"
    );
}
