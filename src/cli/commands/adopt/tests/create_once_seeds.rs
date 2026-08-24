//! What adoption of an **unmarkable** create-once seed does to the bytes on disk, measured
//! on both sides of the adoption and on both sides of the write that follows it.
//!
//! The shape under test is the one a consumer repo actually hit: `LICENSE`, `mvnw`,
//! `gradlew`, `.gitkeep` -- paths [`crate::cli::pipeline::marker_comment_style`] answers
//! `None` for, emitted `generated_header: false`, so
//! [`crate::cli::commands::adopt::is_create_once_seed`] is true and
//! `--clobber-create-once-seeds` is required to adopt them.
//!
//! Two statements about that case were on record and read as contradictory: the 0.66
//! changelog says such a path is "adopted through the committed record without its contents
//! being touched", while `--clobber-create-once-seeds`'s own help calls itself DANGEROUS
//! because "adopting one consents to alef replacing its contents with a placeholder seed on
//! the next generate". Both are true, and they are true of different moments --
//! [`adopting_an_unmarkable_create_once_seed_touches_none_of_its_bytes`] pins the first,
//! [`the_recorded_adoption_is_what_lets_the_next_overwriting_write_replace_the_seed`] pins
//! the second, and
//! [`without_the_adoption_the_identical_overwriting_write_refuses_and_the_seed_survives`] is
//! the control that shows the adoption is what changed the outcome rather than the
//! `overwrite: true` argument. Split across three tests rather than asserted in one so a
//! regression names which half broke. ~keep

use super::*;

/// A real, hand-maintained `LICENSE` -- the file a consumer commits once and never
/// regenerates. Nothing about it is alef's to rewrite.
const HAND_WRITTEN_LICENSE: &str = "MIT License\n\nCopyright (c) 2019 The Sample Authors\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software and associated documentation files.\n";

/// What alef's scaffold emits for the same path when it is absent: a placeholder with a
/// year and a holder nobody filled in.
const PLACEHOLDER_LICENSE: &str = "MIT License\n\nCopyright (c) <year> <copyright holder>\n";

/// `LICENSE` on disk plus the `GeneratedFile` alef would emit for it, so the same file set
/// can be handed to `adopt` and to the scaffold writer without the two drifting.
fn license_fixture(base: &Path) -> (PathBuf, Vec<GeneratedFile>) {
    let full = seed(base, "LICENSE", HAND_WRITTEN_LICENSE);
    let files = vec![GeneratedFile {
        path: PathBuf::from("LICENSE"),
        content: PLACEHOLDER_LICENSE.to_owned(),
        generated_header: false,
    }];
    (full, files)
}

/// The changelog's half: adoption of an unmarkable seed is a write to
/// `.alef-ownership.toml` and to nothing else. Not one byte of the seed changes, and
/// nothing generated is ever written into it.
#[test]
fn adopting_an_unmarkable_create_once_seed_touches_none_of_its_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let (full, files) = license_fixture(base);
    let outputs = managed_outputs(&files, base);
    assert!(
        outputs[0].create_once,
        "the fixture has to be on the gated rail, or this test proves nothing about the flag"
    );

    let report = run(&clobbering_seeds(base, "LICENSE"), &outputs).expect("adopt");

    assert_eq!(
        std::fs::read_to_string(&full).expect("read after"),
        HAND_WRITTEN_LICENSE,
        "adoption must not write the placeholder, add a marker, or reformat an unmarkable seed"
    );
    assert_eq!(report.recorded_unstampable, vec![PathBuf::from("LICENSE")]);
    assert!(
        crate::cli::cache::is_scaffold_owned_path(base, &full),
        "the consent has to land in the committed record -- it is the only place it can land"
    );
}

