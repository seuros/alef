use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A single text replacement rule for version sync. ~keep
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TextReplacement {
    /// Glob pattern for files to process. ~keep
    pub path: String,
    /// Regex pattern to search for (may contain `{version}` placeholder). ~keep
    pub search: String,
    /// Replacement string (may contain `{version}` placeholder). ~keep
    pub replace: String,
}

/// Configuration for the `sync-versions` command. ~keep
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct SyncConfig {
    /// Extra file paths to update version in (glob patterns). ~keep
    #[serde(default)]
    pub extra_paths: Vec<String>,
    /// Arbitrary text replacements applied during version sync. ~keep
    #[serde(default)]
    pub text_replacements: Vec<TextReplacement>,
}
