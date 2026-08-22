//! Docs-stage dispatch for `alef all`, split out of `all_commands.rs` so
//! wiring `--skip-snippet-validation` through does not push that file over
//! the 1,000-line file-modularization cap.

use std::path::Path;

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

/// Selects between `docs::generate_docs_stage` and
/// `docs::generate_docs_stage_without_snippet_compile_validation` based on
/// `alef all --skip-snippet-validation`. See that function's doc comment for
/// why the compile step is worth skipping on purpose rather than just
/// tolerating its failure. ~keep
pub(crate) fn generate(
    skip_snippet_validation: bool,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    workspace_root: &Path,
) -> (Vec<GeneratedFile>, anyhow::Result<()>) {
    if skip_snippet_validation {
        crate::docs::generate_docs_stage_without_snippet_compile_validation(
            api,
            config,
            languages,
            None,
            workspace_root,
        )
    } else {
        crate::docs::generate_docs_stage(api, config, languages, None, workspace_root)
    }
}
