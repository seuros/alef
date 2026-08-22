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

use anyhow::Context as _;
use std::path::Path;

/// Read `facade_file` and `bridge_file` and return an error naming every facade free function
/// (outside `exclude_functions`) that has no matching function in the bridge.
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

    let missing = crate::backends::dart::missing_bridge_functions(&facade_source, &bridge_source, exclude_functions);
    if missing.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "flutter_rust_bridge bridge {} is missing {} function(s) declared in facade {}: {}. \
         flutter_rust_bridge_codegen did not (re)generate this bridge against the current facade \
         -- install/enable `flutter_rust_bridge_codegen` (or update the committed bridge source) \
         and rerun, since alef's post-build patches must not be applied to a stale bridge.",
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

    /// End-to-end proof that `run_post_build` itself refuses to patch a stale bridge: when the
    /// `RunCommand` step for `flutter_rust_bridge_codegen` is skipped (here via
    /// `ALEF_SKIP_COMMANDS`, standing in for "tool not on PATH") and the facade has since gained
    /// a function the committed bridge lacks, the whole post-build sequence must error out
    /// before the `PostProcessFile` step that follows ever touches the bridge file.
    #[test]
    fn run_post_build_aborts_before_patching_a_stale_bridge_when_frb_is_skipped() {
        use crate::core::backend::{BuildConfig, BuildDependency, PostBuildStep, PostProcessor};
        use crate::core::config::{Language, ResolvedCrateConfig};

        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        // SAFETY: serialized by ENV_LOCK; no other test in this process sets this var concurrently.
        unsafe {
            std::env::set_var("ALEF_SKIP_COMMANDS", "flutter_rust_bridge_codegen");
        }

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
                    cmd: "flutter_rust_bridge_codegen",
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

        match previous {
            Some(value) => unsafe { std::env::set_var("ALEF_SKIP_COMMANDS", value) },
            None => unsafe { std::env::remove_var("ALEF_SKIP_COMMANDS") },
        }

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
