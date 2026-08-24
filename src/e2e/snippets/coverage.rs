use super::{COVERAGE_MANIFEST_VERSION, SnippetCoverageKey, SnippetCoverageLedger};
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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

/// Resolve `[crates.e2e.snippets].curated_snippets` glob patterns against the files that
/// actually exist under `output`, returning every relative path claimed as curated.
///
/// Anti-vacuity by construction: a pattern that matches no file is refused with an error
/// naming the pattern rather than silently contributing nothing to `curated_paths`. Without
/// this, a glob typo (a misspelled directory, a pattern anchored the wrong way) would parse
/// cleanly, mark nothing as curated, and leave every one of the files it was meant to cover
/// still reported as an unaccounted gap -- the exact defect class this declaration exists to
/// close. A pattern matching a path this run itself generated is refused for the same
/// reason in the other direction: a curated declaration must never silently annex alef's own
/// output.
pub fn resolve_curated_snippet_paths(
    output: &Path,
    patterns: &[String],
    generated_paths: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    if patterns.is_empty() {
        return Ok(Vec::new());
    }
    let generated: BTreeSet<&Path> = generated_paths.iter().map(PathBuf::as_path).collect();
    let existing = existing_relative_files(output)?;
    let mut curated = BTreeSet::new();
    for pattern in patterns {
        let compiled =
            glob::Pattern::new(pattern).with_context(|| format!("invalid curated snippet glob `{pattern}`"))?;
        let mut matched_any = false;
        for relative in &existing {
            if !compiled.matches_path(relative) {
                continue;
            }
            if generated.contains(relative.as_path()) {
                bail!(
                    "curated snippet glob `{pattern}` matches `{}`, which alef itself generates this run; \
                     a curated declaration must never claim a path alef writes",
                    relative.display()
                );
            }
            matched_any = true;
            curated.insert(relative.clone());
        }
        if !matched_any {
            bail!(
                "curated snippet glob `{pattern}` matches no file under `{}`; a curated declaration \
                 matching zero files is refused rather than silently accepted, since that would leave \
                 every file it was meant to cover still reported as an unaccounted gap -- fix the \
                 pattern or remove it",
                output.display()
            );
        }
    }
    Ok(curated.into_iter().collect())
}

