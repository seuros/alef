use super::VerifyCoverage;
use crate::bin_cli::verify_scan::ScanCoverage;
use std::collections::HashSet;
use std::path::PathBuf;

fn paths(directory: &std::path::Path, names: &[&str]) -> HashSet<PathBuf> {
    names.iter().map(|name| directory.join(name)).collect()
}

/// The three managed buckets must partition the managed surface, and each must be measured
/// from what is actually true of the path -- marked, merely present, or absent.
///
/// The defect this closes is a report that counted only what it found: a run where 1 of 3
/// managed paths was content-verified read exactly like a run where all 3 were, because
/// nothing printed the denominator. ~keep
#[test]
fn managed_paths_split_into_verified_present_and_absent() {
    let directory = tempfile::tempdir().expect("temporary project");
    std::fs::write(directory.path().join("stamped.toml"), "x\n").expect("seed");
    std::fs::write(directory.path().join("unmarked.json"), "{}\n").expect("seed");

    let managed = paths(directory.path(), &["stamped.toml", "unmarked.json", "never_written.rs"]);
    let marked = paths(directory.path(), &["stamped.toml"]);

    let coverage = VerifyCoverage::measure(&managed, &marked, ScanCoverage::default());
    assert_eq!(coverage.managed_total, 3);
    assert_eq!(coverage.managed_content_verified, 1);
    assert_eq!(coverage.managed_present_only, 1);
    assert_eq!(coverage.managed_absent, 1);
    assert_eq!(
        coverage.managed_content_verified + coverage.managed_present_only + coverage.managed_absent,
        coverage.managed_total,
        "the three buckets must partition the managed surface, or the report understates its own gap"
    );
}

/// A marked file the managed surface does not claim is counted apart from the surface, never
/// folded into it -- otherwise `managed_content_verified` could exceed `managed_total` and the
/// partition assertion above would silently become unsatisfiable. ~keep
#[test]
fn marked_files_outside_the_surface_are_counted_separately() {
    let directory = tempfile::tempdir().expect("temporary project");
    let managed = paths(directory.path(), &["a.rs"]);
    let marked = paths(directory.path(), &["a.rs", "legacy_visitor.py"]);

    let coverage = VerifyCoverage::measure(&managed, &marked, ScanCoverage::default());
    assert_eq!(coverage.marked_outside_surface, 1);
    assert_eq!(coverage.managed_total, 1);
}

/// The report must NAME the narrowness, not just print numbers. A reader who sees only counts
/// still has no reason to doubt "verify passed" means "the tree is fresh".
///
/// Each assertion below names one claim the report has to make; a rewording that drops any of
/// them fails here rather than quietly shrinking what the report admits to. ~keep
#[test]
fn report_states_that_presence_only_paths_are_not_content_checked() {
    let coverage = VerifyCoverage {
        managed_total: 10,
        managed_content_verified: 4,
        managed_present_only: 5,
        managed_absent: 1,
        marked_outside_surface: 2,
        files_opened: 40,
        files_unexamined: 900,
    };
    let report = coverage.report_lines().join("\n");

    assert!(report.contains("content-verified"), "{report}");
    assert!(
        report.contains("present-but-wrong file passes"),
        "the report must say what a presence-only check fails to catch: {report}"
    );
    assert!(
        report.contains("never examined"),
        "the report must account for files the walk never opened: {report}"
    );
    for number in ["10", "4", "5", "40", "900", "2"] {
        assert!(
            report.contains(number),
            "every measured count must reach the report, missing {number}: {report}"
        );
    }
}

/// The orphan line is conditional; the four coverage claims are not. A run with a clean
/// surface must still state its scope. ~keep
#[test]
fn report_is_emitted_even_when_nothing_is_outside_the_surface() {
    let coverage = VerifyCoverage {
        managed_total: 3,
        managed_content_verified: 3,
        ..VerifyCoverage::default()
    };
    let lines = coverage.report_lines();
    assert_eq!(lines.len(), 6, "no orphan line when nothing is orphaned: {lines:?}");
    assert!(lines[0].contains("Verify coverage"));
}
