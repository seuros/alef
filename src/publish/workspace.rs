//! Workspace-member discovery — globs the root `Cargo.toml` `[workspace]`
//! `members` patterns and reads each member crate's name and version.
//!
//! Used to identify which path dependencies in a manifest refer to other
//! crates in the same workspace (so they can be rewritten to registry
//! version-dependencies during `alef publish`).

use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use toml_edit::DocumentMut;

/// Discovered workspace-member crates: their names and resolved versions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceMembers {
    /// Set of all workspace-member crate names (the `[package].name` field).
    pub names: BTreeSet<String>,
    /// Map of crate name to its resolved version. `version.workspace = true`
    /// is resolved against the root `[workspace.package].version`. Members
    /// without a discoverable version are omitted.
    pub versions: BTreeMap<String, String>,
    /// Subset of `names` whose `[package].publish` field permits publishing
    /// to a registry — absent, `true`, or an array of registry names all
    /// count as publishable. Members with `publish = false` (local-only
    /// compatibility shims kept just to satisfy a path dependency) are
    /// excluded here even though they remain in `names`.
    pub publishable: BTreeSet<String>,
}

/// Glob the root `Cargo.toml` `[workspace]` `members` patterns and collect each
/// member crate's `[package].name` and resolved `[package].version`.
///
/// `version.workspace = true` is resolved against the root
/// `[workspace.package].version`. Missing or unparseable member manifests are
/// tolerated and skipped (matching the previous inline behavior).
pub fn workspace_member_crates(workspace_root: &Path) -> Result<WorkspaceMembers> {
    let root_manifest = workspace_root.join("Cargo.toml");
    let root_content =
        std::fs::read_to_string(&root_manifest).with_context(|| format!("reading {}", root_manifest.display()))?;
    let root_doc: DocumentMut = root_content
        .parse()
        .with_context(|| format!("parsing {}", root_manifest.display()))?;

    let workspace_version = root_doc
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let mut members = WorkspaceMembers::default();

    for pattern in member_patterns(&root_doc) {
        let glob_pattern = workspace_root.join(&pattern).join("Cargo.toml");
        let glob_str = glob_pattern.to_string_lossy();
        let paths = match glob::glob(&glob_str) {
            Ok(paths) => paths,
            Err(_) => continue,
        };
        for entry in paths.flatten() {
            let Ok(content) = std::fs::read_to_string(&entry) else {
                continue;
            };
            let Ok(doc) = content.parse::<DocumentMut>() else {
                continue;
            };
            let Some(package) = doc.get("package") else {
                continue;
            };
            let Some(name) = package.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            members.names.insert(name.to_string());

            if package_is_publishable(package) {
                members.publishable.insert(name.to_string());
            }

            if let Some(version) = resolve_package_version(package, workspace_version.as_deref()) {
                members.versions.insert(name.to_string(), version);
            }
        }
    }

    Ok(members)
}

