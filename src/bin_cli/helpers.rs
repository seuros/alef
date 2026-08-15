use anyhow::{Context, Result};

pub(crate) fn run_required_post_builds(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Result<()> {
    for &language in languages {
        let Some(backend) = crate::cli::registry::try_get_backend(language) else {
            continue;
        };
        let Some(build_config) = backend.build_config_with_config(config) else {
            continue;
        };
        if build_config.post_build.is_empty() {
            continue;
        }
        tracing::info!("  [{language}] running post-build...");
        crate::cli::pipeline::run_post_build(language, &build_config, config, base_dir)
            .with_context(|| format!("failed to run required post-build steps for {language}"))?;
        tracing::info!("  [{language}] post-build processing complete");
    }
    Ok(())
}

/// Returns true when every freshly generated file already matches the file on disk,
/// using the same hash-line-insensitive body comparison as [`crate::cli::pipeline::write_files`].
///
/// The per-run side cache (`.alef/hashes/*.output_hashes`) records what was last
/// generated, but the files on disk can drift from it out-of-band — a `git restore`,
/// a hand-edit, a partial write, or an interrupted run. Treating the cache as the
/// sole authority for an "up to date" skip silently retains that stale output: the
/// generator would emit different bytes, yet the skip fires and `write_files` is
/// never reached. Gating the skip on actual disk agreement closes that gap while
/// staying a no-op for the common clean case.
pub(crate) fn generated_files_match_disk(
    lang_files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
) -> bool {
    lang_files.iter().all(|file| {
        let normalized = crate::cli::pipeline::normalize_content(&file.path, &file.content);
        match std::fs::read_to_string(base_dir.join(&file.path)) {
            Ok(disk) => crate::core::hash::strip_hash_line(&disk) == crate::core::hash::strip_hash_line(&normalized),
            Err(_) => false,
        }
    })
}

/// Map the CLI verbosity flags to a default `tracing` level filter.
///
/// A single verbosity channel drives the whole binary: `--quiet` pins the level to
/// `error`; otherwise the `-v` count raises it — no flag is `info` (progress visible),
/// `-v` is `debug` (per-file/per-item detail), `-vv`+ is `trace`. `RUST_LOG` overrides
/// this default when set (see [`init_tracing`]).
pub(crate) fn default_log_level(verbose: u8, quiet: bool) -> &'static str {
    if quiet {
        return "error";
    }
    match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    }
}

pub(crate) fn init_tracing(verbose: u8, quiet: bool, no_color: bool) {
    use tracing_subscriber::EnvFilter;
    let default_level = default_log_level(verbose, quiet);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(!no_color)
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(false)
        .init();
}

/// Load and resolve an alef.toml, returning the workspace-level config and
/// the per-crate resolved configs.  Detects legacy schema and returns an error
/// with a migration hint rather than a confusing parse error.
pub(crate) fn load_config(
    path: &std::path::Path,
) -> Result<(
    crate::core::config::WorkspaceConfig,
    Vec<crate::core::config::ResolvedCrateConfig>,
)> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read config: {}", path.display()))?;
    crate::core::config::detect_legacy_keys(&content).with_context(|| {
        format!(
            "legacy schema detected in {} — run `alef migrate` to update automatically",
            path.display()
        )
    })?;
    let mut toml_value: toml::Value =
        toml::from_str(&content).with_context(|| format!("Failed to parse alef.toml ({})", path.display()))?;
    let deprecation_warnings = crate::core::config::legacy::strip_deprecated_keys(&mut toml_value);
    for warning in &deprecation_warnings {
        tracing::warn!("{}", warning);
    }
    let cfg: crate::core::config::NewAlefConfig = toml_value
        .try_into()
        .with_context(|| format!("Failed to deserialize alef.toml ({})", path.display()))?;
    let resolved = cfg
        .resolve()
        .with_context(|| format!("failed to resolve crates in {}", path.display()))?;
    for resolved_cfg in &resolved {
        crate::core::config::validation::validate_resolved(resolved_cfg)
            .with_context(|| format!("invalid resolved config for crate `{}`", resolved_cfg.name))?;
    }
    Ok((cfg.workspace, resolved))
}

