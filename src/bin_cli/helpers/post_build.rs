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
    let resolved: Vec<(crate::core::config::Language, crate::core::backend::BuildConfig)> = languages
        .iter()
        .filter_map(|&language| {
            let backend = crate::cli::registry::try_get_backend(language)?;
            let build_config = backend.build_config_with_config(config)?;
            if build_config.post_build.is_empty() {
                return None;
            }
            Some((language, build_config))
        })
        .collect();
    run_resolved_post_builds(&resolved, languages.len(), config, base_dir)
}

/// Run post-build steps for an already-resolved `(language, build_config)` list, aggregating
/// failures across languages instead of aborting on the first one.
///
/// Split out of [`run_required_post_builds`] so the aggregation behavior (attempt every
/// language, report every failure) can be exercised directly against a caller-supplied
/// `BuildConfig` -- in particular so tests can substitute one language's `PostBuildStep` (e.g.
/// swap a `RunCommand`'s `cmd` for one that is never installed) without reaching into the
/// registry-resolved config that `run_required_post_builds` itself uses in production. This is
/// the same seam `run_post_build` already exposes one level down (it takes an explicit
/// `&BuildConfig` rather than resolving one itself); this function extends that seam to the
/// aggregation loop above it. `total_languages` is threaded through separately from
/// `resolved.len()` because it must match `run_required_post_builds`'s original denominator (the
/// full requested language count, including languages with no post-build step at all). ~keep
fn run_resolved_post_builds(
    resolved: &[(crate::core::config::Language, crate::core::backend::BuildConfig)],
    total_languages: usize,
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Result<()> {
    let mut failures: Vec<String> = Vec::new();
    for (language, build_config) in resolved {
        let language = *language;
        tracing::info!("  [{language}] running post-build...");
        match crate::cli::pipeline::run_post_build(language, build_config, config, base_dir) {
            Ok(outcome) if outcome.skipped_missing_tools.is_empty() => {
                tracing::info!("  [{language}] post-build processing complete");
            }
            // Non-fatal by design (falling back to committed generated output is the point --
            // see `run_run_command`'s doc comment), but a build that never actually ran a
            // required tool must not be reported identically to one that ran cleanly: that gap
            // is exactly what let a Dart post-build's skipped `flutter_rust_bridge_codegen`
            // masquerade as a passing run this fixes. ~keep
            Ok(outcome) => tracing::warn!(
                "  [{language}] post-build completed but skipped tool(s) not on PATH: {} -- \
                 falling back to committed generated files",
                outcome.skipped_missing_tools.join(", ")
            ),
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
        total_languages,
        failures.join("; ")
    );
}

#[cfg(test)]
mod tests {
    use super::{languages_have_post_build_steps, run_required_post_builds, run_resolved_post_builds};
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
    /// Dart's post-build would have reported for the same run, the same shape
    /// `e2e::run_generators`'s doc comment describes a consumer hitting for two days.
    ///
    /// Both languages here must fail regardless of host toolchains. Swift always fails because
    /// no build project exists in the temp dir (`cargo` is always present in this repo's own
    /// test environment). Dart's `RunCommand` step for the frb codegen tool is *not* usable
    /// as-is: whether it genuinely runs and errors, or is silently skipped because the tool
    /// isn't on `PATH` (`run_run_command`'s `NotFound` arm returns `Ok(false)`, not an error --
    /// see `PostBuildOutcome`), depends entirely on the host, and if it genuinely ran it would
    /// regenerate the bridge and defeat the "stale bridge" setup below. Forcing the skip via
    /// `ALEF_SKIP_COMMANDS` used to paper over that, but the var is process-global: this test's
    /// own `ENV_LOCK` only serialized against other holders of that *same* lock instance, not
    /// against `run_command_tests::env_lock()` in `build.rs`, which sets the identical var under
    /// a separate lock -- the exact two-locks-guarding-one-resource race `f968767b6` fixed for
    /// `frb_bridge_coverage.rs`'s equivalent test. The fix here is the same one applied there:
    /// route the RunCommand step through a command name that is never installed on any host, so
    /// `run_run_command` reports the deterministic `Ok(false)` skip without touching real
    /// process-global state at all, and no lock is needed. `run_resolved_post_builds` is the
    /// seam that makes that substitution possible -- it takes an explicit resolved
    /// `(language, BuildConfig)` list instead of resolving one from the registry internally, so
    /// Dart's registry-derived config can be cloned and have its `RunCommand` step's `cmd`
    /// swapped before the aggregation loop ever runs. A stale FRB bridge -- pre-seeded at
    /// Dart's real, registry-derived facade/bridge paths -- then makes `VerifyFrbBridgeCoverage`
    /// (a pure-Rust check with no external tool dependency) fail. Both failures must be named --
    /// proving the second language was actually attempted, not just that the error text happens
    /// to mention it. ~keep
    #[test]
    fn a_failing_language_does_not_abort_the_remaining_post_builds() {
        use crate::core::backend::PostBuildStep;
        use crate::core::config::ResolvedCrateConfig;

        let directory = tempfile::tempdir().expect("temporary project");
        let config = ResolvedCrateConfig::default();

        // Discover Dart's real facade/bridge paths (and full post-build steps) from its own
        // derived `BuildConfig` rather than duplicating that backend's internal path formula
        // here.
        let mut dart_build_config = crate::cli::registry::try_get_backend(Language::Dart)
            .and_then(|backend| backend.build_config_with_config(&config))
            .expect("Dart backend must produce a build config for the default crate config");
        let (facade_path, bridge_path) = dart_build_config
            .post_build
            .iter()
            .find_map(|step| match step {
                PostBuildStep::VerifyFrbBridgeCoverage {
                    facade_path,
                    bridge_path,
                    ..
                } => Some((facade_path.clone(), bridge_path.clone())),
                _ => None,
            })
            .expect("Dart's default post-build steps must include VerifyFrbBridgeCoverage");

        // Point the frb codegen `RunCommand` at a command name that is never installed on any
        // host -- see the test's doc comment for why this replaces `ALEF_SKIP_COMMANDS`.
        for step in &mut dart_build_config.post_build {
            if let PostBuildStep::RunCommand { cmd, .. } = step {
                *cmd = "alef-frb-codegen-intentionally-not-on-path-xyz789";
            }
        }

        let swift_build_config = crate::cli::registry::try_get_backend(Language::Swift)
            .and_then(|backend| backend.build_config_with_config(&config))
            .expect("Swift backend must produce a build config for the default crate config");

        // A facade that has grown a function the committed bridge never picked up -- the
        // alef #135 shape `VerifyFrbBridgeCoverage` exists to catch.
        let facade_file = directory.path().join(&facade_path);
        std::fs::create_dir_all(facade_file.parent().expect("facade path must have a parent")).unwrap();
        std::fs::write(
            &facade_file,
            "pub fn count_widgets(collection: String) -> Result<i64, String> {\n    Ok(0)\n}\n\
             pub fn record_price(id: String, price_cents: i64) -> Result<(), String> {\n    Ok(())\n}\n",
        )
        .unwrap();
        let bridge_file = directory.path().join(&bridge_path);
        std::fs::create_dir_all(bridge_file.parent().expect("bridge path must have a parent")).unwrap();
        std::fs::write(
            &bridge_file,
            "Future<int> countWidgets({required String collection}) => \
             RustLib.instance.api.crateCountWidgets(collection: collection);\n",
        )
        .unwrap();

        let resolved = [
            (Language::Swift, swift_build_config),
            (Language::Dart, dart_build_config),
        ];
        let result = run_resolved_post_builds(&resolved, resolved.len(), &config, directory.path());

        let error = result.expect_err("missing Swift build project and a stale Dart bridge must both fail");
        let message = error.to_string();
        assert!(message.contains("swift"), "got: {message}");
        assert!(message.contains("dart"), "got: {message}");
        assert!(message.contains("2 of 2"), "got: {message}");
    }
}