/// Collect the string patterns from the workspace `members` array.
///
/// The `exclude` array is intentionally NOT consulted: excluded crates are not
/// part of the workspace and are not published to the registry, so they must
/// not be treated as members (and therefore not as path-dep rewrite targets).
fn member_patterns(root_doc: &DocumentMut) -> Vec<String> {
    root_doc
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Resolve a member's `[package].version`, following `version.workspace = true`.
fn resolve_package_version(package: &toml_edit::Item, workspace_version: Option<&str>) -> Option<String> {
    let version = package.get("version")?;

    if let Some(s) = version.as_str() {
        return Some(s.to_string());
    }

    if let Some(tbl) = version.as_table_like() {
        let inherited = tbl
            .get("workspace")
            .and_then(|w| w.as_value())
            .and_then(|v| v.as_bool())
            == Some(true);
        if inherited {
            return workspace_version.map(str::to_string);
        }
    }

    None
}

/// Determine whether a `[package]` table's `publish` field permits
/// publishing to a registry.
///
/// - Absent -> publishable (Cargo's own default is `true`)
/// - `publish = true` -> publishable
/// - `publish = ["some-registry"]` -> publishable (restricted to specific
///   registries, but still a real, published crate — must NOT be treated
///   as a local-only shim)
/// - `publish = false` -> NOT publishable
///
/// Any other shape (which Cargo itself would reject) is treated as
/// publishable — fail open, so an unexpected manifest shape does not
/// silently swallow version-sync writes.
fn package_is_publishable(package: &toml_edit::Item) -> bool {
    match package.get("publish").and_then(|item| item.as_value()) {
        None => true,
        Some(value) => value.as_bool().unwrap_or(true),
    }
}

/// Read a Cargo.toml at `cargo_toml_path` and report whether its
/// `[package].publish` field permits publishing to a registry — see
/// [`package_is_publishable`] for the exact rules.
///
/// Manifests that are missing, unparseable, or lack a `[package]` table are
/// treated as publishable (fail open): a parse hiccup elsewhere must not
/// silently suppress a version-sync write that previously happened.
pub fn manifest_is_publishable(cargo_toml_path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(cargo_toml_path) else {
        return true;
    };
    let Ok(doc) = content.parse::<DocumentMut>() else {
        return true;
    };
    let Some(package) = doc.get("package") else {
        return true;
    };
    package_is_publishable(package)
}

/// Resolve the real crate a dependency-table entry refers to: its explicit
/// `package = "..."` alias when present, else the dependency-table key itself.
///
/// Cargo's rename mechanism means the table key is not necessarily the
/// crate's published name — `mylib_core = { path = "...", package =
/// "mylib-core" }` depends on `mylib-core` under the local alias
/// `mylib_core`. Every membership check against [`WorkspaceMembers::names`]
/// must resolve through this first, or a renamed workspace-member dependency
/// is silently treated as external and its `path = "..."` is never rewritten
/// to a registry `version = "..."` pin — a shipped manifest that cannot
/// resolve off a consumer's machine.
pub fn dependency_crate_name<'a>(key: &'a str, item: &'a toml_edit::Item) -> &'a str {
    item.as_table_like()
        .and_then(|table| table.get("package"))
        .and_then(|package| package.as_str())
        .unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Build a temp workspace: root with two members + one exclude, where one
    /// member inherits `version.workspace = true` and the other pins its own.
    fn setup_workspace(root: &Path) {
        fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
resolver = "2"
members = ["crates/my-lib", "crates/my-lib-py"]
exclude = ["crates/excluded-tool"]

[workspace.package]
version = "1.2.3"
edition = "2024"
"#,
        )
        .unwrap();

        let lib_src = root.join("crates/my-lib/src");
        fs::create_dir_all(&lib_src).unwrap();
        fs::write(lib_src.join("lib.rs"), "pub fn hello() {}").unwrap();
        fs::write(
            root.join("crates/my-lib/Cargo.toml"),
            r#"
[package]
name = "my-lib"
version.workspace = true
edition.workspace = true
"#,
        )
        .unwrap();

        let py_src = root.join("crates/my-lib-py/src");
        fs::create_dir_all(&py_src).unwrap();
        fs::write(py_src.join("lib.rs"), "pub fn hello() {}").unwrap();
        fs::write(
            root.join("crates/my-lib-py/Cargo.toml"),
            r#"
[package]
name = "my-lib-py"
version = "0.9.0"
edition = "2024"
"#,
        )
        .unwrap();

        let tool_src = root.join("crates/excluded-tool/src");
        fs::create_dir_all(&tool_src).unwrap();
        fs::write(tool_src.join("main.rs"), "fn main() {}").unwrap();
        fs::write(
            root.join("crates/excluded-tool/Cargo.toml"),
            r#"
[package]
name = "excluded-tool"
version = "2.0.0"
edition = "2024"
"#,
        )
        .unwrap();
    }

    #[test]
    fn collects_member_names_and_resolved_versions() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        setup_workspace(root);

        let members = workspace_member_crates(root).unwrap();

        let expected_names: BTreeSet<String> = ["my-lib", "my-lib-py"].iter().map(|s| s.to_string()).collect();
        assert_eq!(members.names, expected_names);

        assert_eq!(members.versions.get("my-lib").map(String::as_str), Some("1.2.3"));
        assert_eq!(members.versions.get("my-lib-py").map(String::as_str), Some("0.9.0"));
        assert!(!members.names.contains("excluded-tool"));
        assert!(!members.versions.contains_key("excluded-tool"));
    }

    #[test]
    fn tolerates_unparseable_member_manifest() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
