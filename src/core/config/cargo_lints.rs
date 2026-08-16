//! Raw `[lints.rust]` / `[lints.clippy]` passthrough for generated binding-crate
//! `Cargo.toml` manifests.
//!
//! Generated binding crates (`*-ffi`, `*-node`, `*-py`, `*-php`, ...) are full
//! Cargo packages that only opt into the workspace's deny-by-default lint policy
//! (`print_stdout`, `dbg_macro`, `unused_must_use`, `unexpected_cfgs` check-cfg
//! allowlists, ...) by carrying their own `[lints.rust]` / `[lints.clippy]` table —
//! `[lints]\nworkspace = true` is not an option here, since it is all-or-nothing and
//! would drag in `unsafe_code = "deny"`, which an FFI crate cannot satisfy. Before
//! this module, `alef.toml` had no key that could express such a table, so
//! consumers hand-added it directly to the generated manifest; the next `alef
//! generate` silently overwrote it back out, going unnoticed because Cargo just
//! stops enforcing the denies rather than erroring.
//!
//! [`CargoLintsConfig`] gives that table a declarative home. It is a pure
//! passthrough — alef does not validate or interpret lint names, mirroring
//! [`super::manifest_extras::ManifestExtras`] for the language-native-manifest
//! equivalent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Raw `[lints.rust]` / `[lints.clippy]` tables for a generated binding-crate
/// `Cargo.toml`. Each entry's value may be a bare string (`print_stdout = "deny"`)
/// or a table (`unexpected_cfgs = { level = "warn", check-cfg = [...] }`) — alef
/// never inspects the value, only splices it in verbatim.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CargoLintsConfig {
    /// Entries for the generated crate's `[lints.rust]` table.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub rust: BTreeMap<String, toml::Value>,
    /// Entries for the generated crate's `[lints.clippy]` table.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub clippy: BTreeMap<String, toml::Value>,
}

impl CargoLintsConfig {
    /// True when neither table has any entries.
    pub fn is_empty(&self) -> bool {
        self.rust.is_empty() && self.clippy.is_empty()
    }

    /// Render `[lints.rust]` / `[lints.clippy]` tables for splicing into a generated
    /// Cargo.toml. Returns an empty string when both tables are empty, so callers can
    /// splice the result in unconditionally without ever emitting an empty `[lints]`
    /// block. The returned text carries no leading or trailing newline — callers own
    /// the blank-line glue that fits their own template.
    pub fn render(&self) -> String {
        render_lint_tables(&self.rust, &self.clippy)
    }

    /// `"key = value"` lines for every `[lints.rust]` entry except `exclude`, sorted
    /// by key. Used by backends (dart, swift, elixir) that already emit their own
    /// `unexpected_cfgs` check-cfg allowlist into `[lints.rust]` as a hand-written
    /// literal: Cargo allows only one `[lints.rust]` table per manifest, so a
    /// configured entry has to become an extra sibling line under that same
    /// hand-written header rather than open a second one. Excluding the builtin
    /// key here (rather than overwriting it after merging through `toml::Value`)
    /// keeps the builtin's own literal formatting byte-for-byte stable when no
    /// passthrough entry collides with it — this repo's Cargo.toml snapshot tests
    /// pin that exact text. A same-named user entry is dropped: the builtin encodes
    /// a compile-correctness requirement (the allowlist matching the crate's actual
    /// cfg gates) a raw passthrough config has no way to know about. ~keep
    pub fn extra_rust_lines(&self, exclude: &[&str]) -> Vec<String> {
        self.rust
            .iter()
            .filter(|(key, _)| !exclude.contains(&key.as_str()))
            .map(|(key, value)| format!("{key} = {value}"))
            .collect()
    }

    /// The `[lints.clippy]` table alone, rendered the same way [`Self::render`]
    /// would, or an empty string when `clippy` has no entries. Pairs with
    /// [`Self::extra_rust_lines`] for backends that hand-assemble `[lints.rust]`.
    pub fn clippy_block(&self) -> String {
        if self.clippy.is_empty() {
            String::new()
        } else {
            render_table("[lints.clippy]", &self.clippy)
        }
    }
}

