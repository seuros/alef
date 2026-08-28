use crate::snippets::error::Result;
use crate::snippets::types::{SideEffectClass, Snippet, ValidationLevel, ValidationResult};
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

    /// A stable, deterministic identifier for one `(snippet, level, session)` triple.
    ///
    /// This is **not** the cache-invalidation key -- see [`Self::invalidation_key`] for that. It
    /// is reused as-is by the Java/Rust/Kotlin/C# batch validators purely to name deterministic
    /// per-snippet compilation-unit directories/files, a purpose that has nothing to do with
    /// whether a cached verdict should survive an annotation or policy edit. Narrowing this
    /// function to only what those callers need (and *not* widening it to everything
    /// [`Self::invalidation_key`] covers) keeps that unrelated naming scheme stable and avoids
    /// forcing every batch validator to thread new parameters through for no behavioral benefit
    /// to them. ~keep
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

    /// The cache-invalidation key actually used to read and write `.alef/snippets/*.json`.
    ///
    /// Alef defect #554: [`Self::key`] covered the snippet's own content (language, level, code,
    /// `metadata`) and the session fingerprint, but never `snippet.annotation` -- the
    /// `<!-- snippet:*-only -->`/`<!-- snippet:skip -->` comment a doc author edits directly above
    /// a fenced block -- nor the `docs.snippets` side-effect policy (`deny_unclassified`,
    /// `allowed_side_effects`) that `runner::side_effect_rejection` reads before a `Run`-level
    /// snippet is even handed to a validator. Both feed `runner::classify_result` /
    /// `runner::side_effect_rejection` and can change a verdict for byte-identical snippet
    /// content, so a consumer correcting only the annotation on six TOML snippets kept replaying
    /// the previous run's `Downgraded`/`capability_capped` verdict through every `alef all` until
    /// an explicit `alef cache clear` -- and, in the dangerous direction nobody reported, a
    /// snippet whose annotation or side-effect policy newly makes it *fail* would just as silently
    /// keep serving a stale cached `Pass`. ~keep
    fn invalidation_key(
        snippet: &Snippet,
        level: ValidationLevel,
        session_fingerprint: Option<&str>,
        deny_unclassified: bool,
        allowed_side_effects: &[SideEffectClass],
    ) -> String {
        Self::invalidation_key_for_validator_version(
            snippet,
            level,
            session_fingerprint,
            deny_unclassified,
            allowed_side_effects,
            ALEF_VALIDATOR_VERSION,
        )
    }

    /// [`Self::invalidation_key`] with the alef version supplied by the caller, so a test can
    /// observe the effect of an alef upgrade without rebuilding the binary -- mirrors
    /// [`Self::key_for_validator_version`]'s reason for existing.
    fn invalidation_key_for_validator_version(
        snippet: &Snippet,
        level: ValidationLevel,
        session_fingerprint: Option<&str>,
        deny_unclassified: bool,
        allowed_side_effects: &[SideEffectClass],
        validator_version: &str,
    ) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(validator_version.as_bytes());
        hasher.update(snippet.language.to_string().as_bytes());
        hasher.update(level.to_string().as_bytes());
        hasher.update(snippet.code.as_bytes());
        hasher.update(format!("{:?}", snippet.metadata).as_bytes());
        hasher.update(format!("{:?}", snippet.annotation).as_bytes());
        hasher.update(deny_unclassified.to_string().as_bytes());
        hasher.update(format!("{allowed_side_effects:?}").as_bytes());
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
        deny_unclassified: bool,
        allowed_side_effects: &[SideEffectClass],
    ) -> Option<ValidationResult> {
        let path = self.path_for(
            snippet,
            level,
            session_fingerprint,
            deny_unclassified,
            allowed_side_effects,
        );
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
        deny_unclassified: bool,
        allowed_side_effects: &[SideEffectClass],
        result: &ValidationResult,
    ) -> Result<()> {
        std::fs::create_dir_all(&self.directory)?;
        let entry = CacheEntry {
            schema_version: CACHE_SCHEMA_VERSION,
            result: result.clone(),
        };
        std::fs::write(
            self.path_for(
                snippet,
                level,
                session_fingerprint,
                deny_unclassified,
                allowed_side_effects,
            ),
            serde_json::to_vec_pretty(&entry)?,
        )?;
        Ok(())
    }

    fn path_for(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        session_fingerprint: Option<&str>,
        deny_unclassified: bool,
        allowed_side_effects: &[SideEffectClass],
    ) -> PathBuf {
        self.directory.join(format!(
            "{}.json",
            Self::invalidation_key(
                snippet,
                level,
                session_fingerprint,
                deny_unclassified,
                allowed_side_effects
            )
        ))
    }
}

