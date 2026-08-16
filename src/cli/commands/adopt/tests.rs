use super::*;
use crate::core::backend::GeneratedFile;
use crate::core::hash::content_has_alef_marker;

fn managed(relative: &str, content: &str, generated_header: bool, base: &Path) -> Vec<ManagedOutput> {
    managed_outputs(
        &[GeneratedFile {
            path: PathBuf::from(relative),
            content: content.to_owned(),
            generated_header,
        }],
        base,
    )
}

fn seed(base: &Path, relative: &str, content: &str) -> PathBuf {
    let full = base.join(relative);
    std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
    std::fs::write(&full, content).expect("seed");
    full
}

fn options(base: &Path, target: &str, write: bool) -> AdoptOptions {
    AdoptOptions {
        target: target.to_owned(),
        base_dir: base.to_path_buf(),
        write,
    }
}

/// The crawlberg case that motivates the command: a `.toml` manifest that has never
/// carried a marker in its history and has also drifted, so the write-time guard
/// refuses it on every run forever. Adoption must break that freeze.
#[test]
fn drifted_manifest_that_never_carried_a_marker_is_adopted_under_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let on_disk = "[package]\nname = \"crawlberg-ffi\"\n\n[dependencies]\nlibc = \"0.2\"\n";
    let target = seed(base, "crates/crawlberg-ffi/Cargo.toml", on_disk);
    let generated = "[package]\nname = \"crawlberg-ffi\"\n\n[dependencies]\nlibc = \"0.2\"\nserde = \"1\"\n";
    let outputs = managed("crates/crawlberg-ffi/Cargo.toml", generated, true, base);

    let report = run(&options(base, "crates/crawlberg-ffi/Cargo.toml", true), &outputs).expect("adopt");

    let after = std::fs::read_to_string(&target).expect("read after");
    assert!(
        content_has_alef_marker(&after),
        "adoption must leave a marker the guard will recognise, got:\n{after}"
    );
    assert_eq!(report.adopted, vec![PathBuf::from("crates/crawlberg-ffi/Cargo.toml")]);
    assert_eq!(report.diffs.len(), 1);
    assert_eq!(report.diffs[0].state, AdoptionState::Drifted);
}

/// NEGATIVE CONTROL, half one: adoption of a genuinely divergent file must not write
/// generated content. Adopt stamps the bytes that are already there and nothing else,
/// so the very content the diff warned about survives this command intact and is only
/// replaced later, by an ordinary `alef generate`, where `git diff` shows it.
///
/// This is the control that fails if adopt is ever "simplified" into writing
/// `candidate.generated` — the shape that would make every positive test above pass
/// while silently clobbering the consumer's file. ~keep
#[test]
fn adopting_a_drifted_file_stamps_it_without_replacing_a_single_body_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let on_disk = "[package]\nname = \"hand-tuned\"\n\n[dependencies]\nlibc = \"0.2\"\n";
    let target = seed(base, "crates/sample-ffi/Cargo.toml", on_disk);
    let generated = "[package]\nname = \"regenerated\"\n\n[dependencies]\nserde = \"1\"\n";
    let outputs = managed("crates/sample-ffi/Cargo.toml", generated, true, base);

    run(&options(base, "crates/sample-ffi/Cargo.toml", true), &outputs).expect("adopt");

    let after = std::fs::read_to_string(&target).expect("read after");
    assert_eq!(
        after,
        crate::cli::pipeline::ensure_generated_header(&target, on_disk),
        "adoption must be exactly the on-disk bytes plus a header -- nothing else may change"
    );
    assert!(
        after.contains("hand-tuned") && !after.contains("regenerated"),
        "adopt must never write generated content -- that is the clobber this guard exists to prevent, got:\n{after}"
    );
}

