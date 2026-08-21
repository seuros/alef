//! Running every language's required post-build step.
//!
//! Split out of `helpers.rs` (already at this repo's 1,000-line cap) rather than left inline.

use anyhow::Result;

/// Run every language's required post-build step, isolating one language's failure from
/// every other language's.
///
/// ~keep This used to propagate the first failure with `?` immediately, which made one
/// language's post-build break abort every later-listed language's post-build too -- even
/// though each is an independent `cargo build` (or equivalent) with no dependency on the
/// others having succeeded. `e2e::run_generators` hit the identical shape first (see its own
/// doc comment: a consumer's C backend `bail!` silently starved every later e2e backend and
/// the snippet stage for two days) and was fixed to attempt every backend regardless of
/// earlier failures, reporting all of them once every backend that could run has. This mirrors
/// that fix for `alef generate`'s post-build phase: a Swift codegen defect that fails `cargo
/// build` must not also hide whatever Kotlin/Android, Wasm, or Dart's post-build would have
/// reported for the same run.
/// Whether `language` has any post-build step configured (`build_config_with_config(config)`
/// resolves and `post_build` is non-empty) -- the same predicate [`run_required_post_builds`]
/// uses to decide whether to run one, factored out so a caller can ask the question without
/// running anything.
fn language_has_post_build_steps(
    language: crate::core::config::Language,
    config: &crate::core::config::ResolvedCrateConfig,
) -> bool {
    crate::cli::registry::try_get_backend(language)
        .and_then(|backend| backend.build_config_with_config(config))
        .is_some_and(|build_config| !build_config.post_build.is_empty())
}

/// Whether any of `languages` has a post-build step configured for `config`.
///
/// `run_required_post_builds` runs unconditionally on every `alef all`/`alef generate` pass --
/// unlike bindings/stubs/scaffold/docs, its steps (`RunCommand`, e.g.
/// `flutter_rust_bridge_codegen` for Dart) write directly to disk via an external tool, with no
/// `WriteReport` to say whether the bytes actually changed. That makes it invisible to any
/// "did this run change anything" signal built from write reports alone -- the format-gating
/// bug this fixes (alef #119) had exactly that shape for Dart's frb output. Since the step
/// always runs and alef cannot see whether it rewrote anything, treating its mere presence as
/// "output may have changed" is the only sound default: a false-positive costs one extra
/// (idempotent) formatting pass, a false-negative ships unformatted output forever. ~keep
pub(crate) fn languages_have_post_build_steps(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
) -> bool {
    languages
        .iter()
        .any(|&language| language_has_post_build_steps(language, config))
}

pub(super) fn run_required_post_builds(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Result<()> {
    let mut failures: Vec<String> = Vec::new();
    for &language in languages {
        let Some(backend) = crate::cli::registry::try_get_backend(language) else {
            continue;
        };
        let Some(build_config) = backend.build_config_with_config(config) else {
            continue;
        };
        if build_config.post_build.is_empty() {
            continue;
        }
        tracing::info!("  [{language}] running post-build...");
        match crate::cli::pipeline::run_post_build(language, &build_config, config, base_dir) {
            Ok(()) => tracing::info!("  [{language}] post-build processing complete"),
            Err(error) => {
                tracing::warn!("[{language}] post-build failed, continuing with remaining languages: {error:#}");
                failures.push(format!("[{language}] {error:#}"));
            }
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "post-build failed for {} of {} language(s): {}",
        failures.len(),
        languages.len(),
        failures.join("; ")
    );
}

#[cfg(test)]
mod tests {
    use super::{languages_have_post_build_steps, run_required_post_builds};
    use crate::core::config::Language;

    /// A language with a real, always-runs post-build (see `run_post_build`'s
    /// `Language::Swift` arm) must be detected -- this is the "Dart frb" shape from
    /// alef #119: `languages_have_post_build_steps` is what lets the format-gate in
    /// `alef all` (`all_commands.rs`) treat such a language's presence as "output may
    /// have changed" even though the post-build step itself never reports a changed
    /// count.
    #[test]
    fn detects_a_language_with_a_configured_post_build_step() {
        assert!(languages_have_post_build_steps(
            &[Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default()
        ));
    }

    /// A language with no post-build step (pyo3's `BuildConfig::post_build` is `vec![]`)
    /// must not be reported as having one -- otherwise every `alef all` run would treat
    /// every language as "may have changed", defeating the point of a targeted signal.
    #[test]
    fn reports_false_for_a_language_with_no_post_build_step() {
        assert!(!languages_have_post_build_steps(
            &[Language::Python],
            &crate::core::config::ResolvedCrateConfig::default()
        ));
    }

    /// One post-build-bearing language among several others without one is enough --
    /// mirrors a real multi-language `alef all` run where only one target (e.g. Dart)
    /// has a post-build step.
    #[test]
    fn detects_a_post_build_language_mixed_with_languages_that_have_none() {
        assert!(languages_have_post_build_steps(
            &[Language::Python, Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default()
        ));
    }

    /// An empty language list has nothing to check and must not spuriously report a
    /// post-build step.
    #[test]
    fn reports_false_for_an_empty_language_list() {
        assert!(!languages_have_post_build_steps(
            &[],
            &crate::core::config::ResolvedCrateConfig::default()
        ));
    }

    #[test]
    fn required_post_build_failure_is_propagated_with_language_context() {
        let directory = tempfile::tempdir().expect("temporary project");
        let error = run_required_post_builds(
            &[Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default(),
            directory.path(),
        )
        .expect_err("missing Swift build project must fail");

        assert!(error.to_string().contains("swift"));
    }

    /// One language's post-build failure used to abort the loop via `?` before any later
    /// language's post-build ran at all -- so a Swift codegen defect silently hid whatever
    /// Dart's post-build (`flutter_rust_bridge_codegen`, a `RunCommand` step with no
    /// precondition gate of its own, so it genuinely runs and fails rather than being skipped)
    /// would have reported for the same run, the same shape `e2e::run_generators`'s doc comment
    /// describes a consumer hitting for two days. Both languages here fail (no build project
    /// exists in the temp dir for either -- Dart's default style is FRB, see
    /// `dart_style_defaults_to_frb`), and both failures must be named -- proving the second
    /// language was actually attempted, not just that the error text happens to mention it.
    /// ~keep
    #[test]
    fn a_failing_language_does_not_abort_the_remaining_post_builds() {
        let directory = tempfile::tempdir().expect("temporary project");
        let error = run_required_post_builds(
            &[Language::Swift, Language::Dart],
            &crate::core::config::ResolvedCrateConfig::default(),
            directory.path(),
        )
        .expect_err("missing build projects for both languages must fail");

        let message = error.to_string();
        assert!(message.contains("swift"), "got: {message}");
        assert!(message.contains("dart"), "got: {message}");
        assert!(message.contains("2 of 2"), "got: {message}");
    }
}
