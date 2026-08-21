//! In-place repair for a `poly.toml` table [`crate::scaffold::scaffold_poly_config`] stopped
//! emitting but that `merge_managed_toml`'s managed-merge never reaches on an already-scaffolded
//! consumer. See [`migrate_poly_toml_drop_snippet_hook`]'s doc for the full defect.

use anyhow::Context as _;
use std::path::Path;

/// Path of the repo-root poly config this migration repairs, relative to the repo root.
const POLY_CONFIG_RELATIVE: &str = "poly.toml";

/// The exact `run` command `workspace_hook`'s retracted snippet-check call site (`snippet_check_hook`,
/// dropped in `a139a680`, "drops the alef-snippets pre-commit hook from generated poly.toml") last
/// emitted -- the one fact this migration keys off, alongside `workspace = true`, so a consumer's own
/// unrelated `[hooks.pre-commit.commands.alef-snippets]` entry running a different command is never a
/// match. ~keep
const STALE_SNIPPET_HOOK_RUN: &str = "alef snippets check --strict --cache off";

/// Remove a pre-existing `[hooks.pre-commit.commands.alef-snippets]` table from `poly.toml` --
/// the exact hook retracted from generation in `a139a680` but never reachable on an
/// already-scaffolded consumer.
///
/// `merge_managed_toml_core`'s prune pass (see its doc) tracks and removes only ARRAY values,
/// via `.alef/toml-merge-provenance.json` -- never a whole TABLE alef stops emitting. The union
/// pass that follows only ever ADDS tables present in `generated` and not yet in `existing`; it
/// has no counterpart that removes one present in `existing` but absent from `generated`. A
/// consumer scaffolded while `workspace_hook` still emitted this table therefore keeps
/// re-merging it forever: every regenerate leaves it untouched (it is already present, so the
/// union pass changes nothing), and nothing in the merge ever proposes removing it. Every commit
/// this hook runs on shells out to an `alef` binary the consumer's lint job never installs,
/// failing `poly lint`/pre-commit with `alef-snippets: 1: alef: not found`.
///
/// Guarded on the table's own `run` command matching [`STALE_SNIPPET_HOOK_RUN`] -- the one
/// string alef itself ever emitted here -- AND `workspace = true`, the only mode `workspace_hook`
/// ever set for it. Matching on the table's name alone would risk removing a consumer's own,
/// differently-configured `alef-snippets` command; this guard leaves that untouched. Silent
/// (returns `Ok(false)`) on a missing `poly.toml`, unparsable TOML, a `poly.toml` with no such
/// table, or a table that no longer matches (idempotent: nothing left to remove on a second
/// pass). ~keep
pub(crate) fn migrate_poly_toml_drop_snippet_hook(base_dir: &Path) -> anyhow::Result<bool> {
    let path = base_dir.join(POLY_CONFIG_RELATIVE);
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Ok(mut doc) = existing.parse::<toml_edit::DocumentMut>() else {
        return Ok(false);
    };

    let commands = doc
        .as_table_mut()
        .get_mut("hooks")
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|hooks| hooks.get_mut("pre-commit"))
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|pre_commit| pre_commit.get_mut("commands"))
        .and_then(toml_edit::Item::as_table_mut);
    let Some(commands) = commands else {
        return Ok(false);
    };

    let is_stale_snippet_hook = commands
        .get("alef-snippets")
        .and_then(toml_edit::Item::as_table)
        .is_some_and(|hook| {
            hook.get("run").and_then(toml_edit::Item::as_str) == Some(STALE_SNIPPET_HOOK_RUN)
                && hook.get("workspace").and_then(toml_edit::Item::as_bool) == Some(true)
        });
    if !is_stale_snippet_hook {
        return Ok(false);
    }

    commands.remove("alef-snippets");

    let parent = path.parent().context("poly.toml path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, doc.to_string().as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing poly.toml: removed the retracted alef-snippets pre-commit hook"
    );
    Ok(true)
}