pub(crate) fn resolve_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<crate::core::config::Language>> {
    resolve_languages_inner(config, filter, false)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
/// Docs can always be generated for Rust since it's the source language.
pub(crate) fn resolve_doc_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<crate::core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
///
/// Every Rust crate that publishes to crates.io needs a `crates/<lib>/README.md`,
/// so the readme command must regenerate it from the same templates that produce
/// the per-binding READMEs. Configure with `[crates.readme.languages.rust]` in
/// `alef.toml` to opt in.
pub(crate) fn resolve_readme_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<crate::core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

/// Resolve languages for `alef test`.
///
/// Test suites can exist for targets that do not generate host bindings, such
/// as Rust e2e tests for the source crate. Keep binding language resolution
/// strict for generation/build commands, but allow explicit test targets and
/// include e2e-only entries when `alef test --e2e` runs without a filter.
pub(crate) fn resolve_test_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
    include_e2e: bool,
) -> Result<Vec<crate::core::config::Language>> {
    match filter {
        Some(langs) => {
            let mut result = vec![];
            for lang_str in langs {
                let lang = parse_language(lang_str)?;
                if config.languages.contains(&lang) || config.test.contains_key(&lang.to_string()) {
                    result.push(lang);
                } else {
                    anyhow::bail!("Language '{lang_str}' not in config languages list or test configuration");
                }
            }
            Ok(result)
        }
        None => {
            let mut langs = config.languages.clone();
            if include_e2e {
                let mut extra_test_langs = vec![];
                for (lang_str, test_config) in &config.test {
                    if test_config.e2e.is_none() {
                        continue;
                    }
                    let lang = parse_language(lang_str)
                        .with_context(|| format!("Invalid test language in alef.toml: {lang_str}"))?;
                    if !langs.contains(&lang) {
                        extra_test_langs.push(lang);
                    }
                }
                extra_test_langs.sort_by_key(|lang| lang.to_string());
                for lang in extra_test_langs {
                    if !langs.contains(&lang) {
                        langs.push(lang);
                    }
                }
            }
            Ok(langs)
        }
    }
}

pub(crate) fn resolve_languages_inner(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
    allow_rust: bool,
) -> Result<Vec<crate::core::config::Language>> {
    match filter {
        Some(langs) => {
            let mut result = vec![];
            for lang_str in langs {
                let lang = parse_language(lang_str)?;
                if config.languages.contains(&lang) || (allow_rust && lang == crate::core::config::Language::Rust) {
                    result.push(lang);
                } else {
                    anyhow::bail!("Language '{lang_str}' not in config languages list");
                }
            }
            Ok(result)
        }
        None => {
            let mut langs = config.languages.clone();
            if allow_rust && !langs.contains(&crate::core::config::Language::Rust) {
                langs.push(crate::core::config::Language::Rust);
            }
            Ok(langs)
        }
    }
}

pub(crate) fn parse_language(lang_str: &str) -> Result<crate::core::config::Language> {
    toml::Value::String(lang_str.to_string())
        .try_into()
        .with_context(|| format!("Unknown language: {lang_str}"))
}

pub(crate) fn format_languages(languages: &[crate::core::config::Language]) -> String {
    languages.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(", ")
}

/// A file whose embedded `alef:hash:` did not match any expected inputs hash.
///
/// Returned by [`verify_walk_multi`] and [`verify_walk`] for every stale file
/// found during a verify walk. The `computed` field holds all candidate hashes
/// from the current workspace (one per crate); `embedded` is what was found in
/// the file's header. Pass `--verbose` / `-v` to `alef verify` to print these.
pub(crate) struct StaleMismatch {
    /// Absolute path of the stale generated file.
    pub(crate) path: String,
    /// The `alef:hash:` value embedded in the file's header.
    pub(crate) embedded: String,
    /// All candidate inputs hashes computed from the current workspace.
    /// The file is stale because none of these equals `embedded`.
    pub(crate) computed: Vec<String>,
}

/// Build/cache directories the verify walk never descends into.
const VERIFY_SKIP_DIRS: &[&str] = &[
    ".git",
    ".alef",
    "target",
    "node_modules",
    "_build",
    "deps",
    "parsers",
    "dist",
    "dist-node",
    "vendor",
    ".venv",
    ".cache",
    ".remote-cache",
    "__pycache__",
    "build",
    "tmp",
    "out",
    ".idea",
    ".vscode",
];

/// File extensions the verify walk inspects for an `alef:hash:` header.
const VERIFY_SCAN_EXTENSIONS: &[&str] = &[
    "rs", "py", "pyi", "ts", "tsx", "js", "mjs", "cjs", "rb", "rbs", "php", "phpstub", "go", "java", "cs", "ex", "exs",
    "R", "r", "toml", "json", "md", "h", "c", "yaml", "yml",
];

