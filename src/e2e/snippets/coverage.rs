use super::{COVERAGE_MANIFEST_VERSION, SnippetCoverageKey, SnippetCoverageLedger};
use anyhow::{Result, bail};
use std::collections::BTreeSet;
use std::path::Path;

pub fn normalize(mut ledger: SnippetCoverageLedger) -> SnippetCoverageLedger {
    ledger.expected.sort();
    ledger.generated.sort();
    ledger.generated_paths.sort();
    ledger
        .generated_metadata
        .sort_by(|left, right| left.path.cmp(&right.path));
    ledger.missing.sort_by(|left, right| left.key.cmp(&right.key));
    ledger
        .documented_exceptions
        .sort_by(|left, right| left.key.cmp(&right.key));
    ledger
}

pub fn validate(ledger: &SnippetCoverageLedger) -> Result<()> {
    if ledger.format_version != COVERAGE_MANIFEST_VERSION {
        bail!(
            "snippet coverage manifest version {} is unsupported; expected {}",
            ledger.format_version,
            COVERAGE_MANIFEST_VERSION
        );
    }
    ensure_unique("expected", ledger.expected.iter())?;
    ensure_unique("generated", ledger.generated.iter())?;
    ensure_unique("missing", ledger.missing.iter().map(|entry| &entry.key))?;
    ensure_unique(
        "documented exceptions",
        ledger.documented_exceptions.iter().map(|entry| &entry.key),
    )?;

    let expected = key_set(ledger.expected.iter());
    let generated = key_set(ledger.generated.iter());
    let missing = key_set(ledger.missing.iter().map(|entry| &entry.key));
    let exceptions = key_set(ledger.documented_exceptions.iter().map(|entry| &entry.key));
    ensure_subset("generated", &generated, &expected)?;
    ensure_subset("missing", &missing, &expected)?;
    ensure_subset("documented exceptions", &exceptions, &expected)?;
    ensure_disjoint("generated", &generated, "missing", &missing)?;
    ensure_disjoint("generated", &generated, "documented exceptions", &exceptions)?;
    ensure_disjoint("missing", &missing, "documented exceptions", &exceptions)?;

    let classified: BTreeSet<_> = generated
        .union(&missing)
        .cloned()
        .chain(exceptions.iter().cloned())
        .collect();
    if classified != expected {
        let first = expected
            .difference(&classified)
            .next()
            .expect("unequal sets have an unclassified key");
        bail!(
            "snippet coverage cell `{}` / `{}` is not classified",
            first.fixture_id,
            first.language
        );
    }
    validate_generated_metadata(ledger, &generated)?;
    for exception in &ledger.documented_exceptions {
        if exception.reason.trim().is_empty() {
            bail!(
                "snippet coverage exception for `{}` / `{}` has an empty reason",
                exception.key.fixture_id,
                exception.key.language
            );
        }
    }
    Ok(())
}

pub fn validate_tracked_files(ledger: &SnippetCoverageLedger, output: &Path) -> Result<()> {
    for relative in &ledger.generated_paths {
        let path = super::ledger_paths::resolve_tracked_path(output, relative)?;
        if !path.is_file() {
            bail!("tracked snippet file is missing: {}", path.display());
        }
    }
    Ok(())
}

pub fn validate_current(disk: SnippetCoverageLedger, computed: SnippetCoverageLedger) -> Result<()> {
    validate(&disk)?;
    validate(&computed)?;
    if normalize(disk) != normalize(computed) {
        bail!("snippet coverage ledger is stale");
    }
    Ok(())
}

fn validate_generated_metadata(ledger: &SnippetCoverageLedger, generated: &BTreeSet<SnippetCoverageKey>) -> Result<()> {
    if ledger.generated_paths.len() != ledger.generated_metadata.len() {
        bail!("snippet coverage generated paths and metadata have different lengths");
    }
    let paths: BTreeSet<_> = ledger.generated_paths.iter().collect();
    if paths.len() != ledger.generated_paths.len() {
        bail!("snippet coverage generated paths contain duplicates");
    }
    let metadata_paths: BTreeSet<_> = ledger.generated_metadata.iter().map(|entry| &entry.path).collect();
    if paths != metadata_paths {
        bail!("snippet coverage generated paths do not match metadata paths");
    }
    let metadata_keys = key_set(ledger.generated_metadata.iter().map(|entry| &entry.key));
    if &metadata_keys != generated {
        bail!("snippet coverage generated keys do not match metadata keys");
    }
    ensure_unique(
        "generated metadata",
        ledger.generated_metadata.iter().map(|entry| &entry.key),
    )
}

fn key_set<'a>(keys: impl Iterator<Item = &'a SnippetCoverageKey>) -> BTreeSet<SnippetCoverageKey> {
    keys.cloned().collect()
}

fn ensure_unique<'a>(label: &str, keys: impl Iterator<Item = &'a SnippetCoverageKey>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key) {
            bail!(
                "snippet coverage {label} contains duplicate cell `{}` / `{}`",
                key.fixture_id,
                key.language
            );
        }
    }
    Ok(())
}

