use crate::core::backend::GeneratedFile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStatus {
    Identical,
    Different,
    NoGeneratedEquivalent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationEntry {
    pub path: PathBuf,
    pub status: MigrationStatus,
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