/// Walk `base_dir` and return every alef-owned file paired with its optional
/// `alef:hash:<hex>` stamp. Skips build/cache directories and files without the
/// Alef ownership marker. Shared by [`verify_walk`] and [`verify_walk_multi`]
/// so both see the same file set.
fn collect_alef_hashes(base_dir: &std::path::Path) -> Vec<(std::path::PathBuf, Option<String>, String)> {
    let mut found = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![base_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if VERIFY_SKIP_DIRS.contains(&name) || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| {
                    VERIFY_SCAN_EXTENSIONS
                        .iter()
                        .any(|allowed| allowed.eq_ignore_ascii_case(e))
                })
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if crate::core::hash::content_has_alef_marker(&content) {
                found.push((path, crate::core::hash::extract_hash(&content), content));
            }
        }
    }
    found
}

/// Multi-crate variant of [`verify_walk`].
///
/// Walk the repo from `base_dir`, find every alef-headered file, and return the
/// list of stale ones. In a multi-crate workspace each file passes when its
/// content-derived hash matches one crate's generation-input hash.
pub(crate) fn verify_walk_multi(
    base_dir: &std::path::Path,
    inputs_hashes: &[String],
) -> anyhow::Result<Vec<StaleMismatch>> {
    if inputs_hashes.is_empty() {
        return Ok(Vec::new());
    }
    if inputs_hashes.len() == 1 {
        return verify_walk(base_dir, &inputs_hashes[0]);
    }

    let mut stale: Vec<StaleMismatch> = collect_alef_hashes(base_dir)
        .into_iter()
        .filter(|(_, disk_hash, content)| {
            disk_hash.as_ref().is_none_or(|disk_hash| {
                !inputs_hashes
                    .iter()
                    .any(|inputs_hash| crate::core::hash::compute_file_hash(inputs_hash, content) == *disk_hash)
            })
        })
        .map(|(path, disk_hash, content)| StaleMismatch {
            path: path.display().to_string(),
            embedded: disk_hash.unwrap_or_else(|| "<missing>".to_owned()),
            computed: inputs_hashes
                .iter()
                .map(|inputs_hash| crate::core::hash::compute_file_hash(inputs_hash, &content))
                .collect(),
        })
        .collect();

    stale.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(stale)
}