/// NEGATIVE CONTROL, half two: the diff is a required product of the command, not a
/// side effect of printing. A drifted file must yield a diff body that actually shows
/// both sides of the divergence.
///
/// Delete or stub the diff step and this fails: `report.diffs` goes empty, or its body
/// stops carrying the `-`/`+` lines. A test that only asserted on final file contents
/// would stay green through exactly that regression, which is why the rendered diff is
/// carried in [`AdoptReport`] rather than written straight to stdout. ~keep
#[test]
fn a_divergent_file_produces_a_full_diff_showing_both_sides() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    seed(
        base,
        "crates/sample-ffi/Cargo.toml",
        "[package]\nname = \"hand-tuned\"\n",
    );
    let outputs = managed(
        "crates/sample-ffi/Cargo.toml",
        "[package]\nname = \"regenerated\"\n",
        true,
        base,
    );

    let report = run(&options(base, "crates/sample-ffi/Cargo.toml", false), &outputs).expect("adopt preview");

    assert_eq!(report.diffs.len(), 1, "a divergent file must produce a diff");
    let body = &report.diffs[0].body;
    assert!(
        body.contains("-name = \"hand-tuned\""),
        "the diff must show the line adoption puts at risk, got:\n{body}"
    );
    assert!(
        body.contains("+name = \"regenerated\""),
        "the diff must show the line that would replace it, got:\n{body}"
    );
    assert_eq!(report.drifted().count(), 1, "divergence must be reported as drift");
}

/// The dry run is the default, and it must be inert: no marker, no content change, no
/// durable record. A human who types `alef adopt <path>` and walks away has consented
/// to nothing.
#[test]
fn preview_run_prints_a_diff_and_leaves_the_file_completely_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let on_disk = "[package]\nname = \"hand-tuned\"\n";
    let target = seed(base, "crates/sample-ffi/Cargo.toml", on_disk);
    let outputs = managed(
        "crates/sample-ffi/Cargo.toml",
        "[package]\nname = \"regenerated\"\n",
        true,
        base,
    );

    let report = run(&options(base, "crates/sample-ffi/Cargo.toml", false), &outputs).expect("adopt preview");

    assert_eq!(
        std::fs::read_to_string(&target).expect("read after"),
        on_disk,
        "a preview must leave the file byte-for-byte untouched"
    );
    assert!(report.preview);
    assert!(report.adopted.is_empty(), "a preview must adopt nothing");
    assert!(!report.diffs.is_empty(), "a preview must still produce the diff");
}

/// A converged file — identical to generated output apart from the header — is the case
/// an automatic predicate used to claim. It is still adoptable, but only here, and only
/// after the diff has been shown.
#[test]
fn converged_file_is_classified_as_converged_and_still_requires_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let body = "[package]\nname = \"sample-ffi\"\n";
    let target = seed(base, "crates/sample-ffi/Cargo.toml", body);
    let outputs = managed("crates/sample-ffi/Cargo.toml", body, true, base);

    let preview = run(&options(base, "crates/sample-ffi/Cargo.toml", false), &outputs).expect("preview");
    assert_eq!(preview.diffs[0].state, AdoptionState::Converged);
    assert_eq!(
        std::fs::read_to_string(&target).expect("read after preview"),
        body,
        "even a converged file is not stamped without --write"
    );

    let applied = run(&options(base, "crates/sample-ffi/Cargo.toml", true), &outputs).expect("apply");
    assert_eq!(applied.adopted.len(), 1);
    assert!(content_has_alef_marker(
        &std::fs::read_to_string(&target).expect("read after write")
    ));
}

/// Adopt must never become a general-purpose "stamp this file" tool. A path alef does
/// not generate is refused outright, whatever the human types.
#[test]
fn a_path_alef_does_not_generate_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    seed(base, "src/main.rs", "fn main() {}\n");
    let outputs = managed("crates/sample-ffi/Cargo.toml", "[package]\n", true, base);

    let error = run(&options(base, "src/main.rs", true), &outputs).expect_err("must refuse");

    assert!(
        error.to_string().contains("no alef-managed output matches"),
        "unexpected error: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(base.join("src/main.rs")).expect("read after"),
        "fn main() {}\n",
        "a refused target must be left untouched"
    );
}

/// A file that already carries a marker is not in the ownership trap at all. Adopt
/// reports it and produces no diff, so a glob that sweeps a healthy tree stays quiet.
#[test]
fn already_marked_file_is_reported_as_owned_and_produces_no_diff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let marked = format!(
        "{}[package]\nname = \"sample-ffi\"\n",
        crate::core::hash::header(crate::core::hash::CommentStyle::Hash)
    );
    seed(base, "crates/sample-ffi/Cargo.toml", &marked);
    let outputs = managed(
        "crates/sample-ffi/Cargo.toml",
        "[package]\nname = \"other\"\n",
        true,
        base,
    );

    let report = run(&options(base, "crates/sample-ffi/Cargo.toml", true), &outputs).expect("adopt");

    assert_eq!(
        report.already_owned,
        vec![PathBuf::from("crates/sample-ffi/Cargo.toml")]
    );
    assert!(report.diffs.is_empty());
    assert!(report.adopted.is_empty());
}

