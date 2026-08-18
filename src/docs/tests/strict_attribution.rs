use crate::docs::{attribute_capability_capped, attribute_declared_capped, attribute_results, attribute_unavailable};
use crate::snippets::types::{
    DowngradeReason, Language, RunSummary, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel,
    ValidationResult,
};

fn downgraded(id: &str, language: Language) -> ValidationResult {
    result(id, language, SnippetStatus::Downgraded)
}

fn downgraded_with_reason(id: &str, language: Language, reason: DowngradeReason) -> ValidationResult {
    ValidationResult {
        downgrade_reason: Some(reason),
        ..downgraded(id, language)
    }
}

fn capability_capped(id: &str, language: Language) -> ValidationResult {
    ValidationResult {
        status: SnippetStatus::Pass,
        capability_capped: true,
        downgrade_reason: Some(DowngradeReason::ValidatorCapability),
        ..result(id, language, SnippetStatus::Pass)
    }
}

fn declared_capped(id: &str, language: Language) -> ValidationResult {
    ValidationResult {
        status: SnippetStatus::Pass,
        downgrade_reason: Some(DowngradeReason::Declared),
        ..result(id, language, SnippetStatus::Pass)
    }
}

fn unresolved_dependency(id: &str, language: Language) -> ValidationResult {
    ValidationResult {
        unresolved_dependency: true,
        ..result(id, language, SnippetStatus::Unavailable)
    }
}

fn toolchain_missing(id: &str, language: Language) -> ValidationResult {
    result(id, language, SnippetStatus::Unavailable)
}

fn result(id: &str, language: Language, status: SnippetStatus) -> ValidationResult {
    ValidationResult {
        snippet: Snippet {
            id: Some(id.to_string()),
            path: "docs/example.md".into(),
            language,
            title: None,
            code: String::new(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "docs/example.md".into(),
                line: 7,
                block_index: 0,
            },
        },
        status,
        level: ValidationLevel::Syntax,
        requested_level: ValidationLevel::TypeCheck,
        effective_level: ValidationLevel::Syntax,
        message: None,
        duration_ms: 0,
        capability_capped: false,
        downgrade_reason: None,
        unresolved_dependency: false,
    }
}

/// A strict failure must name the snippets behind the count. The achieved level is not recorded
/// in emitted frontmatter, so a bare total leaves a consumer with no way to find the regressions. ~keep
#[test]
fn attribution_names_language_count_and_level_transition() {
    let summary = RunSummary::from_results(vec![
        downgraded("fixture_c_smoke", Language::C),
        downgraded("fixture_py_smoke", Language::Python),
    ]);

    let detail = attribute_results(&summary, SnippetStatus::Downgraded);

    assert!(
        detail.contains("fixture_c_smoke (typecheck -> syntax)"),
        "got: {detail}"
    );
    assert!(
        detail.contains("fixture_py_smoke (typecheck -> syntax)"),
        "got: {detail}"
    );
    assert!(detail.contains("c: 1"), "per-language count missing, got: {detail}");
    assert!(
        detail.contains("python: 1"),
        "per-language count missing, got: {detail}"
    );
}

/// A run with hundreds of downgrades must stay readable: sample a few ids per language and say
/// how many were elided, rather than emitting one line per snippet.
#[test]
fn attribution_bounds_the_sample_and_reports_the_remainder() {
    let results = (0..10)
        .map(|index| downgraded(&format!("fixture_c_{index}"), Language::C))
        .collect::<Vec<_>>();
    let summary = RunSummary::from_results(results);

    let detail = attribute_results(&summary, SnippetStatus::Downgraded);

    assert!(detail.contains("c: 10"), "total must be the real count, got: {detail}");
    assert!(
        detail.contains("+7 more"),
        "elided remainder must be reported, got: {detail}"
    );
    assert_eq!(
        detail.matches("fixture_c_").count(),
        3,
        "sample must be bounded to three ids per language, got: {detail}"
    );
}

/// Attribution is per status: a downgrade listing must not absorb failures, or a mixed run
/// would report the wrong snippets against the wrong cause.
#[test]
fn attribution_filters_to_the_requested_status() {
    let summary = RunSummary::from_results(vec![
        downgraded("fixture_downgraded", Language::C),
        result("fixture_failed", Language::C, SnippetStatus::Fail),
    ]);

    let detail = attribute_results(&summary, SnippetStatus::Downgraded);

    assert!(detail.contains("fixture_downgraded"), "got: {detail}");
    assert!(!detail.contains("fixture_failed"), "got: {detail}");
}

/// No matching results means no trailing detail, so the message reads cleanly when the count
/// the caller is reporting came from a different status.
#[test]
fn attribution_is_empty_when_nothing_matches() {
    let summary = RunSummary::from_results(vec![result("fixture_passed", Language::C, SnippetStatus::Pass)]);

    assert_eq!(attribute_results(&summary, SnippetStatus::Downgraded), "");
}

/// A bare per-language count does not tell a consumer whether the fix is "stop suppressing this
/// snippet" or "the environment is broken" — those call for entirely different actions. Two
/// downgrades in the same language for different reasons must both be visible, with their own
/// counts, not collapsed into one undifferentiated total.
#[test]
fn attribution_breaks_downgrades_down_by_reason_within_a_language() {
    let summary = RunSummary::from_results(vec![
        downgraded_with_reason("fixture_suppressed", Language::C, DowngradeReason::Annotation),
        downgraded_with_reason("fixture_env_broke", Language::C, DowngradeReason::Environment),
    ]);

    let detail = attribute_results(&summary, SnippetStatus::Downgraded);

    assert!(detail.contains("c: 2"), "total must cover both reasons, got: {detail}");
    assert!(detail.contains("author suppressed via annotation: 1"), "got: {detail}");
    assert!(
        detail.contains("environment could not reach this level: 1"),
        "got: {detail}"
    );
}

