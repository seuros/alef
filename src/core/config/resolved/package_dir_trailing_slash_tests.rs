//! Regression coverage for alef task #477: `ResolvedCrateConfig::package_dir` must strip a
//! trailing slash from a user-configured `[crates.output]` path. Most downstream call sites
//! build child paths with `format!("{pkg_dir}/child")`, not the trailing-slash-safe
//! `Path::join`, so an un-normalised `pkg_dir` produces a double-slash path
//! (`packages/csharp/src//LICENSE`) that `alef adopt` can never match against the real
//! on-disk file, permanently stranding it as unadoptable.
//!
//! Split into its own file rather than grown inline in `lookups.rs`: that file is already at
//! the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md). ~keep

use crate::core::config::extras::Language;
use crate::core::config::new_config::NewAlefConfig;

fn resolved_one(toml: &str) -> super::ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn package_dir_strips_trailing_slash_from_configured_output_path() {
    let with_trailing_slash = resolved_one(
        r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.output]
csharp = "packages/csharp/src/"
"#,
    );
    let without_trailing_slash = resolved_one(
        r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.output]
csharp = "packages/csharp/src"
"#,
    );
    assert_eq!(
        with_trailing_slash.package_dir(Language::Csharp),
        without_trailing_slash.package_dir(Language::Csharp),
        "a trailing slash on a configured output path must not change package_dir's result"
    );
    assert_eq!(
        with_trailing_slash.package_dir(Language::Csharp),
        "packages/csharp/src",
        "the trailing slash must be stripped"
    );
}

/// Negative control for the test above: a path with no trailing slash must round-trip
/// unchanged, proving the normalisation isn't silently mangling ordinary input.
#[test]
fn package_dir_leaves_a_normal_configured_path_unchanged() {
    let r = resolved_one(
        r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.output]
csharp = "packages/csharp/src"
"#,
    );
    assert_eq!(r.package_dir(Language::Csharp), "packages/csharp/src");
}