#[must_use]
pub fn default_cache_dir(root: &Path) -> PathBuf {
    root.join(".alef/snippets")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{
        Language, SnippetAnnotation, SnippetAnnotationKind, SnippetMetadata, SnippetStatus, SourceOrigin,
    };

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

    /// `ValidationCache::key` (the batch-naming entry point every batch validator actually calls)
    /// must itself be wired to the running binary's own version, not just the
    /// version-parameterized helper in isolation -- this is what a hand-copied reimplementation
    /// of the fix would miss.
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

    fn passing_result(snippet: &Snippet, level: ValidationLevel) -> ValidationResult {
        ValidationResult {
            snippet: snippet.clone(),
            status: SnippetStatus::Pass,
            level,
            requested_level: level,
            effective_level: level,
            message: None,
            duration_ms: 1,
            capability_capped: false,
            downgrade_reason: None,
            unresolved_dependency: false,
        }
    }

    /// The cheap, string-level half of the fix: `invalidation_key` must separate an annotation
    /// edit and a side-effect-policy edit from each other and from the unmodified baseline, all
    /// while every other input stays byte-identical. The decisive proof that this actually blocks
    /// a stale disk read is the `ValidationCache::store`/`load` round trip below -- this test only
    /// pins the hash inputs so a regression here is cheap to catch without touching disk. ~keep
    #[test]
    fn invalidation_key_separates_annotation_and_side_effect_policy() {
        let plain = snippet("value = 1");
        let mut annotated = plain.clone();
        annotated.annotation = Some(SnippetAnnotation {
            kind: SnippetAnnotationKind::SyntaxOnly,
            reason: None,
        });

        let base = ValidationCache::invalidation_key(&plain, ValidationLevel::Compile, None, false, &[]);
        assert_ne!(
            base,
            ValidationCache::invalidation_key(&annotated, ValidationLevel::Compile, None, false, &[]),
            "an annotation-only change must move the invalidation key"
        );
        assert_ne!(
            base,
            ValidationCache::invalidation_key(&plain, ValidationLevel::Compile, None, true, &[]),
            "a deny_unclassified-only change must move the invalidation key"
        );
        assert_ne!(
            base,
            ValidationCache::invalidation_key(
                &plain,
                ValidationLevel::Compile,
                None,
                false,
                &[SideEffectClass::Network]
            ),
            "an allowed_side_effects-only change must move the invalidation key"
        );
        assert_eq!(
            base,
            ValidationCache::invalidation_key(&plain, ValidationLevel::Compile, None, false, &[]),
            "the invalidation key must be deterministic for unchanged inputs"
        );
    }

    /// Alef defect #554, direction 1 -- the reported symptom: a consumer corrected a
    /// `<!-- snippet:*-only -->` annotation on six TOML snippets and `alef all` kept reporting the
    /// old `Downgraded`/`capability_capped` verdict until an explicit `alef cache clear`. Goes
    /// through the real `ValidationCache::store`/`load` round trip -- the exact API
    /// `runner::cached_result`/`runner::finalize_result` call -- so this proves the actual read
    /// path a validation run takes, not just that two key strings differ in isolation. ~keep
    #[test]
    fn cache_load_misses_when_only_the_annotation_changes() {
        let directory = tempfile::tempdir().expect("cache directory");
        let cache = ValidationCache::new(directory.path().into());
        let mut original = snippet("value = 1");
        original.language = Language::Toml;
        let cached = passing_result(&original, ValidationLevel::Compile);
        cache
            .store(&original, ValidationLevel::Compile, None, false, &[], &cached)
            .expect("store cache entry");

        assert!(
            cache
                .load(&original, ValidationLevel::Compile, None, false, &[])
                .is_some(),
            "querying with the exact snippet that wrote the cache must still be a hit"
        );

        let mut annotated = original.clone();
        annotated.annotation = Some(SnippetAnnotation {
            kind: SnippetAnnotationKind::SyntaxOnly,
            reason: Some("author correction".to_string()),
        });
        assert!(
            cache
                .load(&annotated, ValidationLevel::Compile, None, false, &[])
                .is_none(),
            "correcting only the snippet's annotation must miss the cache, not replay the \
             previous annotation's verdict -- in either direction: a snippet whose corrected \
             annotation should now pass must not keep reporting the old downgrade, and a snippet \
             whose new annotation should now fail must not keep reporting the old cached pass"
        );
    }

    /// Alef defect #554, direction 2: the `docs.snippets` side-effect policy
    /// (`deny_unclassified`/`allowed_side_effects`) feeds `runner::side_effect_rejection` before a
    /// `Run`-level snippet is even handed to a validator. An `alef.toml` edit to that policy must
    /// miss the cache the same way an annotation edit does, with code, metadata, level, and
    /// session held byte-identical -- otherwise tightening the policy would keep replaying a
    /// `Pass` computed under the old, looser policy: the dangerous direction, since nothing about
    /// that stale `Pass` announces it was never re-checked against the new rule. ~keep
    #[test]
    fn cache_load_misses_when_only_the_side_effect_policy_changes() {
        let directory = tempfile::tempdir().expect("cache directory");
        let cache = ValidationCache::new(directory.path().into());
        let side_effecting = snippet("run_side_effecting_thing();");
        let cached = passing_result(&side_effecting, ValidationLevel::Run);
        cache
            .store(&side_effecting, ValidationLevel::Run, None, false, &[], &cached)
            .expect("store cache entry");

        assert!(
            cache
                .load(&side_effecting, ValidationLevel::Run, None, false, &[])
                .is_some(),
            "querying with the exact policy that wrote the cache must still be a hit"
        );
        assert!(
            cache
                .load(&side_effecting, ValidationLevel::Run, None, true, &[])
                .is_none(),
            "tightening deny_unclassified must miss the cache, not replay a Pass computed under \
             the previous, looser policy"
        );
        assert!(
            cache
                .load(
                    &side_effecting,
                    ValidationLevel::Run,
                    None,
                    false,
                    &[SideEffectClass::Network]
                )
                .is_none(),
            "widening allowed_side_effects must also miss the cache"
        );
    }
}
