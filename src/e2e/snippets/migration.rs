use crate::core::backend::GeneratedFile;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    Identical,
    Different,
    NoGeneratedEquivalent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationEntry {
    pub path: PathBuf,
    pub status: MigrationStatus,
    /// True when `path` matches a `[crates.e2e.snippets].curated_snippets` glob -- always
    /// `false` from [`compare_root`] / [`compare_existing`], which know nothing about
    /// curated declarations. [`compare_root_curated`] / [`compare_existing_curated`] are the
    /// curated-aware siblings that populate it.
    ///
    /// A `NoGeneratedEquivalent` entry with `curated: true` is a file the project declared,
    /// on purpose, alef will never generate -- distinct from `curated: false`, a genuine,
    /// unaccounted migration gap. Before this field existed, the two were indistinguishable
    /// in a migration comparison: a project with hundreds of intentionally hand-authored
    /// snippets saw every one of them reported identically to a real gap.
    #[serde(default)]
    pub curated: bool,
}

pub fn compare_root(
    existing_root: &Path,
    generated_root: &Path,
    generated: &[GeneratedFile],
) -> Result<Vec<MigrationEntry>> {
    compare_root_curated(existing_root, generated_root, generated, &[])
}

/// [`compare_root`]'s curated-aware sibling: `curated_globs` are
/// `[crates.e2e.snippets].curated_snippets` patterns (relative to `existing_root`, matching
/// the convention that field already uses for the generated output tree), matched against
/// every `NoGeneratedEquivalent` path to populate [`MigrationEntry::curated`].
///
/// A pattern is validated the same way [`crate::e2e::snippets::coverage::resolve_curated_snippet_paths`]
/// validates it for the coverage ledger -- invalid glob syntax fails the comparison rather
/// than silently matching nothing -- but does NOT repeat that function's "must match at
/// least one file" anti-vacuity check: a migration comparison walks `existing_root` itself,
/// so a pattern matching nothing here already shows up as a visibly empty count in the
/// caller's own report, unlike the coverage ledger, which has no equivalent per-pattern
/// visibility.
pub fn compare_root_curated(
    existing_root: &Path,
    generated_root: &Path,
    generated: &[GeneratedFile],
    curated_globs: &[String],
) -> Result<Vec<MigrationEntry>> {
    let existing = read_existing(existing_root)?;
    let nested_prefix = nested_generated_prefix(existing_root, generated_root);
    let relative_generated = generated
        .iter()
        .map(|file| {
            let path = file.path.strip_prefix(generated_root).with_context(|| {
                format!(
                    "generated snippet {} is outside configured output {}",
                    file.path.display(),
                    generated_root.display()
                )
            })?;
            let path = match &nested_prefix {
                Some(prefix) => prefix.join(path),
                None => path.to_path_buf(),
            };
            Ok(GeneratedFile {
                path,
                content: file.content.clone(),
                generated_header: file.generated_header,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    compare_existing_curated(
        existing
            .iter()
            .map(|(path, content)| (path.as_path(), content.as_str())),
        &relative_generated,
        curated_globs,
    )
}

/// Where `generated_root` sits relative to `existing_root`, when alef's configured output tree
/// lives INSIDE the tree being migrated.
///
/// Both sides of the comparison have to be keyed off one base. For parallel trees -- a
/// handwritten `docs/handwritten` against `output = "docs/generated"` -- each side keys off its
/// own root and this is `None`, which is the original behaviour. When `output` is a subdirectory
/// of `existing_root` (`alef e2e snippets-migrate docs/snippets` against
/// `output = "docs/snippets/generated"`) the walk of `existing_root` enumerates alef's own output
/// under `generated/...`, so the generated side must carry the same prefix or the two key spaces
/// are disjoint by construction and every file alef just wrote reports as a migration gap. ~keep
///
/// The lexical `strip_prefix` answers the CLI's own shape, where both paths are project-relative;
/// the absolute retry covers a caller mixing an absolute root with a relative configured output.
/// Identical roots yield an empty prefix, which is `None` rather than a no-op join.
fn nested_generated_prefix(existing_root: &Path, generated_root: &Path) -> Option<PathBuf> {
    let non_empty = |prefix: PathBuf| (!prefix.as_os_str().is_empty()).then_some(prefix);
    if let Ok(prefix) = generated_root.strip_prefix(existing_root) {
        return non_empty(prefix.to_path_buf());
    }
    let existing = std::path::absolute(existing_root).ok()?;
    let generated = std::path::absolute(generated_root).ok()?;
    non_empty(generated.strip_prefix(&existing).ok()?.to_path_buf())
}

fn read_existing(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    if !root.is_dir() {
        bail!("existing snippet root is not a directory: {}", root.display());
    }
    let mut files = Vec::new();
    read_directory(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn read_directory(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, String)>) -> Result<()> {
    let entries =
        fs::read_dir(directory).with_context(|| format!("failed to read snippet directory {}", directory.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read entry in {}", directory.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        if file_type.is_dir() {
            read_directory(root, &entry.path(), files)?;
        } else if file_type.is_file() {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("failed to relativize {}", path.display()))?
                .to_path_buf();
            let content = fs::read_to_string(&path)
                .with_context(|| format!("failed to read snippet {} as UTF-8", path.display()))?;
            files.push((relative, content));
        }
    }
    Ok(())
}

pub fn compare_existing<'a>(
    existing: impl IntoIterator<Item = (&'a Path, &'a str)>,
    generated: &[GeneratedFile],
) -> Vec<MigrationEntry> {
    compare_existing_curated(existing, generated, &[]).expect("no curated globs to compile means this can never fail")
}

/// [`compare_existing`]'s curated-aware sibling: see [`compare_root_curated`] for the field
/// this populates and why it exists.
pub fn compare_existing_curated<'a>(
    existing: impl IntoIterator<Item = (&'a Path, &'a str)>,
    generated: &[GeneratedFile],
    curated_globs: &[String],
) -> Result<Vec<MigrationEntry>> {
    let compiled_globs = curated_globs
        .iter()
        .map(|pattern| glob::Pattern::new(pattern).with_context(|| format!("invalid curated snippet glob `{pattern}`")))
        .collect::<Result<Vec<_>>>()?;
    let expected: BTreeMap<_, _> = generated
        .iter()
        .map(|file| (file.path.as_path(), file.content.as_str()))
        .collect();
    Ok(existing
        .into_iter()
        .map(|(path, content)| {
            let status = match expected.get(path) {
                Some(expected) if *expected == content => MigrationStatus::Identical,
                Some(_) => MigrationStatus::Different,
                None => MigrationStatus::NoGeneratedEquivalent,
            };
            // Curated is only meaningful alongside `NoGeneratedEquivalent`: it answers "is
            // this the *declared* absence of a generated equivalent, or a genuine gap" --
            // a path that DOES have a generated equivalent is Identical/Different regardless
            // of what any curated glob says about it.
            let curated = status == MigrationStatus::NoGeneratedEquivalent
                && compiled_globs.iter().any(|pattern| pattern.matches_path(path));
            MigrationEntry {
                path: path.to_path_buf(),
                status,
                curated,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_root_recurses_and_reports_stable_relative_paths() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(directory.path().join("python/topic")).expect("create nested directory");
        fs::write(directory.path().join("python/topic/a.md"), "same").expect("write identical snippet");
        fs::write(directory.path().join("python/topic/b.md"), "old").expect("write different snippet");
        fs::write(directory.path().join("orphan.md"), "manual").expect("write orphan snippet");
        let generated = vec![
            generated("docs/generated/python/topic/a.md", "same"),
            generated("docs/generated/python/topic/b.md", "new"),
        ];

        let entries =
            compare_root(directory.path(), Path::new("docs/generated"), &generated).expect("compare snippets");

        assert_eq!(
            entries,
            vec![
                MigrationEntry {
                    path: PathBuf::from("orphan.md"),
                    status: MigrationStatus::NoGeneratedEquivalent,
                    curated: false,
                },
                MigrationEntry {
                    path: PathBuf::from("python/topic/a.md"),
                    status: MigrationStatus::Identical,
                    curated: false,
                },
                MigrationEntry {
                    path: PathBuf::from("python/topic/b.md"),
                    status: MigrationStatus::Different,
                    curated: false,
                },
            ]
        );
    }

    /// The curated-declaration side of the migration comparison: a hand-authored file with
    /// no generated equivalent, matching a `curated_snippets` glob, must classify as
    /// `NoGeneratedEquivalent` with `curated: true` -- distinct from an unrelated
    /// hand-authored file with no glob match, which stays `curated: false`. Both remain
    /// `NoGeneratedEquivalent`; the flag is what a caller filters a real migration gap on.
    #[test]
    fn compare_root_curated_flags_paths_a_curated_glob_claims() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(directory.path().join("docker")).expect("create curated directory");
        fs::write(directory.path().join("docker/quick-start.md"), "curated by hand").expect("write curated snippet");
        fs::write(directory.path().join("orphan.md"), "manual").expect("write uncurated orphan snippet");
        let generated: Vec<GeneratedFile> = Vec::new();

        let entries = compare_root_curated(
            directory.path(),
            Path::new("docs/generated"),
            &generated,
            &["docker/*.md".to_string()],
        )
        .expect("curated comparison succeeds");

        assert_eq!(
            entries,
            vec![
                MigrationEntry {
                    path: PathBuf::from("docker/quick-start.md"),
                    status: MigrationStatus::NoGeneratedEquivalent,
                    curated: true,
                },
                MigrationEntry {
                    path: PathBuf::from("orphan.md"),
                    status: MigrationStatus::NoGeneratedEquivalent,
                    curated: false,
                },
            ]
        );
    }

    /// A curated glob must never retroactively make a real gap disappear: it only annotates
    /// `NoGeneratedEquivalent` entries, so a path that DOES have a generated equivalent stays
    /// `Identical`/`Different` regardless of whether some curated pattern also happens to
    /// match its name.
    #[test]
    fn a_curated_glob_matching_a_path_with_a_real_generated_equivalent_leaves_its_status_untouched() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(directory.path().join("python")).expect("create directory");
        fs::write(directory.path().join("python/example.md"), "old").expect("write stale snippet");
        let generated = vec![generated("docs/generated/python/example.md", "new")];

        let entries = compare_root_curated(
            directory.path(),
            Path::new("docs/generated"),
            &generated,
            &["python/*.md".to_string()],
        )
        .expect("curated comparison succeeds");

        assert_eq!(
            entries,
            vec![MigrationEntry {
                path: PathBuf::from("python/example.md"),
                status: MigrationStatus::Different,
                curated: false,
            }]
        );
    }

    #[test]
    fn an_invalid_curated_glob_fails_the_comparison_rather_than_silently_matching_nothing() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::write(directory.path().join("orphan.md"), "manual").expect("write orphan snippet");
        let generated: Vec<GeneratedFile> = Vec::new();

        let error = compare_root_curated(
            directory.path(),
            Path::new("docs/generated"),
            &generated,
            &["[unterminated".to_string()],
        )
        .expect_err("an invalid glob pattern must fail rather than silently match nothing");

        assert!(error.to_string().contains("invalid curated snippet glob"), "{error}");
    }

    /// The nested-root defect: `alef e2e snippets-migrate docs/snippets` against a project
    /// whose `[crates.e2e.snippets].output` is `docs/snippets/generated` -- the generated tree
    /// lives INSIDE the tree being migrated.
    ///
    /// The walk of `existing_root` enumerates alef's own output under an `existing_root`-relative
    /// key (`generated/python/a.md`) while the generated list was keyed against `output`
    /// (`python/a.md`). The two key spaces are disjoint by construction, so every file alef had
    /// just written reported as `NoGeneratedEquivalent`. One consumer saw 7796 files it had
    /// generated itself reported as migration gaps this way.
    #[test]
    fn a_generated_tree_nested_inside_the_migrated_root_is_matched_not_reported_as_a_gap() {
        let directory = tempfile::tempdir().expect("tempdir");
        let existing_root = directory.path();
        let generated_root = existing_root.join("generated");
        fs::create_dir_all(generated_root.join("python")).expect("create generated tree");
        fs::create_dir_all(existing_root.join("cli")).expect("create hand-authored tree");
        fs::write(generated_root.join("python/a.md"), "alef:hash:abc\nsame").expect("write fresh generated snippet");
        fs::write(generated_root.join("python/b.md"), "alef:hash:def\nstale").expect("write stale generated snippet");
        fs::write(existing_root.join("cli/quickstart.md"), "by hand").expect("write hand-authored snippet");
        let generated = vec![
            generated(
                &generated_root.join("python/a.md").to_string_lossy(),
                "alef:hash:abc\nsame",
            ),
            generated(
                &generated_root.join("python/b.md").to_string_lossy(),
                "alef:hash:def\nfresh",
            ),
        ];

        let entries = compare_root(existing_root, &generated_root, &generated).expect("compare snippets");

        assert_eq!(
            entries,
            vec![
                MigrationEntry {
                    path: PathBuf::from("cli/quickstart.md"),
                    status: MigrationStatus::NoGeneratedEquivalent,
                    curated: false,
                },
                MigrationEntry {
                    path: PathBuf::from("generated/python/a.md"),
                    status: MigrationStatus::Identical,
                    curated: false,
                },
                MigrationEntry {
                    path: PathBuf::from("generated/python/b.md"),
                    status: MigrationStatus::Different,
                    curated: false,
                },
            ],
            "a file alef itself generates must never be reported as having no generated equivalent"
        );
    }

    fn generated(path: &str, content: &str) -> GeneratedFile {
        GeneratedFile {
            path: PathBuf::from(path),
            content: content.into(),
            generated_header: false,
        }
    }
}
