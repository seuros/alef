//! Regression coverage for the `schemas/**` poly.toml exclude.
//!
//! `alef schema` writes `schemas/alef.schema.json` via `serde_json::to_string_pretty`, and
//! both `alef schema --check` and `alef verify` classify that file byte-for-byte (see
//! `core::config::schema::check_alef_config_schema`'s doc). Before this exclude,
//! `alef all`'s whole-tree `poly fmt --fix` pass (`cli::pipeline::format::converge_full_regen`)
//! had nothing telling it to skip that path, so it reformatted the schema through poly's own
//! JSON engine -- putting alef's own formatter and its own byte-exact gate in permanent
//! disagreement: `alef schema --check` passed right after `alef schema` ran, then failed
//! stale the moment `alef all` next touched the same file. These tests pin the fix at the
//! config-generation layer: the scaffolded `poly.toml` must exclude `schemas/**` from
//! `[discovery]` and every `[hooks.builtin]` formatting/lint pass, so `alef all` never hands
//! the schema to poly in the first place.

use super::*;
use crate::core::config::Language;

/// Locate the generated `poly.toml` in a scaffold result. Duplicated from `poly.rs`'s own
/// private helper of the same name rather than shared: it is five lines, and the two test
/// modules are siblings under `tests`, not visible to each other's private items.
fn poly_toml(files: &[GeneratedFile]) -> &GeneratedFile {
    files
        .iter()
        .find(|f| f.path.to_string_lossy() == "poly.toml")
        .expect("scaffold should emit a repo-root poly.toml")
}

/// `[discovery]` must exclude `schemas/**` (root-anchored: `alef schema`'s
/// `DEFAULT_SCHEMA_PATH` is a fixed top-level path, not something that recurs per-crate) so a
/// direct `poly fmt`/`poly lint` invocation, and CI's `poly fmt --check .` step, never touch
/// the vendored schema either. ~keep
#[test]
fn poly_toml_discovery_excludes_schemas_directory() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("\"/schemas/**\","),
        "[discovery] exclude must root-anchor schemas/**; got:\n{c}"
    );
}

/// The same exclude must reach every `[hooks.builtin]` pass -- `lint`, `fmt`, and
/// `file_safety` -- because those are what `poly hooks install`'s pre-commit stage actually
/// runs, independent of `[discovery]`. Missing any one of the three would leave a path by
/// which the schema still gets reformatted or relinted. ~keep
#[test]
fn poly_toml_hooks_builtins_exclude_schemas_directory() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    for builtin in ["lint = { exclude = [", "fmt = { exclude = [", "file_safety = { exclude = ["] {
        let pos = c
            .find(builtin)
            .unwrap_or_else(|| panic!("{builtin} builtin present; got:\n{c}"));
        let end = c[pos..].find(" }").map(|offset| pos + offset).unwrap_or(c.len());
        assert!(
            c[pos..end].contains("\"schemas/**\","),
            "{builtin} must exclude schemas/**; got:\n{}",
            &c[pos..end]
        );
    }
}
