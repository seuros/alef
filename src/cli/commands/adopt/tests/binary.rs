//! Adoption of alef's base64-encoded binary outputs.
//!
//! Split out of `adopt::tests` to keep that file under the 1,000-line module cap.

use super::*;
use base64::Engine;

const JAR_BYTES: &[u8] = &[0x50, 0x4b, 0x03, 0x04, 0x00, 0xff, 0xfe, 0x01];

fn encoded_jar() -> String {
    base64::engine::general_purpose::STANDARD.encode(JAR_BYTES)
}

fn seed_bytes(base: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
    let full = base.join(relative);
    std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
    std::fs::write(&full, bytes).expect("seed bytes");
    full
}

/// A binary match must not end the run for the text files beside it.
///
/// `packages/**` in a real repo sweeps up a `gradle-wrapper.jar`, and reading it as UTF-8 used
/// to fail the whole target with "stream did not contain valid UTF-8" -- before a single one of
/// the hundreds of adoptable text files under the same glob was stamped. ~keep
#[test]
fn a_binary_match_does_not_stop_the_text_matches_beside_it_from_being_adopted() {
    let base = tempfile::tempdir().expect("tempdir");
    let root = base.path();

    let mut outputs = managed("packages/demo/config.toml", "name = \"demo\"\n", true, root);
    outputs.extend(managed("packages/demo/wrapper.jar", &encoded_jar(), false, root));

    seed(root, "packages/demo/config.toml", "name = \"demo\"\n");
    let binary = seed_bytes(root, "packages/demo/wrapper.jar", &[0xff, 0xfe, 0x00, 0x01]);

    let report = run(&clobbering_seeds(root, "packages/**"), &outputs).expect("adopt must not abort on a binary match");

    assert!(report.unreadable.is_empty(), "a decodable jar is not unreadable");
    assert_eq!(
        std::fs::read(&binary).expect("binary still readable"),
        vec![0xff, 0xfe, 0x00, 0x01],
        "adoption never rewrites content -- the binary's bytes must be untouched"
    );
    assert!(
        content_has_alef_marker(&std::fs::read_to_string(root.join("packages/demo/config.toml")).expect("read")),
        "the text file under the same glob must still have been adopted"
    );
}

/// THE regression. A binary generated output had no route into ownership at all: `alef diff`
/// reported it as pending and `alef adopt` refused it as "not text", so the write guard refused
/// it forever and no command could ever change that. alef's own writers already guard binaries
/// with `is_scaffold_owned_path` and record them with `record_scaffold_owned_path`, so the
/// ownership rail existed -- only the door in was missing. ~keep
#[test]
fn a_drifted_binary_is_adopted_through_the_durable_ownership_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = "packages/kotlin-android/gradle/wrapper/gradle-wrapper.jar";
    let on_disk = [0xff, 0xfe, 0x00, 0x01];
    let target = seed_bytes(base, relative, &on_disk);
    assert!(!crate::cli::cache::is_scaffold_owned_path(base, &target));
    let outputs = managed(relative, &encoded_jar(), false, base);

    let report = run(&clobbering_seeds(base, relative), &outputs).expect("adopt");

    assert_eq!(
        std::fs::read(&target).expect("read after"),
        on_disk,
        "a binary's bytes must not change: the record carries the ownership proof"
    );
    assert!(
        crate::cli::cache::is_scaffold_owned_path(base, &target),
        "the guard's own ownership question must now answer yes"
    );
    assert_eq!(report.recorded_unstampable, vec![PathBuf::from(relative)]);
    assert_eq!(report.adopted, vec![PathBuf::from(relative)]);

    // The proof has to be committable, not cached: dropping `.alef/` is what a fresh clone
    // of the operator's commit looks like. ~keep
    std::fs::remove_dir_all(base.join(".alef")).ok();
    assert!(base.join(".alef-ownership.toml").exists());
    assert!(crate::cli::cache::is_scaffold_owned_path(base, &target));
}

