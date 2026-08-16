use crate::core::config::{Language, ResolvedCrateConfig};
use std::path::PathBuf;

/// Determine the output path for a language README.
pub(super) fn readme_output_path(
    config: &ResolvedCrateConfig,
    lang: Language,
    readme_cfg: &crate::core::config::ReadmeConfig,
    lang_json: &serde_json::Value,
) -> PathBuf {
    configured_output_path(readme_cfg, lang, lang_json).unwrap_or_else(|| default_readme_path(config, lang))
}

/// The output path the config asks for, if it asks for one: the language entry's `output_path`
/// (or its `output` alias), else the `crates.readme.output_pattern` formula. `None` means the
/// config expresses no opinion and the path must be derived.
///
/// This is the half of [`readme_output_path`]'s precedence that does not depend on a template
/// render, and **both** README routes must apply it. The hardcoded fallback in `fallback.rs`
/// reaches this with no template at all: `try_render_configured_readme` returns `None` for a
/// language whenever no template-based render was possible -- no `crates.readme.template_dir`
/// is set, the directory does not exist, the language has no entry, or the entry's template
/// file is missing -- and a configured output path survives every one of those, since it is not
/// a property of the template. The fallback used to derive its own path instead, writing the
/// README somewhere the consumer never asked for; that went unnoticed because the derived path
/// usually agrees with the configured one, which is coincidence, not design. ~keep
pub(super) fn configured_output_path(
    readme_cfg: &crate::core::config::ReadmeConfig,
    lang: Language,
    lang_json: &serde_json::Value,
) -> Option<PathBuf> {
    if let Some(output) = lang_json
        .get("output_path")
        .or_else(|| lang_json.get("output"))
        .and_then(|v| v.as_str())
    {
        return Some(PathBuf::from(output));
    }

    readme_cfg
        .output_pattern
        .as_ref()
        .map(|pattern| PathBuf::from(pattern.replace("{language}", lang_dir_name(lang))))
}

/// Determine the output path for a named README target.
pub(super) fn readme_target_output_path(target_name: &str, target_json: &serde_json::Value) -> anyhow::Result<PathBuf> {
    target_json
        .get("output_path")
        .or_else(|| target_json.get("output"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("README target '{target_name}' requires `output_path` or `output`"))
}

/// Workspace-relative README path for the four languages whose package is a Rust crate in the
/// workspace rather than a `packages/<lang>/` directory. `None` for every other language.
///
/// None of these directories is `crates/{alef crate name}-<lang>`. That convention holds only
/// when the alef crate name happens to equal the crate's directory stem, and in two of the three
/// consumer repos it does not: html-to-markdown's crate is named `html-to-markdown-rs` while its
/// crates are `crates/html-to-markdown{,-ffi,-node,-wasm}`, and tree-sitter-language-pack's crate
/// is named `tree-sitter-language-pack` while its crates are `crates/ts-pack-core*`. A
/// name-derived path writes the README into a directory that is not the crate at all — silently,
/// because a misplaced README is not a build error. html-to-markdown still carries a
/// `crates/html-to-markdown-rs-ffi/` directory holding nothing but such a README. See the zig
/// scaffold's identical defect and fix in alef `dc36840b2`.
///
/// Every arm delegates; no arm restates a precedence rule, because a second copy of one is
/// exactly how this module and `fallback.rs` drifted from the resolved config in the first place.
/// - FFI: `ffi_crate_relative_dir` (`[crates.output] ffi`, else `crates/{name}-ffi`).
/// - Node/Wasm: `package_dir`, which consults `[crates.node]`/`[crates.wasm] crate_dir` before
///   the `crates/{name}-<lang>` formula. Both h2m and tslp set `crate_dir`.
/// - Rust: `core_crate_dir`, the crate directory stem taken from `sources[0]`. There is no
///   `[crates.output] rust` key to consult — `OutputConfig` has no `rust` field — and this is
///   the same composition the FFI scaffold uses (`crates/{core_crate_dir}-ffi`). It assumes the
///   core crate lives under `crates/`; a core crate at `libs/foo/` would need a
///   `core_crate_relative_dir` returning the full directory, which does not exist today. That
///   assumption is no longer reachable from README output: `generate_readme` skips Rust unless
///   `[crates.readme.languages.rust]` carries an `output_path`, and both routes now return that
///   `output_path` before any derivation runs, so the Rust arm is mutually exclusive with its
///   own precondition. The `crates/` assumption still binds the FFI scaffold. ~keep
pub(super) fn crate_readme_path(config: &ResolvedCrateConfig, lang: Language) -> Option<PathBuf> {
    let crate_dir = match lang {
        Language::Ffi => config.ffi_crate_relative_dir(),
        Language::Node | Language::Wasm => config.package_dir(lang),
        Language::Rust => format!("crates/{}", config.core_crate_dir()),
        _ => return None,
    };
    Some(PathBuf::from(format!("{crate_dir}/README.md")))
}

pub(super) fn default_readme_path(config: &ResolvedCrateConfig, lang: Language) -> PathBuf {
    crate_readme_path(config, lang)
        .unwrap_or_else(|| PathBuf::from(format!("packages/{}/README.md", lang_dir_name(lang))))
}

/// Return the short directory/key name for a language. This is the canonical
/// `packages/<dir>/` directory name used when no explicit `output_path` is
/// configured. For Language::Node we return `"node"` (matching the alef-scaffold
/// directory convention); the YAML/TOML config key remains `"typescript"`
/// (see [`lang_code`]).
pub(super) fn lang_dir_name(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node => "node",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi => "ffi",
        Language::Wasm => "wasm",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin => "kotlin",
        Language::KotlinAndroid => "kotlin-android",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
        Language::C | Language::Jni => "c",
    }
}

/// Return the YAML config key for a language.
pub(super) fn lang_code(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi => "ffi",
        Language::Wasm => "wasm",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin => "kotlin",
        Language::KotlinAndroid => "kotlin_android",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
        Language::C | Language::Jni => "c",
    }
}
