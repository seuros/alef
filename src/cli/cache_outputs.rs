//! Manifest verification: does the tree a `.alef/` cache entry vouches for still hold?
//!
//! Split out of [`crate::cli::cache`] because it answers a different question from the rest of
//! that module. A cache key ([`crate::cli::cache_identity`]) says *what* was generated and by
//! which alef build; these two predicates say whether the files that generation produced are
//! still on disk and still unmodified. Both must hold before a skip is honest.
//!
//! [`outputs_exist`] alone was the whole check, and existence turned out not to be enough: a
//! consumer appended a line to a generated file, re-ran `alef generate` with `.alef/` intact,
//! and got `Generated 0 files` with the edit still in place — the file existed, so the language
//! stayed a cache hit and was dropped from the generation set before anything read it.
//! [`stamped_outputs_agree_with_disk`] is what makes the hit mean something. ~keep

use std::fs;
use std::path::Path;

/// Check that all files listed in a manifest exist on disk. False if any listed file is missing,
/// if the manifest is empty, or if the manifest could not be read at all.
///
/// Every failure to read is a cache **miss**, never a hit. A manifest is absent for three
/// different reasons -- the stage has never run, the cache predates the manifest format, or the
/// previous run died between `fs::write`ing the hash and writing the manifest
/// (`write_lang_hash` and `write_stage_hash` are two separate writes, so an interrupt leaves
/// exactly a matching hash with no manifest) -- and only the read itself can no longer tell them
/// apart. The callers spend this answer on "may I skip regenerating?", where an unknown costs one
/// regeneration, while a wrong `true` skips the write entirely and leaves the tree permanently
/// short of files the cache insists are present. An empty-but-readable manifest already answered
/// `false`; an unreadable one is strictly less evidence and must not answer better. ~keep
pub(crate) fn outputs_exist(manifest_path: &Path) -> bool {
    match fs::read_to_string(manifest_path) {
        Ok(content) => {
            let mut paths = content.lines().filter(|line| !line.is_empty()).peekable();
            paths.peek().is_some() && paths.all(|line| Path::new(line).exists())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(
                manifest = %manifest_path.display(),
                "no output manifest recorded; treating the cache entry as a miss"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                manifest = %manifest_path.display(),
                %error,
                "output manifest exists but could not be read; treating the cache entry as a miss"
            );
            false
        }
    }
}

/// Whether every stamped file in `manifest_path` still hashes to its own embedded `alef:hash:`
/// value.
///
/// This is the same comparison `alef verify` runs, so a tree that passes verify passes here.
/// Paths that are absent (already a miss via [`outputs_exist`]), unreadable as UTF-8, or carry no
/// marker return `true`: the question is "was a stamped file modified after alef wrote it", and
/// only a stamped file can be asked. Unstamped outputs -- `generated_header: false`, create-once
/// seeds -- must keep the existence-only rule or a warm run would never hit again.
///
/// `inputs_hash` is accepted, unused, purely so [`super::cache::is_lang_cached`] -- its one
/// caller -- keeps its own existing signature and every one of *its* callers stays unchanged:
/// see [`crate::core::hash::compute_file_hash`]'s doc for why the per-file stamp no longer
/// takes a generation-inputs argument at all. ~keep
pub(crate) fn stamped_outputs_agree_with_disk(manifest_path: &Path, _inputs_hash: &str) -> bool {
    let Ok(manifest) = fs::read_to_string(manifest_path) else {
        return false;
    };
    for line in manifest.lines().filter(|line| !line.is_empty()) {
        let Ok(content) = fs::read_to_string(Path::new(line)) else {
            continue;
        };
        let Some(embedded) = crate::core::hash::extract_hash(&content) else {
            continue;
        };
        if crate::core::hash::compute_file_hash(&content) != embedded {
            tracing::debug!(
                path = line,
                "manifested output no longer matches its embedded alef:hash:; treating the cache entry as a miss"
            );
            return false;
        }
    }
    true
}
