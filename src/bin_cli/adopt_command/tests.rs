//! What one `alef adopt` invocation costs: how many times it renders the managed surface,
//! and how much it prints.

use std::cell::Cell;
use std::path::PathBuf;

use super::report::{AdoptSummary, CONVERGED_ONLY_DIFF_BODY_LIMIT, DiffBudget, ReportChunk, render};
use super::*;
use crate::core::backend::GeneratedFile;

fn generated(relative: &str) -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(relative),
        content: format!("# {relative}\n"),
        generated_header: true,
    }
}

fn targets(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("packages/p{index}/manifest.toml"))
        .collect()
}

#[test]
fn should_render_the_surface_once_per_crate_however_many_targets_are_requested() {
    let base = PathBuf::from("/nonexistent-base");
    let renders = Cell::new(0_usize);
    let crates = [(), (), ()];

    let managed = managed_surface(&crates[..], &targets(48), &base, |_| {
        renders.set(renders.get() + 1);
        Ok((vec![generated("packages/p0/manifest.toml")], Vec::new()))
    })
    .expect("surface");

    assert_eq!(
        renders.get(),
        3,
        "48 targets over 3 crates must render 3 surfaces, not 144"
    );
    assert_eq!(managed.len(), 3, "each crate contributes its own outputs");
}

#[test]
fn should_render_no_surface_when_no_crate_is_selected() {
    let base = PathBuf::from("/nonexistent-base");
    let renders = Cell::new(0_usize);
    let crates: [(); 0] = [];

    let managed = managed_surface(&crates[..], &targets(4), &base, |_: &()| {
        renders.set(renders.get() + 1);
        Ok((Vec::new(), Vec::new()))
    })
    .expect("surface");

    assert_eq!(renders.get(), 0, "no crate means no render");
    assert!(managed.is_empty());
}

