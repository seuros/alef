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
/// Paths that are absent (already a miss via [`outputs_exist`]) or unreadable as UTF-8 return
/// `true`: the question is "was a stamped file modified after alef wrote it", and only a stamped
/// file can be asked. Genuinely unstampable outputs -- `generated_header: false` create-once seeds,
/// and formats with no comment syntax at all (`.json`, `.jar`) -- carry no alef marker either, so
/// they keep the existence-only rule and a warm run still hits.
///
/// A file that carries the marker but *no* `alef:hash:` line is the one unstamped shape that must
/// NOT be read as agreement. It is not an unstampable output: alef claims it, and
/// `hash::inject_hash_line` shares `content_has_alef_marker`'s scan window, so anything claimed is
/// stampable. The missing line therefore means the stamping pass never ran for it -- an
/// interrupted run, or a stage that aborted before `finalize_hashes`. Answering `true` there is
/// what makes that state permanent: the stage reads as cached, its `finalize_hashes` call is
/// skipped, and the file stays claimed-but-unstamped, so `poly`'s hash-keyed skip never covers it,
/// `poly fmt` reformats it, and the next alef write puts alef's own bytes back -- an unbreakable
/// ping-pong neither tool yields on. Repo-root scaffold files (`.cargo/config.toml`,
/// `rust-toolchain.toml`, `poly.toml`, `rustfmt.toml`) have no second route out: they sit outside
/// every `generate::orphans::generate_sweep_roots` root, so `finalize_hashes_sweeping`'s disk-scan
/// self-heal cannot reach them either. ~keep
pub(crate) fn stamped_outputs_agree_with_disk(manifest_path: &Path) -> bool {
    let Ok(manifest) = fs::read_to_string(manifest_path) else {
        return false;
    };
    for line in manifest.lines().filter(|line| !line.is_empty()) {
        let Ok(content) = fs::read_to_string(Path::new(line)) else {
            continue;
        };
        let Some(embedded) = crate::core::hash::extract_hash(&content) else {
            if crate::core::hash::content_has_alef_marker(&content) {
                tracing::debug!(
                    path = line,
                    "manifested output carries an alef marker but no alef:hash: line; treating the \
                     cache entry as a miss so the stamping pass runs again"
                );
                return false;
            }
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