/// The flag help's half, and the reason the gate is not miscalibrated: that same record is
/// precisely what `write_scaffold_files_report` accepts as proof of ownership for an
/// unmarkable path, and `can_skip` (`!overwrite && !generated_header && ...`) is false under
/// the `overwrite: true` a routine `alef version` regen passes. So the very next such write
/// replaces the hand-maintained file with the placeholder -- far from the adoption, on an
/// unrelated command, with no diff in front of anyone. ~keep
#[test]
fn the_recorded_adoption_is_what_lets_the_next_overwriting_write_replace_the_seed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let (full, files) = license_fixture(base);
    let outputs = managed_outputs(&files, base);

    run(&clobbering_seeds(base, "LICENSE"), &outputs).expect("adopt");
    let write = crate::cli::pipeline::write_scaffold_files_report(&files, base, true).expect("scaffold write");

    assert_eq!(
        std::fs::read_to_string(&full).expect("read after"),
        PLACEHOLDER_LICENSE,
        "the adoption armed this write: the hand-maintained licence is gone and the placeholder \
         is in its place"
    );
    assert!(
        write.changed_paths.contains(&full),
        "the write must report the replacement it performed, got: {:?}",
        write.changed_paths
    );
    assert!(
        write.refused_paths.is_empty(),
        "nothing refused it -- that is the point: {:?}",
        write.refused_paths
    );
}

/// THE control. Byte-identical inputs, byte-identical write call, no adoption: the guard
/// refuses and the seed survives. Without this, the test above would pass just as happily if
/// `overwrite: true` alone were doing the damage, and the flag would be gating something it
/// does not control. ~keep
#[test]
fn without_the_adoption_the_identical_overwriting_write_refuses_and_the_seed_survives() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let (full, files) = license_fixture(base);

    let write = crate::cli::pipeline::write_scaffold_files_report(&files, base, true).expect("scaffold write");

    assert_eq!(
        std::fs::read_to_string(&full).expect("read after"),
        HAND_WRITTEN_LICENSE,
        "with no ownership record the guard is the only thing protecting this file, and it must hold"
    );
    assert!(
        write.refused_paths.contains(&full),
        "the refusal must be reported, got: {:?}",
        write.refused_paths
    );
    assert!(write.changed_paths.is_empty());
}

/// The timing half of the same claim, and the reason the warning names an *overwriting*
/// regen rather than "the next generate".
///
/// `can_skip` is `!overwrite && !generated_header && exists && !is_alef_derived_output`, and
/// it runs before the ownership guard and consults no ownership signal at all -- so an
/// ordinary `alef generate` leaves an adopted seed exactly where it is. An operator who read
/// "on the next generate", ran one, and saw the file untouched would reasonably conclude the
/// warning was false; the loss actually lands on the next `alef version` sync, days later.
/// Naming the wrong command is what makes a true warning dismissible. ~keep
#[test]
fn an_ordinary_non_overwriting_write_leaves_the_adopted_seed_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let (full, files) = license_fixture(base);
    let outputs = managed_outputs(&files, base);

    run(&clobbering_seeds(base, "LICENSE"), &outputs).expect("adopt");
    let write = crate::cli::pipeline::write_scaffold_files_report(&files, base, false).expect("scaffold write");

    assert_eq!(
        std::fs::read_to_string(&full).expect("read after"),
        HAND_WRITTEN_LICENSE,
        "a non-overwriting write skips a create-once seed whether or not it was adopted"
    );
    assert!(write.changed_paths.is_empty());
    assert!(
        write.refused_paths.is_empty(),
        "skipped, not refused: the guard is never even reached"
    );
}

/// The same finding stated as the property `alef adopt` gates on, so the gate cannot be
/// loosened without this failing: for a path alef cannot stamp, adoption's *only* effect is
/// the ownership record, and the ownership record is exactly what the write guard accepts.
/// A change that dropped `--clobber-create-once-seeds` for the unmarkable case would be
/// handing out that licence with no opt-in at all. ~keep
#[test]
fn an_unmarkable_seed_has_no_route_to_ownership_other_than_the_licence_the_write_guard_reads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let (full, _files) = license_fixture(base);

    assert!(
        crate::cli::pipeline::marker_comment_style(&full).is_none(),
        "fixture must be genuinely unmarkable, or the marker rail protects it regardless"
    );
    assert!(
        !crate::cli::pipeline::is_owned_by_ownership_record(base, &full),
        "no ownership before adoption"
    );
    assert!(
        crate::cli::pipeline::stamp_for_adoption(&full, HAND_WRITTEN_LICENSE).is_none(),
        "nothing can be stamped into it, so the record is the whole of the adoption"
    );
}