#[test]
fn should_bail_when_a_tolerated_stage_failure_covers_a_requested_target() {
    let base = PathBuf::from("/nonexistent-base");
    let crates = [()];

    let error = managed_surface(&crates[..], &targets(1), &base, |_| {
        Ok((
            vec![generated("packages/p0/manifest.toml")],
            vec![StageFailure {
                stage: "e2e",
                message: "generator refused".to_owned(),
                paths: vec![PathBuf::from("packages/p0/manifest.toml")],
            }],
        ))
    })
    .expect_err("a stage failure covering the target must abort");

    assert!(
        format!("{error:#}").contains("cannot answer for it"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn should_tolerate_a_stage_failure_that_covers_no_requested_target() {
    let base = PathBuf::from("/nonexistent-base");
    let crates = [()];

    let managed = managed_surface(&crates[..], &targets(1), &base, |_| {
        Ok((
            vec![generated("packages/p0/manifest.toml")],
            vec![StageFailure {
                stage: "e2e",
                message: "generator refused".to_owned(),
                paths: vec![PathBuf::from("packages/elsewhere/other.toml")],
            }],
        ))
    })
    .expect("a stage failure unrelated to every target must be tolerated");

    assert_eq!(managed.len(), 1);
}

fn summary_with_drifted(count: usize) -> AdoptSummary {
    let mut summary = AdoptSummary::default();
    for index in 0..count {
        summary.diffs.insert(
            PathBuf::from(format!("packages/p{index:04}/manifest.toml")),
            format!("--- packages/p{index:04}/manifest.toml (on disk)\n-old\n+new\n"),
        );
    }
    summary
}

fn lines(chunks: &[ReportChunk]) -> Vec<String> {
    chunks
        .iter()
        .filter_map(|chunk| match chunk {
            ReportChunk::Line(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn fragments(chunks: &[ReportChunk]) -> usize {
    chunks
        .iter()
        .filter(|chunk| matches!(chunk, ReportChunk::Fragment(_)))
        .count()
}

#[test]
fn should_print_every_drifted_diff_body_when_drifted_files_are_still_adoptable() {
    let total = CONVERGED_ONLY_DIFF_BODY_LIMIT + 17;
    let summary = summary_with_drifted(total);

    let chunks = render(&summary, false);

    assert_eq!(
        fragments(&chunks),
        total,
        "without --converged-only every drifted diff is the consent document for a write this \
         command may perform, so all {total} bodies must be printed"
    );
    assert!(
        !lines(&chunks).iter().any(|line| line.contains("WITHOUT their bodies")),
        "nothing was withheld, so no elision notice may appear"
    );
}

#[test]
fn should_report_the_exact_withheld_count_when_converged_only_bounds_the_diff_bodies() {
    let total = CONVERGED_ONLY_DIFF_BODY_LIMIT + 17;
    let summary = summary_with_drifted(total);

    let chunks = render(&summary, true);
    let rendered = lines(&chunks);

    assert_eq!(
        fragments(&chunks),
        CONVERGED_ONLY_DIFF_BODY_LIMIT,
        "exactly the budgeted number of diff bodies may be printed"
    );
    let notice = rendered
        .iter()
        .find(|line| line.contains("WITHOUT their bodies"))
        .expect("a bounded report must state that it withheld bodies");
    assert!(
        notice.starts_with(&format!("17 of {total} drifted diff(s)")),
        "the notice must name the true withheld count and the true total, got: {notice}"
    );
    let named: Vec<&String> = rendered
        .iter()
        .filter(|line| line.starts_with("  packages/p") && line.ends_with("manifest.toml"))
        .collect();
    assert_eq!(
        named.len(),
        17,
        "every withheld path must still be named, so a reader can fetch each diff individually"
    );
}

#[test]
fn should_withhold_nothing_when_the_drifted_count_is_within_the_budget() {
    let summary = summary_with_drifted(CONVERGED_ONLY_DIFF_BODY_LIMIT);

    let chunks = render(&summary, true);

    assert_eq!(fragments(&chunks), CONVERGED_ONLY_DIFF_BODY_LIMIT);
    assert!(
        !lines(&chunks).iter().any(|line| line.contains("WITHOUT their bodies")),
        "a run exactly at the ceiling withholds nothing and must say nothing"
    );
}

#[test]
fn should_budget_bodies_only_when_converged_only_is_set() {
    assert_eq!(
        DiffBudget::decide(9_999, false),
        DiffBudget {
            printed: 9_999,
            elided: 0
        }
    );
    assert_eq!(
        DiffBudget::decide(9_999, true),
        DiffBudget {
            printed: CONVERGED_ONLY_DIFF_BODY_LIMIT,
            elided: 9_999 - CONVERGED_ONLY_DIFF_BODY_LIMIT,
        }
    );
    assert_eq!(DiffBudget::decide(0, true), DiffBudget { printed: 0, elided: 0 });
}

#[test]
fn should_name_every_create_once_seed_once_however_many_targets_refused_it() {
    let mut summary = AdoptSummary::default();
    summary
        .skipped_create_once
        .insert(PathBuf::from("packages/a/suite_test.zig"));
    summary
        .skipped_create_once
        .insert(PathBuf::from("packages/b/suite_test.zig"));
    summary.converged.insert(PathBuf::from("packages/a/manifest.toml"));

    let rendered = lines(&render(&summary, true));

    let named: Vec<&String> = rendered
        .iter()
        .filter(|line| line.trim().ends_with("suite_test.zig"))
        .collect();
    assert_eq!(named.len(), 2, "each refused seed is named exactly once");
    assert_eq!(
        rendered
            .iter()
            .filter(|line| line.starts_with("NOT ADOPTED -- create-once seeds."))
            .count(),
        1,
        "the paragraph explaining the refusal is printed once for the invocation"
    );
}

#[test]
fn should_print_nothing_when_no_target_reached_a_verdict() {
    let summary = AdoptSummary {
        preview: true,
        ..AdoptSummary::default()
    };

    let chunks = render(&summary, false);

    assert_eq!(
        chunks.len(),
        0,
        "with every target failed there is no result, and the `--write` hint would name the \
         wrong problem"
    );
}
