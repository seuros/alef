//! Cost of a multi-target `alef adopt` — and, more importantly, that lowering it changed no
//! decision.
//!
//! The safety net for the second half is [`should_reach_identical_decisions_when_targets_are_batched`]:
//! it runs one fixture twice, once through the per-target entry point and once through the
//! batch, and asserts the adopted set and the refused create-once-seed set are equal as sets.
//! A cost change that quietly altered adoption decisions would be far worse than a slow
//! command. ~keep

use super::*;
use crate::cli::commands::adopt::managed_outputs;
use crate::core::backend::GeneratedFile;

fn seed(base: &Path, relative: &str, content: &str) {
    let full = base.join(relative);
    std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
    std::fs::write(&full, content).expect("seed");
}

/// A neutral fixture with one path per rail the adopt decision distinguishes:
///
/// * `packages/one/manifest.toml` — marker rail, on-disk bytes already equal generated
///   output, so it converges.
/// * `packages/two/manifest.toml` — marker rail, on-disk bytes differ, so it drifts.
/// * `packages/three/suite_test.zig` — `generated_header: false`, a create-once seed whose
///   on-disk copy has grown past the placeholder. Must be refused without
///   `--clobber-create-once-seeds`.
fn fixture(base: &Path) -> Vec<ManagedOutput> {
    seed(base, "packages/one/manifest.toml", "name = \"one\"\n");
    seed(base, "packages/two/manifest.toml", "name = \"two-as-edited\"\n");
    seed(
        base,
        "packages/three/suite_test.zig",
        "test \"grown by hand\" {\n    try expect(true);\n}\n",
    );
    managed_outputs(
        &[
            GeneratedFile {
                path: PathBuf::from("packages/one/manifest.toml"),
                content: "name = \"one\"\n".to_owned(),
                generated_header: true,
            },
            GeneratedFile {
                path: PathBuf::from("packages/two/manifest.toml"),
                content: "name = \"two\"\n".to_owned(),
                generated_header: true,
            },
            GeneratedFile {
                path: PathBuf::from("packages/three/suite_test.zig"),
                content: "test \"placeholder\" {}\n".to_owned(),
                generated_header: false,
            },
        ],
        base,
    )
}

fn batch_options(base: &Path, write: bool) -> AdoptBatchOptions {
    AdoptBatchOptions {
        base_dir: base.to_path_buf(),
        write,
        converged_only: false,
        clobber_create_once_seeds: false,
    }
}

fn all_three_targets() -> Vec<String> {
    vec![
        "packages/one/manifest.toml".to_owned(),
        "packages/two/manifest.toml".to_owned(),
        "packages/three/suite_test.zig".to_owned(),
    ]
}

fn sorted(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = paths.into_iter().collect();
    all.sort();
    all.dedup();
    all
}

#[test]
fn should_classify_each_path_once_when_many_targets_share_one_invocation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let managed = fixture(dir.path());
    let targets = all_three_targets();

    let outcome = run_batch(&targets, &batch_options(dir.path(), false), &managed).expect("batch");

    assert_eq!(
        outcome.classification_passes, 3,
        "three targets selecting three distinct paths must read and classify exactly three files"
    );
    assert_eq!(outcome.results.len(), 3, "every target must be reported");
}

#[test]
fn should_classify_a_shared_path_once_when_several_targets_select_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let managed = fixture(dir.path());
    // Four targets, three of which resolve to the same single file: an explicit path, the
    // `./`-prefixed spelling adopt trims, and two globs that both cover it.
    let targets = vec![
        "packages/one/manifest.toml".to_owned(),
        "./packages/one/manifest.toml".to_owned(),
        "packages/one/*.toml".to_owned(),
        "packages/two/manifest.toml".to_owned(),
    ];

    let outcome = run_batch(&targets, &batch_options(dir.path(), false), &managed).expect("batch");

    assert_eq!(
        outcome.classification_passes, 2,
        "four targets covering two distinct files must classify two files, not four"
    );
    assert_eq!(outcome.results.len(), 4, "every target must still be reported");
}

