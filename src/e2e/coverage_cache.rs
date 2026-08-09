use crate::e2e::snippets::SnippetCoverageLedger;
use anyhow::{Context, Result};
use std::path::Path;

pub(super) fn read_coverage_manifest(path: &Path) -> Result<SnippetCoverageLedger> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read snippet coverage manifest {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse snippet coverage manifest {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::snippets::{MissingSnippet, SnippetCoverageKey};

    #[test]
    fn round_trip_preserves_missing_ledger() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("coverage.json");
        let coverage = SnippetCoverageLedger {
            expected: vec![SnippetCoverageKey {
                fixture_id: "sample_request".into(),
                language: "swift".into(),
            }],
            missing: vec![MissingSnippet {
                key: SnippetCoverageKey {
                    fixture_id: "sample_request".into(),
                    language: "swift".into(),
                },
                reason: "sample renderer is unavailable".into(),
            }],
            ..SnippetCoverageLedger::default()
        };
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&coverage).expect("serialize coverage"),
        )
        .expect("write coverage");

        let loaded = read_coverage_manifest(&path).expect("read coverage");

        assert_eq!(loaded, coverage);
    }
}