fn existing_relative_files(output: &Path) -> Result<Vec<PathBuf>> {
    if !output.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(output).follow_links(true) {
        let entry = entry.with_context(|| format!("failed to walk snippet output {}", output.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(output)
            .with_context(|| format!("failed to relativize {}", entry.path().display()))?;
        files.push(relative.to_path_buf());
    }
    Ok(files)
}

/// A one-line coverage summary distinguishing curated files from alef-generated ones, so a
/// report can state "N curated, M generated" instead of leaving "all snippets are generated"
/// an unverifiable claim. `curated` and `generated` are file counts -- see
/// [`resolve_curated_snippet_paths`] and [`SnippetCoverageLedger::generated_paths`].
pub fn summary(curated: usize, generated: usize) -> String {
    format!("{curated} curated, {generated} generated")
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

/// Confirm every path the ledger claims it generated actually exists on disk.
///
/// This is deliberately independent of whether `generated`/`generated_paths` were computed
/// correctly upstream (see `function_excluded_for_language` for the case that motivated this
/// check): a ledger's `missing` field only ever explains a cell the *computation* refused to
/// classify as generated. It says nothing about a cell the computation claimed to generate but
/// that never actually reached disk -- that is a different failure mode, caught here instead. ~keep
pub fn validate_tracked_files(ledger: &SnippetCoverageLedger, output: &Path) -> Result<()> {
    let mut absent = Vec::new();
    for relative in &ledger.generated_paths {
        let path = super::ledger_paths::resolve_tracked_path(output, relative)?;
        if !path.is_file() {
            absent.push(path);
        }
    }
    if absent.is_empty() {
        return Ok(());
    }
    let mut detail = String::new();
    for path in &absent {
        detail.push_str("\n  ");
        detail.push_str(&path.display().to_string());
    }
    bail!(
        "snippet coverage ledger records {} file(s) as generated in `generated_paths`, but they are \
         absent from disk -- this is not a coverage gap the ledger's own `missing` field explains, \
         since it is a discrepancy between what the ledger claims it wrote and what is actually \
         there:{detail}",
        absent.len()
    );
}

pub fn validate_current(disk: SnippetCoverageLedger, computed: SnippetCoverageLedger) -> Result<()> {
    validate(&disk)?;
    validate(&computed)?;
    if normalize(disk) != normalize(computed) {
        bail!("snippet coverage ledger is stale");
    }
    Ok(())
}

/// Compute the previously alef-generated snippet paths that this run no
/// longer produces, and which must therefore be deleted from disk.
///
/// Ownership of a path is established *only* by the previous run's own
/// `generated_metadata` — the sole place alef records "I personally wrote
/// this exact path for this key." A candidate path is never reconstructed by
/// guessing from the key; it is copied verbatim from that ledger entry. A
/// hand-authored file is never selected here because alef never generated
/// it, so it was never recorded in `generated_metadata` in the first place.
///
/// The predicate is a path-set difference rather than a scan of
/// `current.missing`, because `missing` is not the state a durable orphan
/// ends up in. `ensure_snippet_coverage_complete` hard-fails on any
/// non-empty `missing`, so a key only *rests* somewhere a later successful
/// run can observe it once it has become a documented coverage exception —
/// or once its fixture is deleted outright, in which case the key is never
/// evaluated and so appears in neither `missing` nor `expected`. A
/// difference against `generated_paths` covers all three transitions at
/// once.
///
/// The language gate is what keeps a `--lang`-filtered (or entirely
/// skipped/cached) run from mass-deleting another language's still-valid
/// output: only languages this run actually evaluated — i.e. that appear in
/// `current.expected`, populated per generator in
/// `generate_snippet_report_with_extensions` — are eligible, so a run that
/// generated nothing deletes nothing.
pub fn orphaned_paths(previous: &SnippetCoverageLedger, current: &SnippetCoverageLedger) -> Vec<PathBuf> {
    let evaluated_languages: BTreeSet<&str> = current.expected.iter().map(|key| key.language.as_str()).collect();
    let still_generated: BTreeSet<&PathBuf> = current.generated_paths.iter().collect();
    previous
        .generated_metadata
        .iter()
        .filter(|entry| evaluated_languages.contains(entry.key.language.as_str()))
        .filter(|entry| !still_generated.contains(&entry.path))
        .map(|entry| entry.path.clone())
        .collect()
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
mod curated_snippet_tests {
    use super::resolve_curated_snippet_paths;
    use std::path::PathBuf;

    fn write(directory: &std::path::Path, relative: &str, content: &str) {
        let path = directory.join(relative);
        std::fs::create_dir_all(path.parent().expect("relative path has a parent")).expect("create parent directory");
        std::fs::write(path, content).expect("write curated fixture file");
    }

    #[test]
    fn a_matching_glob_is_recorded_as_curated() {
        let directory = tempfile::tempdir().expect("temp dir");
        write(directory.path(), "docker/quick-start.md", "curated by hand");

        let curated = resolve_curated_snippet_paths(directory.path(), &["docker/*.md".to_string()], &[])
            .expect("a matching pattern resolves");

        assert_eq!(curated, vec![PathBuf::from("docker/quick-start.md")]);
    }

    /// The anti-vacuity requirement pinned literally: a glob that matches zero files must
    /// fail the run rather than silently contributing nothing. A typo'd directory name here
    /// (`dcoker` for `docker`) is exactly the shape of mistake that would otherwise recreate
    /// the "coverage reports curated files as missing" gap this declaration exists to close.
    #[test]
    fn a_glob_matching_zero_files_is_refused_not_silently_accepted() {
        let directory = tempfile::tempdir().expect("temp dir");
        write(directory.path(), "docker/quick-start.md", "curated by hand");

        let error = resolve_curated_snippet_paths(directory.path(), &["dcoker/*.md".to_string()], &[])
            .expect_err("a glob matching nothing must be refused");

        assert!(error.to_string().contains("dcoker/*.md"), "{error}");
        assert!(error.to_string().contains("matches no file"), "{error}");
    }

    /// A pattern that matches only this run's own generated output must never silently
    /// annex it -- that would let a curated declaration mask a real coverage gap by
    /// reclassifying alef's own file as "not alef's concern".
    #[test]
    fn a_glob_matching_a_generated_path_is_refused() {
        let directory = tempfile::tempdir().expect("temp dir");
        write(directory.path(), "python/quick-start.md", "alef wrote this");

        let error = resolve_curated_snippet_paths(
            directory.path(),
            &["python/*.md".to_string()],
            &[PathBuf::from("python/quick-start.md")],
        )
        .expect_err("a glob claiming generated output must be refused");

        assert!(error.to_string().contains("alef itself generates"), "{error}");
    }

    #[test]
    fn no_configured_globs_yields_no_curated_paths_without_touching_disk() {
        // A directory that does not exist must not error when there are no patterns to
        // resolve -- an unconfigured project pays no cost for this feature.
        let curated = resolve_curated_snippet_paths(std::path::Path::new("/does/not/exist"), &[], &[])
            .expect("no patterns never touches the filesystem");

        assert!(curated.is_empty());
    }

    /// A glob declared for a project that has never generated anything yet (no `output`
    /// directory on disk at all) must fail exactly like any other zero-match glob -- the
    /// curated files it claims must already exist, since curated means hand-authored, not
    /// "will exist eventually".
    #[test]
    fn a_glob_over_a_missing_output_directory_is_refused() {
        let directory = tempfile::tempdir().expect("temp dir");
        let missing_output = directory.path().join("never-created");

        let error = resolve_curated_snippet_paths(&missing_output, &["**/*.md".to_string()], &[])
            .expect_err("a glob over a directory that was never generated must be refused");

        assert!(error.to_string().contains("matches no file"), "{error}");
    }

    #[test]
    fn summary_reports_curated_and_generated_counts() {
        assert_eq!(super::summary(3, 431), "3 curated, 431 generated");
        assert_eq!(super::summary(0, 0), "0 curated, 0 generated");
    }
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
                .contains("absent from disk")
        );
    }

    /// Pins that `validate_tracked_files` reports every absent tracked file, not just the
    /// first one -- a ledger claiming ten generated paths that do not exist must not read
    /// exactly like a ledger claiming one. A count-of-one message here would still pass a
    /// `contains("absent from disk")` check but would fail this exact-count assertion, which
    /// is the point: it pins the multi-file report the single-path `bail!` this replaced could
    /// never produce.
    #[test]
    fn validate_tracked_files_reports_every_absent_path_and_its_count() {
        // `validate_tracked_files` only reads `generated_paths`; the rest of the ledger's
        // bookkeeping (`expected`/`generated`/`missing`) is irrelevant to this check and is
        // left at whatever `generated_ledger()` provides.
        let mut ledger = generated_ledger();
        ledger.generated_paths.push(PathBuf::from("python/second.md"));

        let directory = tempfile::tempdir().expect("temporary directory");
        let error = validate_tracked_files(&ledger, directory.path())
            .expect_err("two claimed-generated files that do not exist must fail")
            .to_string();

        assert!(
            error.contains("2 file(s)"),
            "message must name the exact count of absent files, got: {error}"
        );
        assert!(
            error.contains("python/sample-request.md") && error.contains("python/second.md"),
            "message must name every absent path, not just the first: {error}"
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
    fn orphaned_paths_selects_a_key_that_moved_from_generated_to_missing() {
        let mut previous = generated_ledger();
        previous.generated_metadata.push(GeneratedSnippetMetadata {
            key: key("swift"),
            path: PathBuf::from("swift/sample-request.md"),
            language: "swift".into(),
            target: "swift".into(),
            session: "swift".into(),
            requires: Vec::new(),
            side_effect: SideEffectClass::Safe,
        });

        let mut current = generated_ledger();
        current.generated.clear();
        current.generated_paths.clear();
        current.generated_metadata.clear();
        current.missing.push(MissingSnippet {
            key: key("python"),
            reason: "python fixture requires an extension-owned documentation recipe".into(),
        });

        let orphans = orphaned_paths(&previous, &current);

        assert_eq!(orphans, vec![PathBuf::from("python/sample-request.md")]);
    }

    /// The durable orphan state. `ensure_snippet_coverage_complete` refuses
    /// to finish a run with a non-empty `missing`, so the way a key actually
    /// comes to rest is as a documented coverage exception — at which point
    /// it is in neither `generated` nor `missing`, and only a path-set
    /// difference can still find its stale file.
    #[test]
    fn orphaned_paths_selects_a_path_that_became_a_documented_exception() {
        let previous = generated_ledger();
        let mut current = generated_ledger();
        current.generated.clear();
        current.generated_paths.clear();
        current.generated_metadata.clear();
        current.documented_exceptions.push(DocumentedSnippetException {
            key: key("python"),
            reason: "fixture requires an extension-owned documentation recipe".into(),
            reference: "docs/extensions.md".into(),
        });

        let orphans = orphaned_paths(&previous, &current);

        assert_eq!(orphans, vec![PathBuf::from("python/sample-request.md")]);
    }

    /// A fixture deleted outright is never iterated, so its key reaches
    /// neither `expected` nor `missing`. Its file is still alef-owned and
    /// must go.
    #[test]
    fn orphaned_paths_selects_a_path_whose_fixture_was_deleted_entirely() {
        let previous = generated_ledger();
        let surviving = SnippetCoverageKey {
            fixture_id: "other_request".into(),
            language: "python".into(),
        };
        let current = SnippetCoverageLedger {
            format_version: COVERAGE_MANIFEST_VERSION,
            generated_paths: vec![PathBuf::from("python/other-request.md")],
            generated_metadata: vec![GeneratedSnippetMetadata {
                key: surviving.clone(),
                path: PathBuf::from("python/other-request.md"),
                language: "python".into(),
                target: "python".into(),
                session: "python".into(),
                requires: Vec::new(),
                side_effect: SideEffectClass::Safe,
            }],
            expected: vec![surviving.clone()],
            generated: vec![surviving],
            missing: Vec::new(),
            documented_exceptions: Vec::new(),
        };

        let orphans = orphaned_paths(&previous, &current);

        assert_eq!(orphans, vec![PathBuf::from("python/sample-request.md")]);
    }

    #[test]
    fn orphaned_paths_never_selects_a_path_alef_never_generated() {
        let previous = generated_ledger();
        let mut current = generated_ledger();
        // A key never present in `previous.generated_metadata` (e.g. a
        // hand-authored file that happens to collide with this fixture id)
        // must never be treated as alef-owned, even when it is missing now.
        current.missing.push(MissingSnippet {
            key: key("java"),
            reason: "java fixture requires an extension-owned documentation recipe".into(),
        });

        let orphans = orphaned_paths(&previous, &current);

        assert!(orphans.is_empty(), "expected no orphans, got: {orphans:?}");
    }

    #[test]
    fn orphaned_paths_ignores_a_key_still_generated_this_run() {
        let previous = generated_ledger();
        // `python` stays generated in `current` (not missing), so its file
        // must be left alone even though it is alef-owned.
        let current = generated_ledger();

        let orphans = orphaned_paths(&previous, &current);

        assert!(orphans.is_empty(), "expected no orphans, got: {orphans:?}");
    }

    #[test]
    fn orphaned_paths_ignores_a_language_not_evaluated_this_run() {
        // Simulates a `--lang`-filtered (or cached/skipped) run: the key
        // isn't in `current.expected`/`current.missing` at all, even though
        // the previous manifest still lists it as alef-owned. The still-valid
        // file on disk must survive.
        let previous = generated_ledger();
        let current = SnippetCoverageLedger {
            format_version: COVERAGE_MANIFEST_VERSION,
            ..SnippetCoverageLedger::default()
        };

        let orphans = orphaned_paths(&previous, &current);

        assert!(orphans.is_empty(), "expected no orphans, got: {orphans:?}");
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
