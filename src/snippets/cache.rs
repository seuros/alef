use crate::snippets::error::Result;
use crate::snippets::types::{Snippet, ValidationLevel, ValidationResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The `CacheEntry` struct's own on-disk shape, bumped only when that shape changes
/// incompatibly (a field added, removed, or retyped). This is deliberately independent of
/// [`ALEF_VALIDATOR_VERSION`] below -- it answers "can this JSON still deserialize", not "was
/// this verdict computed by the classification logic running today". ~keep
const CACHE_SCHEMA_VERSION: u32 = 2;

/// Alef defect #138: the cache key used to cover only the snippet's own content (language,
/// level, code, metadata) and the session fingerprint -- never anything that identifies *which
/// alef* produced the verdict. A validator classification fix (a `Fail` pattern narrowed, a
/// dependency-error heuristic corrected, an `achievable_level` gap reclassified) changes no byte
/// this hash was built from, so `alef snippets check` replayed the previous release's verdict
/// after every such fix, and the only working remedy consumers found was `--cache off` on every
/// run -- a permanent admission the cache could not be trusted to invalidate itself.
///
/// This crate is single-binary and root-flat (`alef`, both bin and lib): the snippet validators
/// under `src/snippets/validators/` ship at exactly the same version as everything else, so the
/// crate's own release version is a complete, zero-maintenance proxy for "which classification
/// logic computed this". Folding it into the hash means a stale entry from a different alef
/// version is simply a cache miss under a different filename, not a stored verdict a version
/// check has to remember to compare -- the same reasoning [`CACHE_SCHEMA_VERSION`] already uses
/// for the entry's own shape, extended to cover the logic that filled it in. ~keep
const ALEF_VALIDATOR_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    schema_version: u32,
    result: ValidationResult,
}

pub struct ValidationCache {
    directory: PathBuf,
}

impl ValidationCache {
    #[must_use]
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    #[must_use]
    pub fn key(snippet: &Snippet, level: ValidationLevel, session_fingerprint: Option<&str>) -> String {
        Self::key_for_validator_version(snippet, level, session_fingerprint, ALEF_VALIDATOR_VERSION)
    }

    /// The actual hash, parameterized on the validator version so a test can prove two different
    /// versions of alef never share a cache key without needing two real builds. Production code
    /// only ever reaches this through [`Self::key`], which always passes
    /// [`ALEF_VALIDATOR_VERSION`].
    fn key_for_validator_version(
        snippet: &Snippet,
        level: ValidationLevel,
        session_fingerprint: Option<&str>,
        validator_version: &str,
    ) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(validator_version.as_bytes());
        hasher.update(snippet.language.to_string().as_bytes());
        hasher.update(level.to_string().as_bytes());
        hasher.update(snippet.code.as_bytes());
        hasher.update(format!("{:?}", snippet.metadata).as_bytes());
        if let Some(fingerprint) = session_fingerprint {
            hasher.update(fingerprint.as_bytes());
        }
        hasher.finalize().to_hex().to_string()
    }

    pub fn load(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        session_fingerprint: Option<&str>,
    ) -> Option<ValidationResult> {
        let path = self.path_for(snippet, level, session_fingerprint);
        let content = std::fs::read_to_string(path).ok()?;
        let entry: CacheEntry = serde_json::from_str(&content).ok()?;
        (entry.schema_version == CACHE_SCHEMA_VERSION).then_some(entry.result)
    }

    /// # Errors
    ///
    /// Returns an error when the cache directory or entry cannot be written.
    pub fn store(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        session_fingerprint: Option<&str>,
        result: &ValidationResult,
    ) -> Result<()> {
        std::fs::create_dir_all(&self.directory)?;
        let entry = CacheEntry {
            schema_version: CACHE_SCHEMA_VERSION,
            result: result.clone(),
        };
        std::fs::write(
            self.path_for(snippet, level, session_fingerprint),
            serde_json::to_vec_pretty(&entry)?,
        )?;
        Ok(())
    }

    fn path_for(&self, snippet: &Snippet, level: ValidationLevel, session_fingerprint: Option<&str>) -> PathBuf {
        self.directory
            .join(format!("{}.json", Self::key(snippet, level, session_fingerprint)))
    }
}

#[must_use]
pub fn default_cache_dir(root: &Path) -> PathBuf {
    root.join(".alef/snippets")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{Language, SnippetMetadata, SourceOrigin};

    fn snippet(code: &str) -> Snippet {
        let path = PathBuf::from("example.md");
        Snippet {
            id: None,
            path: path.clone(),
            language: Language::Rust,
            title: None,
            code: code.to_string(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path,
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn cache_key_changes_with_content_and_level() {
        let first = snippet("fn main() {}");
        let second = snippet("fn main() { panic!() }");
        assert_ne!(
            ValidationCache::key(&first, ValidationLevel::Syntax, None),
            ValidationCache::key(&second, ValidationLevel::Syntax, None)
        );
        assert_ne!(
            ValidationCache::key(&first, ValidationLevel::Syntax, None),
            ValidationCache::key(&first, ValidationLevel::Run, None)
        );
        assert_ne!(
            ValidationCache::key(&first, ValidationLevel::Run, Some("binding-a")),
            ValidationCache::key(&first, ValidationLevel::Run, Some("binding-b"))
        );
    }

    /// Alef defect #138's load-bearing assertion: two different alef releases must never agree on
    /// a cache key for the *same* snippet, content, level and session -- otherwise a validator
    /// classification fix ships and `alef snippets check` keeps serving the previous release's
    /// verdict for every snippet whose content never changed. Parameterized on the version string
    /// directly (`key_for_validator_version`, not `key`) because this test cannot build two real
    /// alef binaries; it instead pins the exact mechanism `key` delegates to. ~keep
    #[test]
    fn cache_key_changes_with_the_alef_version_that_computed_it() {
        let snippet = snippet("fn main() {}");

        let older = ValidationCache::key_for_validator_version(&snippet, ValidationLevel::Syntax, None, "0.64.0");
        let newer = ValidationCache::key_for_validator_version(&snippet, ValidationLevel::Syntax, None, "0.64.1");

        assert_ne!(
            older, newer,
            "a validator fix in a new alef release must invalidate every prior release's cache \
             entries, not replay their verdicts"
        );
    }

    /// `ValidationCache::key` (the production entry point every caller actually uses) must itself
    /// be wired to the running binary's own version, not just the version-parameterized helper in
    /// isolation -- this is what a hand-copied reimplementation of the fix would miss.
    #[test]
    fn the_public_key_function_is_pinned_to_this_build_own_alef_version() {
        let snippet = snippet("fn main() {}");

        assert_eq!(
            ValidationCache::key(&snippet, ValidationLevel::Syntax, None),
            ValidationCache::key_for_validator_version(
                &snippet,
                ValidationLevel::Syntax,
                None,
                env!("CARGO_PKG_VERSION")
            )
        );
    }
}
