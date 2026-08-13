use anyhow::Result;

pub(super) fn ensure_success(
    has_stale_bindings: bool,
    has_version_issues: bool,
    snippet_coverage_issue_count: usize,
    report_only: bool,
) -> Result<()> {
    if report_only || (!has_stale_bindings && !has_version_issues && snippet_coverage_issue_count == 0) {
        return Ok(());
    }
    anyhow::bail!("generated bindings, versions, or snippet coverage are out of date")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_only_succeeds_with_multi_crate_coverage_issues() {
        ensure_success(false, false, 2, true).expect("report-only keeps a successful exit status");
    }

    #[test]
    fn coverage_issue_fails_with_honest_diagnostic() {
        let error = ensure_success(false, false, 1, false).expect_err("coverage issue must fail verification");
        assert!(error.to_string().contains("snippet coverage"));
    }
}
