//! Shared helper for extracting a single GitHub Actions job's block of YAML text from a
//! workflow file. Reused by every test that treats one CI job's steps as ground truth for a
//! wiring assertion.
//!
//! This used to be three independent copies -- in `generated_output_downstream_gate.rs`,
//! `ci_poly_pin_gate.rs`, and `test_vacuity_gate.rs` -- and they had already drifted:
//! `test_vacuity_gate.rs`'s copy matched the header with `*line != header` (no `trim_end`),
//! discarded the header line from the returned block instead of including it, and appended a
//! trailing newline per line instead of a leading one. A vacuity gate and a pin gate disagreeing
//! about what a job block even is is exactly the kind of drift a vacuity gate exists to catch --
//! already realised in this repo's own test suite. Consolidated to one implementation so it
//! can't recur. ~keep

#![allow(dead_code)]

/// Extract one job's block from a workflow, from its `  <name>:` line up to (but not including)
/// the next line at the same two-space indent.
///
/// The scoping is the point. An earlier version of a caller of this function searched the whole
/// file for its needles, and they already appeared in unrelated jobs -- so it would have passed
/// with the job under test deleted outright. A wiring check that cannot fail is the same kind of
/// nothing as a lint that examines nothing. ~keep
///
/// Known imprecision, harmless today: the sibling-job boundary only fires on a 2-space-indent
/// line ending in `:`, so a 2-space-indent comment block that precedes the *next* job's header
/// (YAML attaches leading comments loosely) gets folded onto the *end* of the preceding job's
/// block -- e.g. in `.github/workflows/ci.yml`, `msrv`'s leading `~keep` comment currently
/// trails after `generated-output-gate`'s real steps. Harmless as long as no caller's needle
/// happens to appear in a neighbouring job's comments; callers should scope their own checks to
/// specific lines/constructs within the block rather than a bare `block.contains(...)`, so a
/// coincidental match here can't silently pass a check. ~keep
pub fn workflow_job_block(workflow: &str, job: &str) -> Option<String> {
    let header = format!("  {job}:");
    let mut lines = workflow.lines().skip_while(|line| line.trim_end() != header);
    let first = lines.next()?;
    let mut block = String::from(first);
    for line in lines {
        let is_sibling_job = line.starts_with("  ") && !line.starts_with("   ") && line.trim_end().ends_with(':');
        if is_sibling_job {
            break;
        }
        block.push('\n');
        block.push_str(line);
    }
    Some(block)
}

/// The block extractor has to actually stop at the next job, or every needle checked against it
/// would be satisfied by some other job's steps and the wiring check would be vacuous again.
#[test]
fn workflow_job_block_stops_at_the_next_job() {
    let workflow = concat!(
        "jobs:\n",
        "  first:\n    steps:\n      - run: marker-in-first\n",
        "  second:\n    steps:\n      - run: marker-in-second\n",
    );
    let first = workflow_job_block(workflow, "first").expect("first job block");
    assert!(first.contains("marker-in-first"), "block must contain its own steps");
    assert!(
        !first.contains("marker-in-second"),
        "block leaked into the following job, so job-scoped assertions would be meaningless"
    );
    assert!(
        workflow_job_block(workflow, "absent").is_none(),
        "a job that does not exist must not resolve to a block"
    );
}
