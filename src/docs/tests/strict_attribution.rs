use crate::docs::attribute_results;
use crate::snippets::types::{
    Language, RunSummary, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel, ValidationResult,
};

fn downgraded(id: &str, language: Language) -> ValidationResult {
    result(id, language, SnippetStatus::Downgraded)
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
    }
}

/// A strict failure must name the snippets behind the count. The achieved level is not recorded
/// in emitted frontmatter, so a bare total leaves a consumer with no way to find the regressions.
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
