//! Golden vectors pinning the stamp recipe to its recorded revision.
//!
//! Every `alef:hash:` value in every consumer repo is a function of the recipe in
//! [`super::compute_inputs_hash`] and [`super::compute_file_hash`]: the domain separators,
//! the field order, the `alef.toml` normalization, and
//! [`crate::core::template_versions::precommit::CODEGEN_FORMAT_VERSION`]. That constant is
//! the *recorded revision* of the recipe -- it is folded into `inputs_hash`, so bumping it
//! re-derives every stamp in the wild, and its doc carries the bump policy.
//!
//! What was missing was anything mechanical tying the recipe to the revision. A change to
//! the framing -- reordering two `update` calls, renaming a separator, adjusting the TOML
//! normalization -- silently invalidates every stamp ever written while every existing test
//! stays green, because they all compare the recipe against itself. The consumer then sees
//! files reported stale with no code change that explains it, which is exactly the
//! diagnosis that cost this investigation its first hours.
//!
//! These vectors are that tripwire. Editing the recipe fails them, and the only correct way
//! to make them pass is to recompute the vectors *and* bump `CODEGEN_FORMAT_VERSION` in the
//! same commit -- which is what makes the invalidation deliberate and dated rather than
//! silent. They are not a second implementation of the hash: a re-derivation would move in
//! lockstep with the code it is meant to pin. ~keep
//!
//! # Regenerating these vectors
//!
//! The per-file `alef:hash:` recipe changed (`compute_file_hash` no longer folds `inputs_hash`
//! in) and `CODEGEN_FORMAT_VERSION` was bumped 2 -> 3 in the same commit, exactly as this file's
//! contract requires. The vectors below were regenerated on 2026-08-27 by printing the real
//! `compute_inputs_hash`/`compute_file_hash` output for the fixtures above and copying it back.
//!
//! Never hand-derive these. Nobody can compute a blake3 digest by hand, and a fabricated vector
//! that happens to match a wrong implementation is exactly the "check that examined nothing"
//! failure this repo keeps hitting -- worse than a failing test, because it locks the bug in.
//! To regenerate after an intentional recipe change: print both values from a throwaway test,
//! copy them in, delete the throwaway. ~keep

use super::{compute_file_hash, compute_inputs_hash};
use crate::core::template_versions::precommit::CODEGEN_FORMAT_VERSION;

const FIXTURE_SOURCES_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const FIXTURE_ALEF_TOML: &[u8] = b"[workspace]\nlanguages = [\"python\"]\n";
const FIXTURE_FILE_BODY: &str = "fn fixture() {}\n";

/// The revision the vectors below were computed under. Asserted separately so a bump without
/// recomputed vectors, and recomputed vectors without a bump, both fail loudly. ~keep
const PINNED_CODEGEN_FORMAT_VERSION: &str = "3";

const GOLDEN_INPUTS_HASH: &str = "3b80c2438f94b045843fcd148c2128953c96dfcd6dbca82748d6633a3160d50f";
const GOLDEN_FILE_HASH: &str = "335f0c0982c5e42c05344f61b9f107bf072b2bdc778fd920c94b25cc02c57126";

#[test]
fn inputs_hash_recipe_matches_its_recorded_revision() {
    assert_eq!(
        CODEGEN_FORMAT_VERSION, PINNED_CODEGEN_FORMAT_VERSION,
        "CODEGEN_FORMAT_VERSION is the recorded revision of the stamp recipe; bumping it \
         re-derives every alef:hash: value in every consumer repo, so the golden vectors in \
         this file must be recomputed in the same commit"
    );
    assert_eq!(
        compute_inputs_hash(FIXTURE_SOURCES_HASH, FIXTURE_ALEF_TOML),
        GOLDEN_INPUTS_HASH,
        "the inputs-hash recipe changed without a CODEGEN_FORMAT_VERSION bump; every stamp \
         already written is now unverifiable and no regeneration explains why"
    );
}

#[test]
fn file_hash_recipe_matches_its_recorded_revision() {
    assert_eq!(
        compute_file_hash(FIXTURE_FILE_BODY),
        GOLDEN_FILE_HASH,
        "the per-file stamp recipe changed without a CODEGEN_FORMAT_VERSION bump; every stamp \
         already written is now unverifiable and no regeneration explains why"
    );
}
