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
}

pub fn compare_root(
    existing_root: &Path,
    generated_root: &Path,
    generated: &[GeneratedFile],
) -> Result<Vec<MigrationEntry>> {
    let existing = read_existing(existing_root)?;
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
            Ok(GeneratedFile {
                path: path.to_path_buf(),
                content: file.content.clone(),
                generated_header: file.generated_header,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(compare_existing(
        existing
            .iter()
            .map(|(path, content)| (path.as_path(), content.as_str())),
        &relative_generated,
    ))
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
    let expected: BTreeMap<_, _> = generated
        .iter()
        .map(|file| (file.path.as_path(), file.content.as_str()))
        .collect();
    existing
        .into_iter()
        .map(|(path, content)| MigrationEntry {
            path: path.to_path_buf(),
            status: match expected.get(path) {
                Some(expected) if *expected == content => MigrationStatus::Identical,
                Some(_) => MigrationStatus::Different,
                None => MigrationStatus::NoGeneratedEquivalent,
            },
        })
        .collect()
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
                },
                MigrationEntry {
                    path: PathBuf::from("python/topic/a.md"),
                    status: MigrationStatus::Identical,
                },
                MigrationEntry {
                    path: PathBuf::from("python/topic/b.md"),
                    status: MigrationStatus::Different,
                },
            ]
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