#[test]
fn should_reach_identical_decisions_when_targets_are_batched() {
    // Deliberately a *mixed* glob rather than the bare seed path: a target that matches only
    // create-once seeds bails, and a bailed target contributes no report at all, so comparing
    // refusal sets across two empty lists would prove nothing. The glob refuses the seed
    // inside a report that succeeds. ~keep
    let targets = vec![
        "packages/one/manifest.toml".to_owned(),
        "packages/**/*".to_owned(),
        "packages/nowhere/absent.toml".to_owned(),
    ];

    let per_target_dir = tempfile::tempdir().expect("tempdir");
    let per_target_managed = fixture(per_target_dir.path());
    let mut per_target_adopted: Vec<PathBuf> = Vec::new();
    let mut per_target_refused: Vec<PathBuf> = Vec::new();
    let mut per_target_failures = 0_usize;
    for target in &targets {
        let options = AdoptOptions {
            target: target.clone(),
            base_dir: per_target_dir.path().to_path_buf(),
            write: true,
            converged_only: false,
            clobber_create_once_seeds: false,
        };
        match run_single(&options, &per_target_managed) {
            Ok(report) => {
                per_target_adopted.extend(report.adopted.iter().cloned());
                per_target_refused.extend(report.skipped_create_once.iter().cloned());
            }
            Err(_) => per_target_failures += 1,
        }
    }

    let batch_dir = tempfile::tempdir().expect("tempdir");
    let batch_managed = fixture(batch_dir.path());
    let outcome = run_batch(&targets, &batch_options(batch_dir.path(), true), &batch_managed).expect("batch");
    let batch_adopted: Vec<PathBuf> = outcome.reports().flat_map(|r| r.adopted.iter().cloned()).collect();
    let batch_refused: Vec<PathBuf> = outcome
        .reports()
        .flat_map(|r| r.skipped_create_once.iter().cloned())
        .collect();

    let per_target_adopted = sorted(per_target_adopted);
    let per_target_refused = sorted(per_target_refused);
    assert_eq!(
        per_target_adopted,
        vec![
            PathBuf::from("packages/one/manifest.toml"),
            PathBuf::from("packages/two/manifest.toml"),
        ],
        "the per-target baseline must itself adopt both marker-rail files"
    );
    assert_eq!(
        per_target_refused,
        vec![PathBuf::from("packages/three/suite_test.zig")],
        "the per-target baseline must itself refuse the seed, or the comparison below is vacuous"
    );
    assert_eq!(
        per_target_adopted,
        sorted(batch_adopted),
        "the adopted set must not change when targets are batched"
    );
    assert_eq!(
        per_target_refused,
        sorted(batch_refused),
        "the refused create-once-seed set must not change when targets are batched"
    );
    assert_eq!(
        per_target_failures,
        outcome.failures().count(),
        "the same targets must fail either way"
    );
    assert_eq!(per_target_failures, 1, "the unmatched target is the one that must fail");
}

#[test]
fn should_write_identical_bytes_when_targets_are_batched() {
    let targets = all_three_targets();
    let relative = Path::new("packages/one/manifest.toml");

    let per_target_dir = tempfile::tempdir().expect("tempdir");
    let per_target_managed = fixture(per_target_dir.path());
    for target in &targets {
        let options = AdoptOptions {
            target: target.clone(),
            base_dir: per_target_dir.path().to_path_buf(),
            write: true,
            converged_only: true,
            clobber_create_once_seeds: false,
        };
        let _ = run_single(&options, &per_target_managed);
    }

    let batch_dir = tempfile::tempdir().expect("tempdir");
    let batch_managed = fixture(batch_dir.path());
    let mut options = batch_options(batch_dir.path(), true);
    options.converged_only = true;
    let _ = run_batch(&targets, &options, &batch_managed).expect("batch");

    assert_eq!(
        std::fs::read_to_string(per_target_dir.path().join(relative)).expect("per-target bytes"),
        std::fs::read_to_string(batch_dir.path().join(relative)).expect("batch bytes"),
        "adoption must leave the same bytes on disk either way"
    );
}

#[test]
fn should_report_a_target_that_matches_nothing_without_cancelling_the_others() {
    let dir = tempfile::tempdir().expect("tempdir");
    let managed = fixture(dir.path());
    let targets = vec![
        "packages/nowhere/absent.toml".to_owned(),
        "packages/one/manifest.toml".to_owned(),
    ];

    let outcome = run_batch(&targets, &batch_options(dir.path(), true), &managed).expect("batch");

    let failed: Vec<&str> = outcome.failures().map(|(target, _)| target).collect();
    assert_eq!(failed, vec!["packages/nowhere/absent.toml"]);
    let adopted: Vec<PathBuf> = outcome.reports().flat_map(|r| r.adopted.iter().cloned()).collect();
    assert_eq!(adopted, vec![PathBuf::from("packages/one/manifest.toml")]);
}

#[test]
fn should_see_a_path_as_already_owned_when_an_earlier_target_stamped_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let managed = fixture(dir.path());
    let targets = vec![
        "packages/one/manifest.toml".to_owned(),
        "packages/one/*.toml".to_owned(),
    ];

    let outcome = run_batch(&targets, &batch_options(dir.path(), true), &managed).expect("batch");
    let reports: Vec<&AdoptReport> = outcome.reports().collect();

    assert_eq!(
        reports[0].adopted,
        vec![PathBuf::from("packages/one/manifest.toml")],
        "the first target stamps it"
    );
    assert!(
        reports[1].adopted.is_empty(),
        "the second target must not stamp it a second time, got {:?}",
        reports[1].adopted
    );
    assert_eq!(
        reports[1].already_owned,
        vec![PathBuf::from("packages/one/manifest.toml")],
        "the second target must see exactly what re-reading the stamped file shows"
    );
}

#[test]
fn should_match_the_documented_target_semantics() {
    let cases = [
        ("packages/one/manifest.toml", "packages/one/manifest.toml", true),
        ("./packages/one/manifest.toml", "packages/one/manifest.toml", true),
        ("packages/**/*.toml", "packages/one/manifest.toml", true),
        ("packages/*/manifest.toml", "packages/one/manifest.toml", true),
        ("packages/two/manifest.toml", "packages/one/manifest.toml", false),
        ("[unclosed", "packages/one/manifest.toml", false),
    ];
    for (target, relative, expected) in cases {
        let path = Path::new(relative);
        assert_eq!(
            TargetMatcher::new(target).matches(path),
            expected,
            "compiled matcher disagreed for target `{target}` against `{relative}`"
        );
        assert_eq!(
            crate::cli::commands::adopt::matches_target(target, path),
            expected,
            "matches_target disagreed for target `{target}` against `{relative}`"
        );
    }
}
