use super::*;

/// The regression this fix closes: `run_post_build`'s `Ok` arm carries
/// `skipped_missing_tools`, and before `record_post_build_outcome` existed, both call sites
/// in `build_with_environment` matched only `Err` and discarded the `Ok` arm wholesale --
/// `alef build` reported a clean success while silently falling back to stale committed
/// output for a language whose post-build tool was missing from `PATH`. This proves the
/// caller can now tell the difference: a skipped tool must land in
/// `skipped_post_build_tools`, not vanish. ~keep
#[test]
fn a_skipped_tool_is_recorded_not_discarded() {
    let mut failures = Vec::new();
    let mut skipped_post_build_tools = Vec::new();

    record_post_build_outcome(
        Language::Dart,
        Ok(PostBuildOutcome {
            skipped_missing_tools: vec!["flutter_rust_bridge_codegen".to_string()],
        }),
        &mut failures,
        &mut skipped_post_build_tools,
    );

    assert!(
        failures.is_empty(),
        "a skipped tool is non-fatal, not a build failure: {failures:?}"
    );
    assert_eq!(
        skipped_post_build_tools,
        vec!["dart: flutter_rust_bridge_codegen".to_string()]
    );
}

#[test]
fn a_clean_post_build_records_nothing() {
    let mut failures = Vec::new();
    let mut skipped_post_build_tools = Vec::new();

    record_post_build_outcome(
        Language::Dart,
        Ok(PostBuildOutcome::default()),
        &mut failures,
        &mut skipped_post_build_tools,
    );

    assert!(failures.is_empty());
    assert!(skipped_post_build_tools.is_empty());
}

/// The control: a genuine post-build failure must still be fatal, not reclassified as a
/// skip. ~keep
#[test]
fn a_genuine_post_build_error_still_fails() {
    let mut failures = Vec::new();
    let mut skipped_post_build_tools = Vec::new();

    record_post_build_outcome(
        Language::Dart,
        Err(anyhow::anyhow!("patch target missing")),
        &mut failures,
        &mut skipped_post_build_tools,
    );

    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("patch target missing"), "{failures:?}");
    assert!(skipped_post_build_tools.is_empty());
}
