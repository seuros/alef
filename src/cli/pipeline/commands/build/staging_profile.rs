//! Which build artifact a staging post-build step should look for, and whether its absence is
//! worth telling the operator about.
//!
//! Split out of `build.rs` (16 lines under this repo's 1,000-line cap when this concern was
//! extracted) rather than grown in place.

use crate::publish::package::BuildProfile;

/// Which build profile [`crate::core::backend::PostBuildStep::StageFfiLibrary`] and
/// [`crate::core::backend::PostBuildStep::StageDartNatives`] should look for, and whether a
/// build was expected to have produced one.
///
/// `run_post_build` runs from callers with different guarantees about what was just built, and
/// those guarantees answer two separate questions:
///
/// 1. *Which* artifact to stage. `build_with_environment`'s own two dispatch loops call
///    `run_post_build` immediately after running exactly one cargo profile for this invocation,
///    so staging must look at that same profile ([`Self::JustBuilt`]) -- a stale artifact from
///    the *other* profile left over from an earlier, unrelated run must never silently satisfy
///    this run's staging step. The other two variants never invoke `cargo build` at all, so
///    neither can name a profile; they ask for whichever is already on disk, release preferred.
///    Neither ever consults `deps/` -- see [`crate::publish::package::find_built_artifact`]'s
///    doc comment for why.
/// 2. Whether a *missing* artifact is news. This is the distinction [`Self::PreferOnDisk`] and
///    [`Self::NoBuildRequested`] exist to draw, and it is not derivable from the profile: both
///    look in the same places. `alef test --e2e` stages before running suites that link the
///    native library, so nothing on disk means the build the operator was supposed to have run
///    never happened -- a warning naming the fix. `alef generate`/`alef all` are contractually
///    no-build commands: they never ask for a cdylib, so its absence on a fresh checkout is the
///    expected state, and warning about it advises a build the invoked command never intended
///    to perform. Both used to be `PreferOnDisk`, which made every generation-only run on an
///    unbuilt tree emit one unavoidable "run `alef build --release`" warning per FFI-dependent
///    language. ~keep
#[derive(Debug, Clone, Copy)]
pub enum StagingProfile {
    /// This run just built exactly this profile; stage that artifact and no other.
    JustBuilt(BuildProfile),
    /// No build this run, but one was expected to have already happened (`alef test --e2e`).
    /// A missing artifact is a warning.
    PreferOnDisk,
    /// No build was requested at all (`alef generate`/`alef all`'s post-build pass). A missing
    /// artifact is the ordinary state of a tree that has not been built and is logged at
    /// `DEBUG`, never warned about.
    NoBuildRequested,
}

impl StagingProfile {
    /// The profile this run just built, if it built one -- `None` means "whichever is on disk,
    /// release preferred", for both no-build variants.
    pub(crate) fn just_built(self) -> Option<BuildProfile> {
        match self {
            Self::JustBuilt(profile) => Some(profile),
            Self::PreferOnDisk | Self::NoBuildRequested => None,
        }
    }

    /// Whether the caller expected a build to have produced the artifact being staged, and so
    /// whether its absence is worth a warning.
    pub(crate) fn build_was_expected(self) -> bool {
        !matches!(self, Self::NoBuildRequested)
    }
}

#[cfg(test)]
mod tests {
    use super::StagingProfile;
    use crate::publish::package::BuildProfile;

    /// The two no-build variants must look in the same places -- the only thing that separates
    /// them is whether a miss is news. If `just_built` ever started distinguishing them, the
    /// generate path would silently stop staging an artifact a previous build did leave behind.
    #[test]
    fn both_no_build_variants_look_for_whichever_profile_is_on_disk() {
        assert!(StagingProfile::PreferOnDisk.just_built().is_none());
        assert!(StagingProfile::NoBuildRequested.just_built().is_none());
    }

    #[test]
    fn just_built_names_the_profile_this_run_produced() {
        assert_eq!(
            StagingProfile::JustBuilt(BuildProfile::Release).just_built(),
            Some(BuildProfile::Release)
        );
    }

    /// The gate the missing-artifact warning asks. `alef test --e2e` (`PreferOnDisk`) is about
    /// to link the library, so a miss is a real diagnostic; a generation-only run never asked
    /// for one.
    #[test]
    fn only_the_no_build_requested_variant_treats_a_missing_artifact_as_expected() {
        assert!(StagingProfile::JustBuilt(BuildProfile::Debug).build_was_expected());
        assert!(StagingProfile::PreferOnDisk.build_was_expected());
        assert!(!StagingProfile::NoBuildRequested.build_was_expected());
    }
}
