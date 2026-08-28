//! Freshness reporting for a consumer's vendored `alef.toml` JSON Schema.
//!
//! `schemas/alef.schema.json` is alef's *own* config schema -- the JSON Schema for `alef.toml`,
//! rendered from `core::config::NewAlefConfig` -- which a consumer may vendor so their editor can
//! validate `alef.toml` offline. Nothing in `alef generate`/`alef build`/`alef all` produces it:
//! it is not derived from the consumer's Rust sources, no build step reads it, and only the
//! explicit `alef schema` command ever writes it. Before this module, nothing observed it either,
//! so a vendored copy left behind by an alef upgrade kept validating `alef.toml` against a
//! different release's config surface -- a silent wrong answer in the editor, with no failing
//! command anywhere to notice it.
//!
//! Two deliberate scope decisions, both narrower than "treat it as a generated file":
//!
//! * **Report, never write.** Writing `schemas/alef.schema.json` from `alef all` would create a
//!   directory and a file in every consumer repo that never asked for one, including the ones
//!   that reference the schema by its `$id` URL or not at all. This is the same policy
//!   `cli::version_pin` already settled for `[workspace] alef_version`, the other consumer-tree
//!   value that is a function of the alef version: check on every run, rewrite only behind an
//!   explicit opt-in. JSON also cannot carry the `alef:hash:` marker (see `core::hash`'s module
//!   doc), so the file could only ever be path-tracked in the ownership manifest -- the weakest
//!   form of the generated-file machinery, and not the one that would have caught this.
//! * **Only the path alef's own command defaults to, and only when it already exists.** A tree
//!   with no vendored copy gets no finding, because there is nothing there to be wrong. A
//!   consumer who keeps theirs somewhere else gets no finding either; that is a deliberate
//!   false-negative in exchange for alef never inventing a path in someone else's repository.
//!
//! Only [`SchemaDrift::Shape`] gates the exit code. A version-stamp-only difference is the
//! expected steady state after every alef release for every consumer who vendors the schema, and
//! it changes no answer their editor gives -- failing on it would make `alef verify` red on every
//! upgrade for a cosmetic reason, which is exactly how a release gate becomes something operators
//! route around (see the create-once/frozen-file precedent in `core_commands::verify`). ~keep

use std::path::{Path, PathBuf};

use crate::core::config::{DEFAULT_SCHEMA_PATH, SchemaDrift, classify_alef_config_schema};

/// A vendored schema copy that differs from what the running alef renders.
pub(super) struct VendoredSchemaFinding {
    path: PathBuf,
    drift: SchemaDrift,
}

impl VendoredSchemaFinding {
    /// True when the vendored copy describes a different `alef.toml` surface than this alef, and
    /// so must gate `alef verify`'s exit code.
    pub(super) fn describes_a_different_surface(&self) -> bool {
        self.drift.describes_a_different_surface()
    }

    /// The report lines for this finding, headline first.
    pub(super) fn report_lines(&self) -> Vec<String> {
        let display = self.path.display();
        match &self.drift {
            SchemaDrift::None => Vec::new(),
            SchemaDrift::Shape => vec![
                "Vendored alef config schema describes a different alef.toml surface than this alef \
                 (an editor validating alef.toml against this copy is answering for a different \
                 release -- regenerate it):"
                    .to_string(),
                format!("  {display} -- fix with: alef schema --output {display}"),
            ],
            SchemaDrift::SurfaceUnchanged {
                found_version,
                expected_version,
            } => {
                let detail = match found_version.as_deref() {
                    Some(found) if found == expected_version.as_str() => {
                        format!("stamped {found}, but reserialized -- byte differences are formatting only")
                    }
                    Some(found) => format!("stamped {found}, running alef {expected_version}"),
                    None => format!("carries no version stamp; running alef {expected_version}"),
                };
                vec![
                    "Vendored alef config schema differs from this alef's (informational -- the \
                     described alef.toml surface is unchanged, so editor validation is still \
                     correct; refresh it whenever convenient):"
                        .to_string(),
                    format!("  {display} -- {detail}; refresh with: alef schema --output {display}"),
                ]
            }
        }
    }
}

/// Classify the vendored schema at `base_dir`'s [`DEFAULT_SCHEMA_PATH`], if there is one.
///
/// `None` when the file does not exist (nothing vendored, nothing to be wrong), when it matches,
/// or when it cannot be read at all -- an unreadable file is a filesystem problem `alef verify`
/// has no remedy to offer for, and reporting it as schema drift would name the wrong cause.
pub(super) fn find_stale_vendored_schema(base_dir: &Path, cli_version: &str) -> Option<VendoredSchemaFinding> {
    let path = base_dir.join(DEFAULT_SCHEMA_PATH);
    if !path.is_file() {
        return None;
    }
    let drift = match classify_alef_config_schema(&path, cli_version) {
        Ok(drift) => drift,
        Err(error) => {
            tracing::debug!(
                "could not classify vendored config schema {}: {error:#}",
                path.display()
            );
            return None;
        }
    };
    if drift == SchemaDrift::None {
        return None;
    }
    Some(VendoredSchemaFinding { path, drift })
}

#[cfg(test)]
mod tests;
