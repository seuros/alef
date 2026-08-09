use crate::snippets::error::Result;
use crate::snippets::types::{Snippet, ValidationLevel, ValidationResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const CACHE_SCHEMA_VERSION: u32 = 2;

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
        let mut hasher = blake3::Hasher::new();
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
}
