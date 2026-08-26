//! Orchestration for [`crate::core::backend::PostBuildStep::VerifyFrbBridgeCoverage`]: reads the
//! FRB facade and the flutter_rust_bridge-generated bridge off disk and fails the build loudly
//! when the bridge is missing a function the facade declares.
//!
//! Split out of `build.rs` (already at this repo's 1,000-line cap) rather than left inline —
//! see alef #135 for the shape this closes: `PostBuildStep::RunCommand`'s missing-tool/
//! `ALEF_SKIP_COMMANDS` fallback treats a skipped `flutter_rust_bridge_codegen` invocation as
//! success, and the `PostProcessFile` steps that follow it in the same post-build sequence
//! patch whatever bridge Dart source is already on disk regardless of whether frb actually
//! regenerated it this run. [`verify`] is the gate `run_post_build` calls between those two:
//! a stale bridge now aborts the post-build sequence for this language instead of receiving
//! alef's patches on top of missing functions.
//!
//! [`verify`] also resolves the enabled-feature set flutter_rust_bridge's own codegen macro
//! expansion would have used -- the Rust crate's own `[features] default = [...]`, read from
//! `facade_file`'s sibling `Cargo.toml` (`facade_file` is always `<rust crate dir>/src/lib.rs`) --
//! and passes it through so [`crate::backends::dart::missing_bridge_functions`] never reports a
//! `#[cfg(...)]`-gated facade function frb correctly never saw, alongside every function it
//! genuinely dropped.

use anyhow::Context as _;
use std::collections::HashSet;
use std::path::Path;