members = ["crates/good", "crates/broken"]

[workspace.package]
version = "1.0.0"
"#,
        )
        .unwrap();

        let good_src = root.join("crates/good/src");
        fs::create_dir_all(&good_src).unwrap();
        fs::write(
            root.join("crates/good/Cargo.toml"),
            "[package]\nname = \"good\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        fs::create_dir_all(root.join("crates/broken/src")).unwrap();
        fs::write(root.join("crates/broken/Cargo.toml"), "this is not = = valid toml [[[").unwrap();

        let members = workspace_member_crates(root).unwrap();
        assert!(members.names.contains("good"));
        assert!(!members.names.contains("broken"));
        assert_eq!(members.versions.get("good").map(String::as_str), Some("1.0.0"));
    }

    /// Build a temp workspace with three members exercising each `publish`
    /// field shape: absent (default-publishable), `publish = ["registry"]`
    /// (restricted-registry, still publishable), and `publish = false` (a
    /// local-only compatibility shim that must never be published).
    fn setup_publish_field_workspace(root: &Path) {
        fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
resolver = "2"
members = ["crates/default-pub", "crates/registry-pub", "crates/private-shim"]

[workspace.package]
version = "2.0.0"
edition = "2024"
"#,
        )
        .unwrap();

        for (dir, manifest) in [
            (
                "crates/default-pub",
                "[package]\nname = \"default-pub\"\nversion = \"2.0.0\"\n",
            ),
            (
                "crates/registry-pub",
                "[package]\nname = \"registry-pub\"\nversion = \"2.0.0\"\npublish = [\"some-registry\"]\n",
            ),
            (
                "crates/private-shim",
                "[package]\nname = \"private-shim\"\nversion = \"2.0.0\"\npublish = false\n",
            ),
        ] {
            let src = root.join(dir).join("src");
            fs::create_dir_all(&src).unwrap();
            fs::write(src.join("lib.rs"), "pub fn hello() {}").unwrap();
            fs::write(root.join(dir).join("Cargo.toml"), manifest).unwrap();
        }
    }

    #[test]
    fn workspace_member_crates_reports_publishable_set_across_publish_shapes() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        setup_publish_field_workspace(root);

        let members = workspace_member_crates(root).unwrap();

        assert!(members.names.contains("default-pub"));
        assert!(members.names.contains("registry-pub"));
        assert!(members.names.contains("private-shim"));

        assert!(
            members.publishable.contains("default-pub"),
            "absent publish field must default to publishable: {:?}",
            members.publishable
        );
        assert!(
            members.publishable.contains("registry-pub"),
            "publish = [array] must be treated as publishable: {:?}",
            members.publishable
        );
        assert!(
            !members.publishable.contains("private-shim"),
            "publish = false must be excluded from publishable: {:?}",
            members.publishable
        );
    }

    #[test]
    fn manifest_is_publishable_absent_field_is_true() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("Cargo.toml");
        fs::write(&path, "[package]\nname = \"sample\"\nversion = \"1.0.0\"\n").unwrap();
        assert!(manifest_is_publishable(&path));
    }

    #[test]
    fn manifest_is_publishable_true_is_true() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("Cargo.toml");
        fs::write(
            &path,
            "[package]\nname = \"sample\"\nversion = \"1.0.0\"\npublish = true\n",
        )
        .unwrap();
        assert!(manifest_is_publishable(&path));
    }

    #[test]
    fn manifest_is_publishable_registry_array_is_true() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("Cargo.toml");
        fs::write(
            &path,
            "[package]\nname = \"sample\"\nversion = \"1.0.0\"\npublish = [\"my-registry\"]\n",
        )
        .unwrap();
        assert!(manifest_is_publishable(&path));
    }

    #[test]
    fn manifest_is_publishable_false_is_false() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("Cargo.toml");
        fs::write(
            &path,
            "[package]\nname = \"sample\"\nversion = \"1.0.0\"\npublish = false\n",
        )
        .unwrap();
        assert!(!manifest_is_publishable(&path));
    }

    #[test]
    fn manifest_is_publishable_missing_file_fails_open() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.toml");
        assert!(manifest_is_publishable(&path));
    }
}
