//! Format-preserving patch of `[workspace.lints.rust]` in the root `Cargo.toml`.
//!
//! Called during `alef scaffold` to add the check-cfg allowlist for alef's two
//! source-level marker conventions, so that downstream crates can write either
//! without declaring a real Cargo feature or cfg:
//! * `#[cfg_attr(alef, alef(skip))]` — excludes an item from every binding surface
//!   (the far more common of the two; see `core::ir::items` and the extractor's
//!   exclusion handling).
//! * `#[cfg_attr(feature = "alef-meta", alef(since = "..."))]` — the schema-since
//!   annotation.
//!
//! Neither `alef` nor `alef-meta` is ever actually enabled at real compile time —
//! there is no `alef` proc-macro crate a downstream `Cargo.toml` depends on, so the
//! attribute inside each `cfg_attr` never actually runs. Declaring `alef-meta` as a
//! real Cargo feature instead would cause `cargo clippy --all-features` to activate
//! it and fail with a hard compile error trying to invoke that nonexistent macro. The
//! two allowlist entries below tell rustc 1.80+ that both names are known cfg
//! names/values, silencing `unexpected_cfg` under `-D warnings` without ever making
//! either one a real feature.

use anyhow::Context as _;
use std::path::Path;

/// Check-cfg allowlist entries for both marker conventions (see the module doc).
/// Order is the order they render inside `check-cfg = [...]`.
const CHECK_CFG_VALUES: [&str; 2] = ["cfg(alef)", r#"cfg(feature, values("alef-meta"))"#];

/// Patch `[workspace.lints.rust]` in the root `Cargo.toml` to include
/// `unexpected_cfgs = { level = "warn", check-cfg = ['cfg(alef)', 'cfg(feature, values("alef-meta"))'] }`.
///
/// Reads from and writes to `./Cargo.toml` (the current working directory).
pub fn ensure_workspace_alef_meta_check_cfg() -> anyhow::Result<bool> {
    ensure_workspace_alef_meta_check_cfg_at(Path::new("Cargo.toml"))
}

/// Inner implementation that accepts an explicit path — used by tests to avoid
/// process-global `set_current_dir` races.
///
/// - Returns `true` when the file was modified.
/// - Returns `false` (without error) when:
///   - `cargo_toml` does not exist or cannot be read.
///   - The manifest has no `[workspace]` table (single-crate, not a workspace).
///   - `unexpected_cfgs` is already present in `[workspace.lints.rust]` (idempotent).
/// - Propagates errors only for parse or write failures.
fn ensure_workspace_alef_meta_check_cfg_at(cargo_toml: &Path) -> anyhow::Result<bool> {
    use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

    let content = match std::fs::read_to_string(cargo_toml) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    if content.contains("unexpected_cfgs") {
        return Ok(false);
    }

    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse {}", cargo_toml.display()))?;

    if !doc.contains_key("workspace") {
        return Ok(false);
    }

    let workspace_item = doc
        .get_mut("workspace")
        .context("[workspace] entry missing after containment check")?;
    let workspace_table = match workspace_item.as_table_mut() {
        Some(t) => t,
        None => return Ok(false),
    };

    let lints_item = workspace_table
        .entry("lints")
        .or_insert_with(|| Item::Table(Table::new()));
    let lints_table = match lints_item.as_table_mut() {
        Some(t) => t,
        None => return Ok(false),
    };

    let rust_item = lints_table.entry("rust").or_insert_with(|| Item::Table(Table::new()));
    let rust_table = match rust_item.as_table_mut() {
        Some(t) => t,
        None => return Ok(false),
    };

    if rust_table.contains_key("unexpected_cfgs") {
        return Ok(false);
    }

    // Build: unexpected_cfgs = { level = "warn", check-cfg = ['cfg(alef)', 'cfg(feature, values("alef-meta"))'] }
    let mut check_cfg_array = Array::new();
    for value in CHECK_CFG_VALUES {
        check_cfg_array.push(value);
    }

    let mut inline = InlineTable::new();
    inline.insert("level", Value::from("warn"));
    inline.insert("check-cfg", Value::Array(check_cfg_array));

    rust_table.insert("unexpected_cfgs", Item::Value(Value::InlineTable(inline)));

    std::fs::write(cargo_toml, doc.to_string()).with_context(|| format!("failed to write {}", cargo_toml.display()))?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{CHECK_CFG_VALUES, ensure_workspace_alef_meta_check_cfg_at};
    use std::fs;
    use tempfile::TempDir;

    fn run(dir: &TempDir, content: &str) -> anyhow::Result<bool> {
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, content).unwrap();
        ensure_workspace_alef_meta_check_cfg_at(&path)
    }

    fn read(dir: &TempDir) -> String {
        fs::read_to_string(dir.path().join("Cargo.toml")).unwrap()
    }

    #[test]
    fn skips_when_no_cargo_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Cargo.toml");
        assert!(!ensure_workspace_alef_meta_check_cfg_at(&path).unwrap());
    }

    #[test]
    fn skips_single_crate_manifest() {
        let dir = TempDir::new().unwrap();
        let modified = run(&dir, "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n").unwrap();
        assert!(!modified, "single-crate manifest must not be modified");
    }

    #[test]
    fn patches_workspace_manifest_without_lints() {
        let dir = TempDir::new().unwrap();
        let modified = run(&dir, "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        assert!(modified, "should patch manifest that has no lints section");
        let written = read(&dir);
        for value in CHECK_CFG_VALUES {
            assert!(
                written.contains(value),
                "must contain check-cfg value {value}:\n{written}"
            );
        }
        assert!(
            written.contains("unexpected_cfgs"),
            "must contain unexpected_cfgs key:\n{written}"
        );
    }

    #[test]
    fn idempotent_when_check_cfg_already_present() {
        let dir = TempDir::new().unwrap();
        let entries = CHECK_CFG_VALUES.map(|value| format!("'{value}'")).to_vec().join(", ");
        let content = format!(
            "[workspace]\nmembers = []\n\n[workspace.lints.rust]\nunexpected_cfgs = {{ level = \"warn\", check-cfg = [{entries}] }}\n"
        );
        let modified = run(&dir, &content).unwrap();
        assert!(!modified, "must not modify file that already has the check-cfg");
    }

    #[test]
    fn skips_when_unexpected_cfgs_key_exists_with_different_value() {
        let dir = TempDir::new().unwrap();
        let content = "[workspace]\nmembers = []\n\n[workspace.lints.rust]\nunexpected_cfgs = { level = \"deny\", check-cfg = ['cfg(frb_expand)'] }\n";
        let modified = run(&dir, content).unwrap();
        assert!(!modified, "must not touch existing unexpected_cfgs entry");
        assert!(read(&dir).contains("deny"), "existing entry must be preserved");
    }

    #[test]
    fn skips_gracefully_when_lints_is_inline_table() {
        let dir = TempDir::new().unwrap();
        let content = "[workspace]\nmembers = []\nlints = { rust = { unsafe_code = \"forbid\" } }\n";
        let modified = run(&dir, content).unwrap();
        assert!(!modified, "must skip gracefully when lints is inline table");
        assert_eq!(read(&dir), content, "file must not be modified");
    }

    #[test]
    fn patches_workspace_with_existing_lints_rust_without_unexpected_cfgs() {
        let dir = TempDir::new().unwrap();
        let modified = run(
            &dir,
            "[workspace]\nmembers = []\n\n[workspace.lints.rust]\nunsafe_code = \"forbid\"\n",
        )
        .unwrap();
        assert!(modified, "should add unexpected_cfgs alongside existing lint entry");
        let written = read(&dir);
        for value in CHECK_CFG_VALUES {
            assert!(
                written.contains(value),
                "must contain check-cfg value {value}:\n{written}"
            );
        }
        assert!(
            written.contains("unsafe_code"),
            "existing lint must be preserved:\n{written}"
        );
    }
}
