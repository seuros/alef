//! Shared helpers used during config resolution.

use std::collections::HashMap;
use std::path::PathBuf;

use super::extras::Language;
use super::output::{OutputConfig, OutputTemplate};
use super::raw_crate::RawCrateConfig;

/// Compute resolved output paths for a crate: per-crate explicit wins; else use template.
pub(crate) fn resolve_output_paths(
    krate: &RawCrateConfig,
    template: &OutputTemplate,
    languages: &[Language],
    multi_crate: bool,
) -> HashMap<String, PathBuf> {
    let mut paths = HashMap::new();
    for lang in languages {
        let lang_str = lang.to_string();
        let explicit = per_crate_explicit_output(&krate.output, lang);
        let path = explicit
            .map(PathBuf::from)
            .unwrap_or_else(|| template.resolve(&krate.name, &lang_str, multi_crate));
        paths.insert(lang_str, path);
    }
    paths
}

/// Extract an explicit per-crate output path for a language from [`OutputConfig`].
pub(crate) fn per_crate_explicit_output(output: &OutputConfig, lang: &Language) -> Option<String> {
    let path = match lang {
        Language::Python => output.python.as_ref(),
        Language::Node => output.node.as_ref(),
        Language::Ruby => output.ruby.as_ref(),
        Language::Php => output.php.as_ref(),
        Language::Elixir => output.elixir.as_ref(),
        Language::Wasm => output.wasm.as_ref(),
        Language::Ffi => output.ffi.as_ref(),
        Language::Go => output.go.as_ref(),
        Language::Java => output.java.as_ref(),
        Language::Kotlin => output.kotlin.as_ref(),
        Language::KotlinAndroid => output.kotlin_android.as_ref(),
        Language::Dart => output.dart.as_ref(),
        Language::Swift => output.swift.as_ref(),
        Language::Gleam => output.gleam.as_ref(),
        Language::Csharp => output.csharp.as_ref(),
        Language::R => output.r.as_ref(),
        Language::Zig => output.zig.as_ref(),
        Language::Rust | Language::C | Language::Jni => None,
    };
    path.map(|p| p.to_string_lossy().into_owned())
}

/// Merge two HashMaps: per-crate values win; workspace values fill in missing keys.
pub(crate) fn merge_map<V: Clone>(
    workspace: &HashMap<String, V>,
    per_crate: &HashMap<String, V>,
) -> HashMap<String, V> {
    let mut merged = workspace.clone();
    for (k, v) in per_crate {
        merged.insert(k.clone(), v.clone());
    }
    merged
}

/// Helper function to resolve output directory path from config.
/// Replaces {name} placeholder with the crate name.
pub fn resolve_output_dir(config_path: Option<&PathBuf>, crate_name: &str, default: &str) -> String {
    config_path
        .map(|p| p.to_string_lossy().replace("{name}", crate_name))
        .unwrap_or_else(|| default.replace("{name}", crate_name))
}

/// The `crates/{crate}-<suffix>` root every scaffolder already writes a manifest for, and
/// the same root the PyO3/NAPI/PHP/FFI/wasm-bindgen emitters fall back to when no output
/// path is configured. Returns `None` for languages with no dedicated binding crate of
/// their own, whose default binding surface lives under `packages/` instead.
///
/// `OutputTemplate::resolve` (what `alef generate` writes sources under) and
/// `ResolvedCrateConfig::package_dir`'s no-override formula (what the scaffolder and build
/// commands assume) both call this, so the same crate cannot get two different default
/// homes from two different code paths — which is exactly how the scaffolder's
/// `crates/{crate}-ffi` manifest and `generate`'s `packages/ffi` sources ended up
/// disagreeing on a stock scaffold. ~keep
pub(crate) fn default_binding_crate_root(crate_name: &str, lang: &str) -> Option<String> {
    let suffix = match lang {
        "python" => "py",
        "node" => "node",
        "php" => "php",
        "ffi" => "ffi",
        "wasm" => "wasm",
        _ => return None,
    };
    Some(format!("crates/{crate_name}-{suffix}"))
}