/// Walk the consumer's repo from `base_dir`, find every alef-headered file, and
/// return the list of stale ones — where the embedded `alef:hash:<hex>` does not
/// equal the content-derived hash seeded by `inputs_hash`.
///
/// Skips obvious build/cache directories (`target/`, `node_modules/`, `_build/`,
/// `.alef/`, `parsers/`, `dist/`, `vendor/`, `.git/`) so verify stays fast on
/// large repos. Files without the alef header marker are skipped silently —
/// those are user-owned (scaffold-once Cargo.toml templates, composer.json,
/// gemspec, package.json, lockfiles, etc.) and alef has no claim.
pub(crate) fn verify_walk(base_dir: &std::path::Path, inputs_hash: &str) -> anyhow::Result<Vec<StaleMismatch>> {
    let mut stale: Vec<StaleMismatch> = collect_alef_hashes(base_dir)
        .into_iter()
        .filter(|(_, disk_hash, content)| {
            disk_hash
                .as_ref()
                .is_none_or(|disk_hash| crate::core::hash::compute_file_hash(inputs_hash, content) != *disk_hash)
        })
        .map(|(path, disk_hash, content)| StaleMismatch {
            path: path.display().to_string(),
            embedded: disk_hash.unwrap_or_else(|| "<missing>".to_owned()),
            computed: vec![crate::core::hash::compute_file_hash(inputs_hash, &content)],
        })
        .collect();

    stale.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(stale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Language;

    fn resolved_test_config() -> crate::core::config::ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
command = "pytest"

[crates.test.rust]
e2e = "cargo test"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn default_log_level_maps_verbosity_to_levels() {
        assert_eq!(default_log_level(0, false), "info");
        assert_eq!(default_log_level(1, false), "debug");
        assert_eq!(default_log_level(2, false), "trace");
        assert_eq!(default_log_level(9, false), "trace");
        // --quiet wins over any -v count.
        assert_eq!(default_log_level(0, true), "error");
        assert_eq!(default_log_level(3, true), "error");
    }

    #[test]
    fn required_post_build_failure_is_propagated_with_language_context() {
        let directory = tempfile::tempdir().expect("temporary project");
        let error = run_required_post_builds(
            &[Language::Swift],
            &crate::core::config::ResolvedCrateConfig::default(),
            directory.path(),
        )
        .expect_err("missing Swift build project must fail");

        assert!(
            error
                .to_string()
                .contains("failed to run required post-build steps for swift")
        );
    }

    #[test]
    fn resolve_test_languages_allows_explicit_test_only_language() {
        let config = resolved_test_config();
        let langs = resolve_test_languages(&config, Some(&["rust".to_string()]), true).unwrap();
        assert_eq!(langs, vec![Language::Rust]);
    }

    #[test]
    fn resolve_test_languages_appends_e2e_only_languages() {
        let config = resolved_test_config();
        let langs = resolve_test_languages(&config, None, true).unwrap();
        assert_eq!(langs, vec![Language::Python, Language::Rust]);
    }

    #[test]
    fn resolve_test_languages_omits_e2e_only_languages_without_e2e() {
        let config = resolved_test_config();
        let langs = resolve_test_languages(&config, None, false).unwrap();
        assert_eq!(langs, vec![Language::Python]);
    }

    fn gen_file(rel: &str, content: &str) -> crate::core::backend::GeneratedFile {
        crate::core::backend::GeneratedFile {
            path: std::path::PathBuf::from(rel),
            content: content.to_string(),
            generated_header: true,
        }
    }

    #[test]
    fn generated_files_match_disk_true_when_bodies_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("binding.go"), "package x\n\nvar a = 1\n").unwrap();
        let files = vec![gen_file("binding.go", "package x\n\nvar a = 1\n")];
        assert!(generated_files_match_disk(&files, dir.path()));
    }

    #[test]
    fn generated_files_match_disk_ignores_embedded_hash_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("binding.go"),
            "// alef:hash:deadbeef\npackage x\n\nvar a = 1\n",
        )
        .unwrap();
        let files = vec![gen_file("binding.go", "package x\n\nvar a = 1\n")];
        assert!(generated_files_match_disk(&files, dir.path()));
    }

    #[test]
    fn generated_files_match_disk_false_when_body_differs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("binding.go"), "package x\n\nvar a = 1\n").unwrap();
        let files = vec![gen_file("binding.go", "package x\n\nimport \"fmt\"\n\nvar a = 1\n")];
        assert!(!generated_files_match_disk(&files, dir.path()));
    }

    #[test]
    fn generated_files_match_disk_false_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![gen_file("binding.go", "package x\n")];
        assert!(!generated_files_match_disk(&files, dir.path()));
    }

    #[test]
    fn verify_walk_detects_an_edited_generated_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("binding.rs");
        let inputs_hash = "generation-inputs";
        let original = "// This file is auto-generated by alef — DO NOT EDIT.\nfn value() -> u8 { 1 }\n";
        let hash = crate::core::hash::compute_file_hash(inputs_hash, original);
        let finalized = crate::core::hash::inject_hash_line(original, &hash);
        std::fs::write(&path, finalized.replace("{ 1 }", "{ 2 }")).expect("edit generated file");

        let stale = verify_walk(directory.path(), inputs_hash).expect("verify generated files");

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].path, path.display().to_string());
    }

    #[test]
    fn verify_walk_detects_a_mixed_stamped_and_unstamped_generated_tree() {
        let directory = tempfile::tempdir().expect("tempdir");
        let inputs_hash = "generation-inputs";
        let path = directory.path().join("unstamped.rs");
        std::fs::write(
            &path,
            "// This file is auto-generated by alef — DO NOT EDIT.\nfn generated() {}\n",
        )
        .expect("write generated file");
        let stamped_path = directory.path().join("stamped.rs");
        let stamped_body = "// This file is auto-generated by alef — DO NOT EDIT.\nfn stamped() {}\n";
        let stamped_hash = crate::core::hash::compute_file_hash(inputs_hash, stamped_body);
        std::fs::write(
            &stamped_path,
            crate::core::hash::inject_hash_line(stamped_body, &stamped_hash),
        )
        .expect("write stamped generated file");

        let stale = verify_walk(directory.path(), inputs_hash).expect("verify generated files");

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].path, path.display().to_string());
        assert_eq!(stale[0].embedded, "<missing>");
    }
}