/// A drifted binary is still only ever adopted after the operator is shown what is at risk.
/// There is no line diff for bytes, so the reviewable statement is size and digest per side --
/// which must actually distinguish the two sides, or it is a review of nothing.
///
/// A binary output satisfies neither criterion 2 nor criterion 3 of
/// [`crate::cli::cache::is_alef_derived_output`] -- `gradle-wrapper.jar` is not in alef's
/// reserved namespace and Gradle, not alef, is its reader -- so it stays a create-once seed
/// and its preview is reached through `--clobber-create-once-seeds`, exactly like every other
/// seed's. ~keep
#[test]
fn a_drifted_binary_reports_both_sides_by_size_and_digest_before_adoption() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = "packages/demo/wrapper.jar";
    seed_bytes(base, relative, &[0xff, 0xfe, 0x00, 0x01]);
    let outputs = managed(relative, &encoded_jar(), false, base);

    let report = run(&previewing_seeds(base, relative), &outputs).expect("adopt preview");

    let diff = report
        .diffs
        .first()
        .expect("a drifted binary must produce a reviewable diff");
    assert_eq!(diff.state, AdoptionState::Drifted);
    assert!(
        diff.body.contains("4 bytes") && diff.body.contains("8 bytes"),
        "both sides' sizes must be stated, got:\n{}",
        diff.body
    );
    assert!(
        diff.body
            .contains(&crate::core::hash::hash_bytes(&[0xff, 0xfe, 0x00, 0x01]))
            && diff.body.contains(&crate::core::hash::hash_bytes(JAR_BYTES)),
        "both sides' digests must be stated, got:\n{}",
        diff.body
    );
    assert!(report.adopted.is_empty(), "a preview writes nothing");
}

/// A binary already equal to what alef would write is converged, not drifted: there is no
/// content at risk, so it must not demand a review the operator cannot perform. ~keep
#[test]
fn a_binary_matching_its_generated_bytes_is_converged_not_drifted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = "packages/demo/wrapper.jar";
    seed_bytes(base, relative, JAR_BYTES);
    let outputs = managed(relative, &encoded_jar(), false, base);

    let report = run(&previewing_seeds(base, relative), &outputs).expect("adopt preview");

    assert_eq!(report.converged, vec![PathBuf::from(relative)]);
    assert!(report.diffs.is_empty());
}

/// The create-once refusal a binary still gets by default must name the way out. It is the
/// difference between an actionable refusal and the state this fix replaced, where the file was
/// reported by `alef diff` and rejected by `alef adopt` with no third command in existence. ~keep
#[test]
fn a_binary_seed_is_refused_by_default_with_the_flag_that_adopts_it_named() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = "packages/demo/wrapper.jar";
    seed_bytes(base, relative, &[0xff, 0xfe, 0x00, 0x01]);
    let outputs = managed(relative, &encoded_jar(), false, base);

    let error = run(&options(base, relative, true), &outputs).expect_err("a bare --write must refuse a seed");

    assert!(
        format!("{error:#}").contains("--clobber-create-once-seeds"),
        "the refusal must name the flag that resolves it, got: {error:#}"
    );
}

/// The negative control on the widened rail: only *alef's* binary formats take the byte route.
/// A path alef emits as text whose on-disk bytes are not UTF-8 is still something alef can say
/// nothing about, and must keep being refused rather than quietly recorded as owned. ~keep
#[test]
fn a_non_binary_output_with_invalid_utf8_bytes_is_still_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = "packages/demo/config.toml";
    let target = seed_bytes(base, relative, &[0xff, 0xfe, 0x00, 0x01]);
    let outputs = managed(relative, "name = \"demo\"\n", true, base);

    let report = run(&options(base, relative, true), &outputs).expect("adopt");

    assert_eq!(report.unreadable, vec![PathBuf::from(relative)]);
    assert!(report.adopted.is_empty());
    assert!(!crate::cli::cache::is_scaffold_owned_path(base, &target));
}