/// Read `facade_file` and `bridge_file` and return an error naming every facade free function
/// (outside `exclude_functions`, and reachable given the sibling manifest's default features)
/// that has no matching function in the bridge.
///
/// A no-op when either file does not exist yet — mirrors every other `PostBuildStep` handler in
/// `run_post_build`, which treats "nothing to check yet" as fine rather than an error (a
/// project's first-ever generation has not produced a bridge to compare against).
pub(super) fn verify(facade_file: &Path, bridge_file: &Path, exclude_functions: &[String]) -> anyhow::Result<()> {
    if !facade_file.exists() || !bridge_file.exists() {
        tracing::debug!(
            "VerifyFrbBridgeCoverage: facade or bridge not found ({} / {})",
            facade_file.display(),
            bridge_file.display()
        );
        return Ok(());
    }

    let facade_source = std::fs::read_to_string(facade_file)
        .with_context(|| format!("failed to read FRB facade {}", facade_file.display()))?;
    let bridge_source = std::fs::read_to_string(bridge_file)
        .with_context(|| format!("failed to read FRB bridge {}", bridge_file.display()))?;

    // `facade_file` is always `<rust crate dir>/src/lib.rs` (see
    // `backends::dart::gen_bindings::frb_rust_facade_paths`), so its manifest is two directories
    // up. A missing/unparseable manifest resolves to `None`, and `missing_bridge_functions`
    // degrades to the pre-cfg-awareness blanket check rather than exempting every gated function
    // from coverage.
    let manifest_path = facade_file
        .parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("Cargo.toml"));
    let enabled_features = manifest_path
        .as_deref()
        .and_then(crate::codegen::cfg::read_default_enabled_cargo_features);
    let enabled_features_refs: Option<HashSet<&str>> = enabled_features
        .as_ref()
        .map(|set| set.iter().map(String::as_str).collect());

    let missing = crate::backends::dart::missing_bridge_functions(
        &facade_source,
        &bridge_source,
        exclude_functions,
        enabled_features_refs.as_ref(),
    );
    if missing.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "flutter_rust_bridge bridge {} has {} facade function(s) from {} with no matching entry: \
         {}. Each is reachable per this crate's manifest (ungated, or its `#[cfg(feature = ...)]` \
         is in the manifest's default features), so the gap has one of several possible causes: \
         flutter_rust_bridge_codegen did not (re)generate this bridge against the current facade; \
         the manifest's default feature set does not actually match what the codegen run used; or \
         the function's gate depends on a non-feature predicate (target_os, ...) this check cannot \
         evaluate and treats as reachable. Determine which applies, then either rerun \
         flutter_rust_bridge_codegen or update the committed bridge source -- alef's post-build \
         patches must not be applied to a stale bridge.",
        bridge_file.display(),
        missing.len(),
        facade_file.display(),
        missing.join(", "),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACADE_ONE_FUNCTION: &str =
        "pub fn count_widgets(collection: String) -> Result<i64, String> {\n    Ok(0)\n}\n";
    const BRIDGE_COVERING_IT: &str = "Future<int> countWidgets({required String collection}) => RustLib.instance.api.crateCountWidgets(collection: collection);\n";

    #[test]
    fn verify_passes_when_bridge_covers_every_facade_function() {
        let dir = tempfile::tempdir().expect("temp dir");
        let facade = dir.path().join("lib.rs");
        let bridge = dir.path().join("lib.dart");
        std::fs::write(&facade, FACADE_ONE_FUNCTION).unwrap();
        std::fs::write(&bridge, BRIDGE_COVERING_IT).unwrap();

        assert!(verify(&facade, &bridge, &[]).is_ok());
    }

    #[test]
    fn verify_fails_when_the_bridge_is_stale_relative_to_the_facade() {
        let dir = tempfile::tempdir().expect("temp dir");
        let facade = dir.path().join("lib.rs");
        let bridge = dir.path().join("lib.dart");
        // The facade gained a second function that the (stale) bridge never picked up --
        // the alef #135 shape: frb was skipped this pass and the bridge on disk predates
        // `record_price`.
        std::fs::write(
            &facade,
            "pub fn count_widgets(collection: String) -> Result<i64, String> {\n    Ok(0)\n}\n\
             pub fn record_price(id: String, price_cents: i64) -> Result<(), String> {\n    Ok(())\n}\n",
        )
        .unwrap();
        std::fs::write(&bridge, BRIDGE_COVERING_IT).unwrap();

        let error = verify(&facade, &bridge, &[]).expect_err("stale bridge must fail the check");
        let message = format!("{error:#}");
        assert!(
            message.contains("record_price"),
            "error must name the missing function: {message}"
        );
    }

    #[test]
    fn verify_ignores_configured_exclusions() {
        let dir = tempfile::tempdir().expect("temp dir");
        let facade = dir.path().join("lib.rs");
        let bridge = dir.path().join("lib.dart");
        std::fs::write(
            &facade,
            "pub fn count_widgets(collection: String) -> Result<i64, String> {\n    Ok(0)\n}\n\
             pub fn internal_only(id: String) -> Result<(), String> {\n    Ok(())\n}\n",
        )
        .unwrap();
        std::fs::write(&bridge, BRIDGE_COVERING_IT).unwrap();

        assert!(verify(&facade, &bridge, &["internal_only".to_string()]).is_ok());
    }

    #[test]
    fn verify_is_a_no_op_when_the_bridge_does_not_exist_yet() {
        let dir = tempfile::tempdir().expect("temp dir");
        let facade = dir.path().join("lib.rs");
        let bridge = dir.path().join("lib.dart");
        std::fs::write(&facade, FACADE_ONE_FUNCTION).unwrap();

        assert!(verify(&facade, &bridge, &[]).is_ok());
    }

    /// The real `alef all` failure this fix closes: a facade function gated behind a feature the
    /// sibling `Cargo.toml` does not enable by default must not be reported missing, because
    /// flutter_rust_bridge's own codegen macro expansion never saw it either. Uses the real
    /// `<rust crate dir>/src/lib.rs` + `<rust crate dir>/Cargo.toml` layout `verify` resolves the
    /// manifest from, not the flat `dir.path()` layout the other tests in this module use for
    /// their cfg-blind fixtures.
    #[test]
    fn verify_ignores_a_facade_function_behind_an_inactive_cfg_gate() {
        let dir = tempfile::tempdir().expect("temp dir");
        let rust_dir = dir.path().join("rust");
        std::fs::create_dir_all(rust_dir.join("src")).unwrap();
        let facade = rust_dir.join("src/lib.rs");
        let bridge = dir.path().join("lib.dart");
        std::fs::write(
            &facade,
            "#[cfg(feature = \"premium-tier\")]\n\
             pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {\n    Ok(json)\n}\n",
        )
        .unwrap();
        std::fs::write(
            rust_dir.join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\n\n\
             [features]\ndefault = []\npremium-tier = [\"sample-core/premium-tier\"]\n",
        )
        .unwrap();
        // The bridge has nothing for this function -- exactly what a correct frb run produces
        // when the gating feature is off.
        std::fs::write(&bridge, "").unwrap();

        assert!(
            verify(&facade, &bridge, &[]).is_ok(),
            "a facade function behind a feature the manifest does not enable by default must not \
             fail the coverage check"
        );
    }

    /// Negative control for the test above: a facade function whose gate IS in the manifest's
    /// default features, but that is genuinely absent from the bridge, must still fail the check.
    /// Without this control, a fix that stopped evaluating cfg gates at all (treating every gated
    /// function as inactive) would pass the positive test above while quietly breaking the
    /// coverage check's entire purpose.
    #[test]
    fn verify_still_fails_when_a_facade_function_under_an_active_gate_is_missing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let rust_dir = dir.path().join("rust");
        std::fs::create_dir_all(rust_dir.join("src")).unwrap();
        let facade = rust_dir.join("src/lib.rs");
        let bridge = dir.path().join("lib.dart");
        std::fs::write(
            &facade,
            "#[cfg(feature = \"premium-tier\")]\n\
             pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {\n    Ok(json)\n}\n",
        )
        .unwrap();
        std::fs::write(
            rust_dir.join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\n\n\
             [features]\ndefault = [\"premium-tier\"]\npremium-tier = [\"sample-core/premium-tier\"]\n",
        )
        .unwrap();
        // The bridge is stale: the gate IS active by default, so frb should have bridged this
        // function, and did not.
        std::fs::write(&bridge, "").unwrap();

        let error = verify(&facade, &bridge, &[]).expect_err("a missing function under an active gate must still fail");
        let message = format!("{error:#}");
        assert!(
            message.contains("create_premium_backend_options_from_json"),
            "error must name the missing function: {message}"
        );
    }

    /// End-to-end proof that `run_post_build` itself refuses to patch a stale bridge: when the
    /// `RunCommand` step for the frb codegen tool is skipped (here because the configured
    /// command name does not exist on `PATH` -- `run_run_command` reports that as `Ok(false)`
    /// deterministically, with no process-global state involved) and the facade has since
    /// gained a function the committed bridge lacks, the whole post-build sequence must error
    /// out before the `PostProcessFile` step that follows ever touches the bridge file.
    ///
    /// Deliberately does not use the real `flutter_rust_bridge_codegen` command name gated by
    /// `ALEF_SKIP_COMMANDS`: that env var is process-global, and a test-local lock around
    /// setting it only serializes against other holders of that *same* lock instance -- it does
    /// nothing against the crate's other test modules that set the same variable under their own
    /// separate lock (see `run_command_tests::env_lock` in `build.rs`), so two lock instances
    /// guarding one process resource race exactly like the unguarded `current_dir()` callers did
    /// for `CWD_LOCK`. A command name that is simply never installed sidesteps the shared
    /// resource entirely. ~keep
    #[test]
    fn run_post_build_aborts_before_patching_a_stale_bridge_when_frb_is_skipped() {
        use crate::core::backend::{BuildConfig, BuildDependency, PostBuildStep, PostProcessor};
        use crate::core::config::{Language, ResolvedCrateConfig};

        let dir = tempfile::tempdir().expect("temp dir");
        let facade_rel = std::path::PathBuf::from("lib.rs");
        let bridge_rel = std::path::PathBuf::from("lib.dart");
        std::fs::write(
            dir.path().join(&facade_rel),
            "pub fn count_widgets(collection: String) -> Result<i64, String> {\n    Ok(0)\n}\n\
             pub fn record_price(id: String, price_cents: i64) -> Result<(), String> {\n    Ok(())\n}\n",
        )
        .unwrap();
        // Stale committed bridge: predates `record_price`. Carries a trailing space so that
        // if the `DartStripTrailingWhitespace` step below were (wrongly) reached, its effect on
        // this file would be observable.
        let stale_bridge_with_trailing_whitespace = "Future<int> countWidgets({required String collection}) => \nRustLib.instance.api.crateCountWidgets(collection: collection); \n";
        std::fs::write(dir.path().join(&bridge_rel), stale_bridge_with_trailing_whitespace).unwrap();

        let build_config = BuildConfig {
            tool: "cargo",
            crate_suffix: "-dart",
            build_dep: BuildDependency::None,
            post_build: vec![
                PostBuildStep::RunCommand {
                    // Never installed on any PATH -- `run_run_command` reports a missing tool as
                    // `Ok(false)` (skipped) deterministically, standing in for "frb was skipped
                    // this run" without touching real process-global state.
                    cmd: "alef-frb-codegen-intentionally-not-on-path-xyz789",
                    args: vec!["generate"],
                },
                PostBuildStep::VerifyFrbBridgeCoverage {
                    facade_path: facade_rel.clone(),
                    bridge_path: bridge_rel.clone(),
                    exclude_functions: vec![],
                },
                // Would rewrite `bridge_rel` in place if reached -- proving it is unreached is
                // the point of this test.
                PostBuildStep::PostProcessFile {
                    path: bridge_rel.clone(),
                    processor: PostProcessor::DartStripTrailingWhitespace,
                },
            ],
        };

        let result = crate::cli::pipeline::run_post_build(
            Language::Dart,
            &build_config,
            &ResolvedCrateConfig::default(),
            dir.path(),
        );

        let error = result.expect_err("a stale bridge behind a skipped frb run must fail the build");
        assert!(
            format!("{error:#}").contains("record_price"),
            "error must name the missing function: {error:#}"
        );

        let bridge_after = std::fs::read_to_string(dir.path().join(&bridge_rel)).unwrap();
        assert_eq!(
            bridge_after, stale_bridge_with_trailing_whitespace,
            "the PostProcessFile step after VerifyFrbBridgeCoverage must never run against the stale bridge"
        );
    }
}