fn ensure_subset(
    label: &str,
    values: &BTreeSet<SnippetCoverageKey>,
    expected: &BTreeSet<SnippetCoverageKey>,
) -> Result<()> {
    if let Some(key) = values.difference(expected).next() {
        bail!(
            "snippet coverage {label} contains unknown cell `{}` / `{}`",
            key.fixture_id,
            key.language
        );
    }
    Ok(())
}

fn ensure_disjoint(
    left_label: &str,
    left: &BTreeSet<SnippetCoverageKey>,
    right_label: &str,
    right: &BTreeSet<SnippetCoverageKey>,
) -> Result<()> {
    if let Some(key) = left.intersection(right).next() {
        bail!(
            "snippet coverage cell `{}` / `{}` appears in both {left_label} and {right_label}",
            key.fixture_id,
            key.language
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::fixture::SideEffectClass;
    use crate::e2e::snippets::{DocumentedSnippetException, GeneratedSnippetMetadata, MissingSnippet};
    use std::path::PathBuf;

    fn key(language: &str) -> SnippetCoverageKey {
        SnippetCoverageKey {
            fixture_id: "sample_request".into(),
            language: language.into(),
        }
    }

    fn generated_ledger() -> SnippetCoverageLedger {
        SnippetCoverageLedger {
            format_version: COVERAGE_MANIFEST_VERSION,
            generated_paths: vec![PathBuf::from("python/sample-request.md")],
            generated_metadata: vec![GeneratedSnippetMetadata {
                key: key("python"),
                path: PathBuf::from("python/sample-request.md"),
                language: "python".into(),
                target: "python".into(),
                session: "python".into(),
                requires: Vec::new(),
                side_effect: SideEffectClass::Safe,
            }],
            expected: vec![key("python")],
            generated: vec![key("python")],
            missing: Vec::new(),
            documented_exceptions: Vec::new(),
        }
    }

    #[test]
    fn exact_partition_accepts_documented_exception() {
        let mut ledger = generated_ledger();
        ledger.generated_paths.clear();
        ledger.generated_metadata.clear();
        ledger.generated.clear();
        ledger.documented_exceptions.push(DocumentedSnippetException {
            key: key("python"),
            reason: "the sample backend cannot express this recipe".into(),
            reference: "docs/limitations.md".into(),
        });

        validate(&ledger).expect("documented exception completes partition");
    }

    #[test]
    fn exact_partition_rejects_overlap_and_unknown_cells() {
        let mut overlap = generated_ledger();
        overlap.missing.push(MissingSnippet {
            key: key("python"),
            reason: "renderer unavailable".into(),
        });
        assert!(
            validate(&overlap)
                .expect_err("overlap must fail")
                .to_string()
                .contains("both generated and missing")
        );

        let mut unknown = generated_ledger();
        unknown.generated.push(key("java"));
        assert!(
            validate(&unknown)
                .expect_err("unknown cell must fail")
                .to_string()
                .contains("unknown cell")
        );
    }

    #[test]
    fn metadata_and_tracked_files_must_agree() {
        let mut ledger = generated_ledger();
        ledger.generated_metadata[0].path = PathBuf::from("python/other.md");
        assert!(
            validate(&ledger)
                .expect_err("metadata mismatch must fail")
                .to_string()
                .contains("metadata paths")
        );

        let ledger = generated_ledger();
        let directory = tempfile::tempdir().expect("temporary directory");
        assert!(
            validate_tracked_files(&ledger, directory.path())
                .expect_err("missing tracked file must fail")
                .to_string()
                .contains("tracked snippet file is missing")
        );
    }

    #[test]
    fn semantic_comparison_detects_added_fixture_language_cell() {
        let disk = generated_ledger();
        let mut computed = generated_ledger();
        computed.expected.push(key("java"));
        computed.missing.push(MissingSnippet {
            key: key("java"),
            reason: "renderer unavailable".into(),
        });

        assert!(
            validate_current(disk, computed)
                .expect_err("new semantic cell must make disk ledger stale")
                .to_string()
                .contains("stale")
        );
    }

    #[test]
    fn corrupt_version_duplicate_and_empty_exception_are_rejected() {
        let mut version = generated_ledger();
        version.format_version = 0;
        assert!(
            validate(&version)
                .expect_err("version must fail")
                .to_string()
                .contains("version 0")
        );

        let mut duplicate = generated_ledger();
        duplicate.expected.push(key("python"));
        assert!(
            validate(&duplicate)
                .expect_err("duplicate must fail")
                .to_string()
                .contains("duplicate")
        );

        let mut exception = generated_ledger();
        exception.generated.clear();
        exception.generated_paths.clear();
        exception.generated_metadata.clear();
        exception.documented_exceptions.push(DocumentedSnippetException {
            key: key("python"),
            reason: " ".into(),
            reference: "docs/limitations.md".into(),
        });
        assert!(
            validate(&exception)
                .expect_err("empty reason must fail")
                .to_string()
                .contains("empty reason")
        );
    }
}