fn render_lint_tables(rust: &BTreeMap<String, toml::Value>, clippy: &BTreeMap<String, toml::Value>) -> String {
    let mut sections = Vec::new();
    if !rust.is_empty() {
        sections.push(render_table("[lints.rust]", rust));
    }
    if !clippy.is_empty() {
        sections.push(render_table("[lints.clippy]", clippy));
    }
    sections.join("\n\n")
}

fn render_table(header: &str, table: &BTreeMap<String, toml::Value>) -> String {
    let lines: Vec<String> = table.iter().map(|(key, value)| format!("{key} = {value}")).collect();
    format!("{header}\n{}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_empty() {
        let cfg = CargoLintsConfig::default();
        assert!(cfg.is_empty());
        assert_eq!(cfg.render(), "");
    }

    #[test]
    fn deserializes_string_and_table_values() {
        let toml_src = r#"
            [rust]
            unused_must_use = "deny"

            [clippy]
            print_stdout = "deny"
            print_stderr = "deny"
            dbg_macro = "deny"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert!(!cfg.is_empty());
        assert_eq!(cfg.rust.len(), 1);
        assert_eq!(cfg.clippy.len(), 3);
        assert_eq!(cfg.rust["unused_must_use"].as_str(), Some("deny"));
    }

    #[test]
    fn render_emits_both_tables_sorted_and_no_trailing_newline() {
        let toml_src = r#"
            [rust]
            unused_must_use = "deny"

            [clippy]
            print_stdout = "deny"
            dbg_macro = "deny"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        let rendered = cfg.render();
        assert_eq!(
            rendered,
            "[lints.rust]\nunused_must_use = \"deny\"\n\n[lints.clippy]\ndbg_macro = \"deny\"\nprint_stdout = \"deny\""
        );
    }

    #[test]
    fn render_omits_absent_table() {
        let toml_src = r#"
            [clippy]
            print_stdout = "deny"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert_eq!(cfg.render(), "[lints.clippy]\nprint_stdout = \"deny\"");
    }

    #[test]
    fn render_accepts_table_valued_entries() {
        let toml_src = r#"
            [rust.unexpected_cfgs]
            level = "warn"
            check-cfg = ["cfg(docsrs)"]
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        let rendered = cfg.render();
        assert!(rendered.starts_with("[lints.rust]\nunexpected_cfgs = "));
        assert!(rendered.contains("level = \"warn\""));
        assert!(rendered.contains("check-cfg = [\"cfg(docsrs)\"]"));
        toml::from_str::<toml::Value>(&format!("[a]\n{rendered}\n")).expect("rendered entry must itself be valid TOML");
    }

    #[test]
    fn extra_rust_lines_excludes_the_builtin_key_and_sorts_the_rest() {
        let toml_src = r#"
            [rust]
            unexpected_cfgs = "warn"
            unused_must_use = "deny"
            non_snake_case = "warn"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        let lines = cfg.extra_rust_lines(&["unexpected_cfgs"]);
        assert_eq!(
            lines,
            vec![
                "non_snake_case = \"warn\"".to_string(),
                "unused_must_use = \"deny\"".to_string(),
            ],
            "the excluded key must be dropped and the rest sorted by key"
        );
    }

    #[test]
    fn extra_rust_lines_is_empty_when_only_the_excluded_key_is_set() {
        let toml_src = r#"
            [rust]
            unexpected_cfgs = "warn"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert!(cfg.extra_rust_lines(&["unexpected_cfgs"]).is_empty());
    }

    #[test]
    fn clippy_block_renders_the_clippy_table_alone() {
        let toml_src = r#"
            [rust]
            unused_must_use = "deny"

            [clippy]
            print_stdout = "deny"
        "#;
        let cfg: CargoLintsConfig = toml::from_str(toml_src).expect("deserializes");
        assert_eq!(cfg.clippy_block(), "[lints.clippy]\nprint_stdout = \"deny\"");
    }

    #[test]
    fn clippy_block_is_empty_when_clippy_table_unset() {
        let cfg = CargoLintsConfig::default();
        assert_eq!(cfg.clippy_block(), "");
    }
}
