use super::*;

fn dirs(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

/// The consumer incident: all three inputs omitted. Every one has to be named, together with
/// the check class it disabled. ~keep
#[test]
fn every_unset_gap_input_is_named_with_the_check_it_disables() {
    let unset = unset_gap_inputs(&[], &[], &[]);

    let keys: Vec<&str> = unset.iter().map(|input| input.key).collect();
    assert_eq!(keys, vec!["docs_dirs", "required_languages", "include_base_paths"]);
    for input in &unset {
        assert!(
            !input.consequence.is_empty(),
            "{} must state what went unchecked",
            input.key
        );
    }
}

/// Control: a fully configured invocation must report nothing unset, or the warning above is
/// unconditional noise rather than a signal. ~keep
#[test]
fn a_fully_configured_gap_check_reports_no_unset_input() {
    let unset = unset_gap_inputs(&dirs(&["docs"]), &[Language::Python], &dirs(&["."]));

    assert!(unset.is_empty(), "nothing may be reported unset; got {unset:?}");
    assert!(unset_input_lines(&unset, false).is_empty());
    assert!(!has_vacuous_input(&unset));
}

/// Only the two inputs whose emptiness manufactures a false clean may gate a strict run.
/// An unset base-path list over-reports (unresolved targets become reported gaps), so failing
/// on it would fail every project with no `pymdownx.snippets` `base_path`. ~keep
#[test]
fn only_the_vacuity_causing_inputs_are_strict_fatal() {
    assert!(has_vacuous_input(&unset_gap_inputs(
        &[],
        &[Language::Python],
        &dirs(&["."])
    )));
    assert!(has_vacuous_input(&unset_gap_inputs(
        &dirs(&["docs"]),
        &[],
        &dirs(&["."])
    )));
    assert!(
        !has_vacuous_input(&unset_gap_inputs(&dirs(&["docs"]), &[Language::Python], &[])),
        "an unset include_base_paths over-reports and must not fail a strict run on its own"
    );
}

#[test]
fn the_unset_warning_points_at_strict_mode_only_when_the_verdict_is_vacuous() {
    let vacuous = unset_input_lines(&unset_gap_inputs(&[], &[], &[]), false).join("\n");
    assert!(vacuous.contains("--strict"), "got: {vacuous}");
    assert!(vacuous.contains("proves less than it appears to"), "got: {vacuous}");

    let only_base_paths = unset_input_lines(&unset_gap_inputs(&dirs(&["docs"]), &[Language::Python], &[]), false);
    assert_eq!(
        only_base_paths.len(),
        2,
        "a non-vacuous omission gets a heading and its line, and no strict-mode advice: {only_base_paths:?}"
    );
}

/// Under `--strict` the run fails, so advising the reader to pass `--strict` would be absurd.
#[test]
fn a_strict_run_does_not_advise_passing_strict() {
    let lines = unset_input_lines(&unset_gap_inputs(&[], &[], &[]), true).join("\n");

    assert!(lines.contains("docs_dirs unset"), "got: {lines}");
    assert!(!lines.contains("pass --strict"), "got: {lines}");
}

/// The exact incident shape: snippets discovered, all of them vouched for by a coverage
/// ledger, zero documentation pages opened, zero required languages. The report has to make
/// the emptiness visible rather than let "No gaps found." stand alone. ~keep
#[test]
fn coverage_names_the_zeroes_that_make_a_clean_verdict_vacuous() {
    let coverage = GapCoverage {
        snippet_roots: 1,
        snippets_discovered: 148,
        docs_roots: 0,
        docs_pages_scanned: 0,
        include_references: 0,
        configured_references: 148,
        required_languages: 0,
        language_groups: 0,
        include_base_paths: 0,
    };
    let report = coverage.report_lines().join("\n");

    assert!(
        report.contains("NO documentation page entered this result"),
        "got: {report}"
    );
    assert!(report.contains("compared nothing"), "got: {report}");
    assert!(
        report.contains("148"),
        "the discovered snippet count must appear: {report}"
    );
}

/// Control: a run that really did compare something must not carry the emptiness warnings, or
/// the assertions above are true of every report and prove nothing. ~keep
#[test]
fn coverage_of_a_configured_run_carries_no_emptiness_warning() {
    let coverage = GapCoverage {
        snippet_roots: 1,
        snippets_discovered: 148,
        docs_roots: 1,
        docs_pages_scanned: 62,
        include_references: 130,
        configured_references: 18,
        required_languages: 3,
        language_groups: 40,
        include_base_paths: 1,
    };
    let report = coverage.report_lines().join("\n");

    assert!(!report.contains("NO documentation page"), "got: {report}");
    assert!(!report.contains("compared nothing"), "got: {report}");
    assert!(report.contains("62 page(s) opened"), "got: {report}");
    assert!(report.contains("3 required language(s) across 40"), "got: {report}");
}

/// Required languages set but no group found is still a vacuous parity check: the group key
/// derives from a `{language}` path component, so a snippet tree laid out any other way
/// yields no group and the check silently produces nothing. ~keep
#[test]
fn required_languages_without_a_single_group_still_reads_as_uncompared() {
    let coverage = GapCoverage {
        required_languages: 3,
        language_groups: 0,
        ..GapCoverage::default()
    };

    assert!(coverage.report_lines().join("\n").contains("compared nothing"));
}
