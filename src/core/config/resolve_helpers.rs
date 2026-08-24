//! Shared helpers used during config resolution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// Express `target` as a `/`-joined relative path from `base`, where both are given
/// relative to the same root (typically the project root).
///
/// Lexical only -- no filesystem access -- so it works while scaffolding a project that
/// does not exist on disk yet. Always emits forward slashes regardless of the host OS, so
/// the result is a valid Cargo.toml `path = "..."` value on every platform `alef` runs on.
/// An empty `target` (the project root itself) collapses trailing `..` segments correctly,
/// e.g. `base = "crates/foo-ffi"`, `target = ""` yields `"../.."`, not `"../../"`. ~keep
pub(crate) fn relative_slash_path(base: &Path, target: &Path) -> String {
    let base_components: Vec<_> = base.components().collect();
    let target_components: Vec<_> = target.components().collect();
    let common = base_components
        .iter()
        .zip(target_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut parts: Vec<String> = base_components[common..].iter().map(|_| "..".to_string()).collect();
    parts.extend(
        target_components[common..]
            .iter()
            .map(|component| component.as_os_str().to_string_lossy().into_owned()),
    );

    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

/// Strip a trailing path `suffix` from `path`, matching components tail-to-head.
///
/// Returns `None` when `path` has fewer components than `suffix`, or the trailing components
/// do not match, so callers can fall back to treating `path` as already being the shorter
/// shape. `split == 0` (the suffix consumes the whole path) resolves to `"."`, the project
/// root, rather than an empty (and therefore invalid) `PathBuf`.
pub(crate) fn strip_trailing_components(path: &Path, suffix: &Path) -> Option<PathBuf> {
    let path_components: Vec<_> = path.components().collect();
    let suffix_components: Vec<_> = suffix.components().collect();
    if path_components.len() < suffix_components.len() {
        return None;
    }
    let split = path_components.len() - suffix_components.len();
    let tail_matches = path_components[split..]
        .iter()
        .zip(suffix_components.iter())
        .all(|(a, b)| a == b);
    if !tail_matches {
        return None;
    }
    if split == 0 {
        return Some(PathBuf::from("."));
    }
    Some(path_components[..split].iter().collect())
}

/// The Maven project root implied by a resolved `java` binding output path.
///
/// `[crates.output].java` and the unconfigured `packages/java` default may name the project
/// root itself, or the Maven standard `src/main/java/<dotted_package_as_path>/` source
/// directory inside it -- the same root-vs-source-dir ambiguity
/// [`crate::backends::kotlin_android`]'s `ProjectLayout` resolves for `kotlin_android`.
/// `JavaBackend`'s own file placement (`backends::java::gen_bindings`) disambiguates by
/// checking whether the configured path already ends with the package path; `package_dir`
/// must derive the *same* root from the *same* check, or `mvn -f {root}/pom.xml` and the
/// scaffolded `lint`/`test`/`clean`/`setup` commands target a directory java never wrote its
/// sources into. ~keep
pub(crate) fn java_project_root(output_dir: &str, package_path: &str) -> PathBuf {
    let source_dir = if output_dir.ends_with(package_path) || output_dir.ends_with(&format!("{package_path}/")) {
        PathBuf::from(output_dir)
    } else {
        PathBuf::from(output_dir).join(package_path)
    };
    let without_package = strip_trailing_components(&source_dir, Path::new(package_path)).unwrap_or(source_dir);
    strip_trailing_components(&without_package, Path::new("src/main/java")).unwrap_or(without_package)
}

/// The Gradle project root implied by a resolved `kotlin` (JVM, non-Android) binding output
/// path.
///
/// Mirrors [`java_project_root`], but the plain Kotlin backend's own generator picks its
/// unconfigured-vs-configured branch by *presence* (`explicit_output.kotlin.is_some()`, see
/// `backends::kotlin::gen_bindings`) rather than by path shape: an unconfigured output path is
/// always the project root with sources placed under `src/main/kotlin/<pkg>/`, while a
/// configured path is written to directly, with no source-set suffix appended. `package_dir`
/// follows the same presence check instead of re-guessing the branch from the path, and falls
/// back to the configured path unchanged when it does not carry the source-set suffix. ~keep
pub(crate) fn kotlin_project_root(output_dir: &str, package_path: &str, is_explicitly_configured: bool) -> PathBuf {
    let output_path = PathBuf::from(output_dir);
    if !is_explicitly_configured {
        return output_path;
    }
    let full_suffix = Path::new("src/main/kotlin").join(package_path);
    strip_trailing_components(&output_path, &full_suffix).unwrap_or(output_path)
}

#[cfg(test)]
mod tests;
