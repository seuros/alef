//! The `is_owned_by_ownership_record` half of `classify`'s ownership union.
//!
//! Split out of `adopt::tests` to keep that file under the 1,000-line module cap.

use super::*;

/// The other half of the union `classify` now consults: a plain unmarkable path (not a
/// derived-output name) that a *previous* run already recorded in the committed
/// `.alef-ownership.toml`. Uses `config.m4` -- unmarkable per `marker_comment_style`, and
/// deliberately not one of `ALEF_DERIVED_OUTPUT_NAMES`, so this isolates the
/// `is_scaffold_owned_path` disjunct from the by-name `is_alef_derived_output` one the
/// ledger test above already covers.
///
/// The negative control matters as much as the positive one: without it, a `classify`
/// that ignored the record entirely and a `classify` that consulted it correctly would
/// both pass a test that only checked the recorded case. Asserting `Drifted` first proves
/// the record is genuinely gating the outcome, not merely present. ~keep
#[test]
fn classify_reports_already_owned_for_a_path_the_ownership_record_lists() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let relative = Path::new("generated/config.m4");
    let full = base.join(relative);
    let existing = "dnl old\n";
    let generated = "dnl new\n";
    seed(base, relative.to_str().expect("utf8 path"), existing);

    let before = super::classify(base, &full, relative, generated, existing, false);
    assert_eq!(
        before.state,
        super::AdoptionState::Drifted,
        "negative control: an unrecorded, unmarkable path with genuinely different content \
         must not be reported already owned -- otherwise this test cannot prove the record \
         is what flips the verdict below"
    );

    crate::cli::cache::record_scaffold_owned_path(base, &full).expect("record ownership");

    let after = super::classify(base, &full, relative, generated, existing, false);
    assert_eq!(
        after.state,
        super::AdoptionState::AlreadyOwned,
        "a path the committed ownership record already lists must never be re-offered for \
         adoption -- the write guard (`is_owned_by_ownership_record`) already accepts it"
    );
}