/// The crate root and generated-source directory implied by one binding output path.
///
/// `[crates.output]` entries and [`OutputTemplate`](super::output::OutputTemplate) defaults
/// disagree about shape: consumer configs spell out a `src`-suffixed path
/// (`crates/foo-wasm/src/`) naming where generated *sources* land, while the template default
/// is crate-root-shaped (`packages/wasm`) and names the *package* directory. A backend that
/// recovers the crate root from either shape with a bare `Path::parent` is right only for the
/// first: given the second it writes the crate's own manifest and build script into the parent
/// of the package directory, and every sibling package's cargo invocation then walks up into
/// that stray manifest. Derive the root from the shape instead of assuming one. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputLayout {
    /// The binding crate's root: where `Cargo.toml`, `build.rs`, and sibling
    /// directories such as `stubs/` belong.
    pub root: PathBuf,
    /// Where generated Rust sources belong. Always `root`, `root/src`, or the
    /// configured `src`-suffixed path itself.
    pub src: PathBuf,
}

impl OutputLayout {
    /// Split an already-resolved output directory into its crate root and source directory.
    ///
    /// A path whose final component is `src` is a source directory, so the root is its
    /// parent. Anything else is the crate root itself, so sources go under `<root>/src`.
    #[must_use]
    pub fn from_output_dir(output_dir: &str) -> Self {
        let path = PathBuf::from(output_dir);
        if path.file_name().and_then(|name| name.to_str()) == Some("src") {
            // An empty parent means the configured path was a bare `src`, so the crate root
            // is the project root; joining onto it yields `Cargo.toml`, which is what the
            // previous `unwrap_or_else` fallback produced for that case. ~keep
            let root = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
            Self { root, src: path }
        } else {
            let src = path.join("src");
            Self { root: path, src }
        }
    }
}

/// Resolve a configured output path and split it into its crate root and source directory.
///
/// Takes the same arguments as [`resolve_output_dir`], whose `{name}` substitution and
/// default handling it reuses.
#[must_use]
pub fn resolve_output_layout(config_path: Option<&PathBuf>, crate_name: &str, default: &str) -> OutputLayout {
    OutputLayout::from_output_dir(&resolve_output_dir(config_path, crate_name, default))
}

/// Detect whether `serde` and `serde_json` are available in a binding crate's Cargo.toml.
///
/// `output_dir` is the generated source directory (e.g., `crates/sample_project-py/src/`).
/// The function walks up to find the crate's Cargo.toml and checks its `[dependencies]`
/// for both `serde` and `serde_json`.
pub fn detect_serde_available(output_dir: &str) -> bool {
    let src_path = std::path::Path::new(output_dir);
    let mut dir = src_path;
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            return cargo_toml_has_serde(&cargo_toml);
        }
        match dir.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => dir = parent,
            _ => break,
        }
    }
    false
}

/// Check if a Cargo.toml has both `serde` (with derive feature) and `serde_json` in its dependencies.
///
/// The `serde::Serialize` derive macro requires `serde` as a direct dependency with the `derive`
/// feature enabled. Having only `serde_json` is not sufficient since it only pulls in `serde`
/// transitively without the derive proc-macro.
fn cargo_toml_has_serde(path: &std::path::Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let has_serde_json = content.contains("serde_json");
    let has_serde_dep = content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("serde ")
            || trimmed.starts_with("serde=")
            || trimmed.starts_with("serde.")
            || trimmed == "[dependencies.serde]"
    });

    has_serde_json && has_serde_dep
}

/// Find the path segment that comes after a `crates/` component.
///
/// Handles both absolute paths (e.g., `/workspace/repo/crates/foo/src/lib.rs`)
/// and relative paths (e.g., `crates/foo/src/lib.rs`).  Returns the slice
/// starting immediately after the `crates/` prefix, or `None` if the path
/// does not contain such a component.
pub(crate) fn find_after_crates_prefix(path: &str) -> Option<&str> {
    if let Some(pos) = path.find("/crates/") {
        return Some(&path[pos + "/crates/".len()..]);
    }
    if let Some(stripped) = path.strip_prefix("crates/") {
        return Some(stripped);
    }
    None
}

#[cfg(test)]
mod tests;