/// A format with no comment syntax at all (`.json`) cannot be stamped, so adoption
/// falls back to the committed `.alef-ownership.toml` record — the same proof route the
/// write-time guard already consults for unmarkable extensions.
#[test]
fn unstampable_format_is_adopted_through_the_durable_record_instead() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let on_disk = "{\n  \"name\": \"sample\"\n}\n";
    let target = seed(base, "packages/node/package.json", on_disk);
    assert!(!crate::cli::cache::is_scaffold_owned_path(base, &target));
    let outputs = managed(
        "packages/node/package.json",
        "{\n  \"name\": \"sample\",\n  \"version\": \"2\"\n}\n",
        true,
        base,
    );

    let report = run(&options(base, "packages/node/package.json", true), &outputs).expect("adopt");

    assert_eq!(
        std::fs::read_to_string(&target).expect("read after"),
        on_disk,
        "an unstampable file's bytes must not change: the record carries the ownership proof"
    );
    assert!(crate::cli::cache::is_scaffold_owned_path(base, &target));
    assert_eq!(
        report.recorded_unstampable,
        vec![PathBuf::from("packages/node/package.json")]
    );

    // The axis that matters and that `is_scaffold_owned_path` alone does not examine:
    // *where* the consent was written down. A human's adoption decision that only exists
    // inside the gitignored `.alef/` cache is not a decision the rest of the team, or CI,
    // can ever see — the operator reads the diff once and every other checkout still
    // refuses. Dropping the cache is what a fresh clone of their commit looks like. ~keep
    std::fs::remove_dir_all(base.join(".alef")).ok();
    assert!(
        base.join(".alef-ownership.toml").exists(),
        "adoption must leave its proof in a file the operator can commit"
    );
    assert!(
        crate::cli::cache::is_scaffold_owned_path(base, &target),
        "an adoption must survive into a checkout that never had the adopting machine's cache"
    );
}

/// A glob selects several managed paths in one invocation, and every one of them gets
/// its own full diff before anything is written.
#[test]
fn glob_target_diffs_every_match_before_adopting_any_of_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    seed(base, "crates/a-ffi/Cargo.toml", "[package]\nname = \"a\"\n");
    seed(base, "crates/b-ffi/Cargo.toml", "[package]\nname = \"b\"\n");
    let outputs = managed_outputs(
        &[
            GeneratedFile {
                path: PathBuf::from("crates/a-ffi/Cargo.toml"),
                content: "[package]\nname = \"a\"\nedition = \"2024\"\n".to_owned(),
                generated_header: true,
            },
            GeneratedFile {
                path: PathBuf::from("crates/b-ffi/Cargo.toml"),
                content: "[package]\nname = \"b\"\nedition = \"2024\"\n".to_owned(),
                generated_header: true,
            },
        ],
        base,
    );

    let report = run(&options(base, "crates/*-ffi/Cargo.toml", true), &outputs).expect("adopt");

    assert_eq!(report.diffs.len(), 2, "every match must be diffed");
    assert_eq!(report.adopted.len(), 2);
    for diff in &report.diffs {
        assert!(
            diff.body.contains("+edition = \"2024\""),
            "each match needs its own real diff, got:\n{}",
            diff.body
        );
    }
}

/// A target that matches managed output which does not exist on disk yet is refused
/// with a message pointing at `alef generate`: there is no ownership conflict to
/// resolve, and stamping a file into existence is not adoption.
#[test]
fn target_matching_only_absent_output_is_refused_with_generate_guidance() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let outputs = managed("crates/sample-ffi/Cargo.toml", "[package]\n", true, base);

    let error = run(&options(base, "crates/sample-ffi/Cargo.toml", true), &outputs).expect_err("must refuse");

    assert!(
        error.to_string().contains("nothing exists on disk yet"),
        "unexpected error: {error}"
    );
    assert!(!base.join("crates/sample-ffi/Cargo.toml").exists());
}
