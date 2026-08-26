//! Declarative configuration for MCP surfaces that attribute extraction cannot see.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Static extraction config for rmcp-style MCP reference docs, plus a declarative fallback
/// for surfaces that static extraction cannot see at all.
///
/// `rust_static::extract_mcp_surface` finds tools, prompts, and resources by scanning method
/// attributes (`#[tool]`, `#[prompt]`, `#[resource]`). That works only for surfaces *declared*
/// that way. Some consumers instead *construct* prompts/resources at runtime (for example a
/// `Prompt::new(...)` or `Resource::new(...)` call assembled from a config file or a loop), and
/// no attribute exists for extraction to find. `declared` is the fallback for exactly that
/// condition — MCP surfaces constructed at runtime rather than declared by attribute — not a
/// general-purpose override of attribute extraction.
///
/// This is intentionally a second source of truth rather than lifting constructor-call
/// arguments: lifting `Prompt::new(...)` arguments only works while every argument is a string
/// literal, and it degrades silently the moment one argument becomes computed (a `format!`, a
/// variable, a loop). A declarative list is bounded and honest about being hand-maintained,
/// instead of quietly falling back to guesses.
///
/// This field lives on `DocsMcpConfig` rather than the shared `DocsSourceConfig` used by
/// `docs.cli` on purpose: Clap CLI surfaces are always declared through struct/enum derives, so
/// a `declared` escape hatch would be a no-op there — accepted by config, silently ignored by
/// extraction. That is the same "examined nothing" shape this fallback exists to fix, so the
/// field only exists where it does something.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocsMcpConfig {
    /// Enable this reference extractor. Defaults to true when the table exists.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Rust source files to parse for this reference surface. When empty, Alef
    /// falls back to the crate source list.
    #[serde(default)]
    pub sources: Vec<PathBuf>,
    /// Output markdown file. Relative paths are resolved from the repository root.
    /// When unset, Alef writes into `reference_output`.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Allow the first render to replace an existing unmanaged output file.
    /// Defaults to false to avoid clobbering hand-authored CLI/MCP docs.
    #[serde(default)]
    pub adopt_existing: bool,
    /// Tools, prompts, and resources that are constructed at runtime and therefore invisible
    /// to attribute-based extraction. Additive: entries here are appended to what extraction
    /// finds. When a declared entry's `(kind, name)` matches an attribute-derived item, the
    /// attribute-derived item wins and the declared entry is dropped with a warning, since a
    /// collision means the declared entry is stale (the surface has since gained an attribute)
    /// rather than a genuine second copy. Consumers who declare nothing see no behavior
    /// change and no new warnings.
    #[serde(default)]
    pub declared: Vec<DeclaredMcpItem>,
}

impl DocsMcpConfig {
    #[must_use]
    pub fn merge(workspace: Option<&Self>, krate: Option<&Self>) -> Option<Self> {
        if workspace.is_none() && krate.is_none() {
            return None;
        }
        let sources = krate
            .filter(|cfg| !cfg.sources.is_empty())
            .map(|cfg| cfg.sources.clone())
            .or_else(|| {
                workspace
                    .filter(|cfg| !cfg.sources.is_empty())
                    .map(|cfg| cfg.sources.clone())
            })
            .unwrap_or_default();
        let declared = krate
            .filter(|cfg| !cfg.declared.is_empty())
            .map(|cfg| cfg.declared.clone())
            .or_else(|| {
                workspace
                    .filter(|cfg| !cfg.declared.is_empty())
                    .map(|cfg| cfg.declared.clone())
            })
            .unwrap_or_default();
        Some(Self {
            enabled: krate
                .and_then(|cfg| cfg.enabled)
                .or_else(|| workspace.and_then(|cfg| cfg.enabled)),
            sources,
            output: krate
                .and_then(|cfg| cfg.output.clone())
                .or_else(|| workspace.and_then(|cfg| cfg.output.clone())),
            adopt_existing: krate
                .map(|cfg| cfg.adopt_existing)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.adopt_existing).unwrap_or(false)),
            declared,
        })
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
}

/// A single MCP tool, prompt, or resource declared in config because it is constructed at
/// runtime rather than through a `#[tool]`/`#[prompt]`/`#[resource]` attribute. See
/// [`DocsMcpConfig::declared`] for when this fallback applies.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredMcpItem {
    /// Which MCP surface this entry belongs to.
    pub kind: DeclaredMcpKind,
    /// The name the MCP client sees.
    pub name: String,
    /// Human-readable title. Defaults to a title-cased `name`, matching attribute extraction.
    #[serde(default)]
    pub title: Option<String>,
    /// Human-readable description shown in the reference page.
    #[serde(default)]
    pub description: Option<String>,
    /// Rendered parameter type name, if any.
    #[serde(default)]
    pub params_type: Option<String>,
    /// Free-form annotations (e.g. `read_only_hint`), matching attribute extraction's shape.
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

/// The MCP surface a [`DeclaredMcpItem`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeclaredMcpKind {
    Tool,
    Prompt,
    Resource,
}