/// A downgrade with no recorded reason (constructed directly, bypassing `finalize_result`'s
/// `classify_result`) must not break attribution — it just contributes no reason breakdown,
/// exactly like the pre-existing tests above that never set `downgrade_reason`. ~keep
#[test]
fn attribution_tolerates_a_downgrade_with_no_recorded_reason() {
    let summary = RunSummary::from_results(vec![downgraded("fixture_c_smoke", Language::C)]);

    let detail = attribute_results(&summary, SnippetStatus::Downgraded);

    assert!(detail.contains("c: 1"), "got: {detail}");
    assert!(
        !detail.contains('['),
        "no reason must mean no reason breakdown, got: {detail}"
    );
}

/// `capability_capped` results are `Pass`, not `Downgraded`, so they need their own attribution
/// path — `attribute_capability_capped` — for a consumer to see which snippets and languages hit
/// a validator ceiling instead of just watching the bare summary count climb.
#[test]
fn attribution_capability_capped_names_the_validator_reason() {
    let summary = RunSummary::from_results(vec![
        capability_capped("fixture_php_typecheck", Language::Php),
        capability_capped("fixture_ruby_typecheck", Language::Ruby),
    ]);

    let detail = attribute_capability_capped(&summary);

    assert!(detail.contains("fixture_php_typecheck"), "got: {detail}");
    assert!(detail.contains("php: 1"), "got: {detail}");
    assert!(detail.contains("ruby: 1"), "got: {detail}");
    assert!(detail.contains("validator cannot reach this level: 1"), "got: {detail}");
}

/// `declared_capped` results are `Pass` too, but for a distinct reason from `capability_capped`:
/// the snippet's own front-matter `level:` set the ceiling, not the validator. This is the path
/// every `alef e2e generate` fixture snippet takes (they all declare `level: typecheck`), so a
/// consumer who configured `validation_level = "run"` needs `attribute_declared_capped` to say
/// which snippets never actually reached `run`, not just watch the summary count climb silently. ~keep
#[test]
fn attribution_declared_capped_names_the_front_matter_reason() {
    let summary = RunSummary::from_results(vec![
        declared_capped("fixture_go_smoke", Language::Go),
        declared_capped("fixture_java_smoke", Language::Java),
    ]);

    let detail = attribute_declared_capped(&summary);

    assert!(detail.contains("fixture_go_smoke"), "got: {detail}");
    assert!(detail.contains("go: 1"), "got: {detail}");
    assert!(detail.contains("java: 1"), "got: {detail}");
    assert!(
        detail.contains("author declared this level via front matter: 1"),
        "got: {detail}"
    );
}

/// `attribute_declared_capped` must not absorb `capability_capped` results — they are `Pass` too,
/// but for a different reason, and collapsing them would misattribute a validator ceiling as a
/// snippet's own declared contract. ~keep
#[test]
fn attribution_declared_capped_does_not_absorb_capability_capped_results() {
    let summary = RunSummary::from_results(vec![capability_capped("fixture_ruby_typecheck", Language::Ruby)]);

    let detail = attribute_declared_capped(&summary);

    assert_eq!(detail, "", "got: {detail}");
}

/// `Unavailable` results get counts per language and cause, not sample ids: the remediation
/// ("run `alef build`" vs. "install the toolchain") applies to the whole language batch, so a
/// sample id told a consumer nothing a count didn't already.
#[test]
fn attribution_unavailable_counts_by_language_and_cause() {
    let summary = RunSummary::from_results(vec![
        unresolved_dependency("fixture_ts_import", Language::TypeScript),
        unresolved_dependency("fixture_ts_import_2", Language::TypeScript),
        toolchain_missing("fixture_zig_missing", Language::Zig),
    ]);

    let detail = attribute_unavailable(&summary);

    assert!(
        detail.contains("typescript: 2 unresolved dependency, 0 toolchain missing"),
        "got: {detail}"
    );
    assert!(
        detail.contains("zig: 0 unresolved dependency, 1 toolchain missing"),
        "got: {detail}"
    );
}

/// Regression for the bug this whole fix targets: `attribute_results` read `downgrade_reason`
/// for its `[reasons]` bracket, which `finalize_result` never populates for `Unavailable` (see
/// the `debug_assert!` guarding `classify_result`), and rendered a `(requested -> effective)`
/// arrow that implied a level downgrade `Unavailable` results never have. `attribute_unavailable`
/// must not carry either artifact forward. ~keep
#[test]
fn attribution_unavailable_has_no_empty_reason_bracket_or_misleading_arrow() {
    let summary = RunSummary::from_results(vec![unresolved_dependency("fixture_ts_import", Language::TypeScript)]);

    let detail = attribute_unavailable(&summary);

    assert!(!detail.contains('['), "no dangling reason bracket, got: {detail}");
    assert!(!detail.contains("->"), "no level-transition arrow, got: {detail}");
    assert!(!detail.contains("fixture_ts_import"), "no sample ids, got: {detail}");
}

/// No matching results means no trailing detail, matching `attribute_results`'s empty behavior.
#[test]
fn attribution_unavailable_is_empty_when_nothing_matches() {
    let summary = RunSummary::from_results(vec![result("fixture_passed", Language::C, SnippetStatus::Pass)]);

    assert_eq!(attribute_unavailable(&summary), "");
}
