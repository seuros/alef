use anyhow::{Context, Result};

fn run_required_post_builds(
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

/// Complete every generated artifact that depends on a backend build and then
/// enforce FFI source/header parity. Keeping these operations together prevents
/// commands from validating a cbindgen header before its producer runs or from
/// omitting the final parity gate. ~keep
pub(crate) fn complete_generated_artifacts(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Result<()> {
    run_required_post_builds(languages, config, base_dir)?;
    if !languages.contains(&crate::core::config::Language::Ffi) {
        return Ok(());
    }

    crate::cli::pipeline::ensure_ffi_header_freshness(config, base_dir, || {
        crate::cli::pipeline::build(config, &[crate::core::config::Language::Ffi], false)
    })
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

/// Extensions the ownership walk will open. A generated file whose extension is absent here is
/// invisible to `alef verify` entirely — not reported stale, not reported missing, and not
/// visible to [`find_stamp_disagreement`] either.
///
/// This list is only ONE of two filters. [`collect_alef_hashes`] needs a scanned extension AND
/// an `alef:hash:` line, so adding an extension does nothing for a language whose emitted files
/// carry no stamp at all — measured in tree-sitter-language-pack, `packages/java` and
/// `packages/go` have ZERO stamped files while `java`/`go` were already listed here. Those are
/// unreachable by any extension change; see the task tracking per-file stamping.
///
/// Scope of what a passing verify proves, because "verify passed" reads as the stronger claim
/// downstream: the hash covers generation INPUTS, not output bytes. One stamped manifest per
/// crate therefore detects input drift for that crate's outputs even when the outputs are
/// unstamped — but a hand-edit to an emitted file leaves inputs untouched and still verifies
/// fresh. Demonstrated in tslp: a dependency bumped inside a stamped, alef-generated
/// `Cargo.toml` reports fresh while the committed bytes differ from what alef would emit.
/// Freshness means the inputs have not moved, not that the file is what the generator writes.
///
/// `zig`/`dart`/`kt`/`kts`/`swift`/`gleam` were missing, which meant the cross-artifact straddle
/// gate could not see the zig side of a zig-vs-FFI-header straddle — the exact artifact pair it
/// exists to protect. `properties`/`pro`/`sh`/`props` were also stamped-but-unscanned, and
/// `packages/csharp/Directory.Build.props` is the ONLY stamped file in that whole package, so
/// csharp's freshness claim rested entirely on a file this walk never opened. Any new emitting
/// backend must add its extension here or its output silently leaves the
/// freshness claim.
///
/// This list must stay a **superset** of everything
/// [`crate::cli::pipeline::generate::write::marker_header_syntax`] can stamp. The walk filters on
/// extension *before* it reads any content, so an unlisted extension is invisible no matter what
/// marker the file carries. `xml`/`csproj`/`zon`/`cmake`/`gemspec` were added to that emit table
/// while missing here, which made their freshness claim unverifiable rather than merely
/// unverified — a stamped file nothing ever checks. ~keep
const VERIFY_SCAN_EXTENSIONS: &[&str] = &[
    "rs",
    "py",
    "pyi",
    "ts",
    "tsx",
    "js",
    "mjs",
    "cjs",
    "rb",
    "rbs",
    "php",
    "phpstub",
    "go",
    "java",
    "cs",
    "ex",
    "exs",
    "R",
    "r",
    "toml",
    "json",
    "md",
    "h",
    "c",
    "yaml",
    "yml",
    "zig",
    "dart",
    "kt",
    "kts",
    "swift",
    "gleam",
    "properties",
    "pro",
    "sh",
    "props",
    "xml",
    "csproj",
    "zon",
    "cmake",
    "gemspec",
];

/// Dotfiles alef stamps that [`VERIFY_SCAN_EXTENSIONS`] structurally cannot reach: `Path::extension`
/// returns `None` for a name that is entirely a leading-dot stem, so `.gitignore` has no extension
/// to match and would stay invisible no matter what is added to that list. Matched on the whole
/// file name instead.
///
/// Extensionless *stamped* files belong here for the same structural reason, not just dotfiles:
/// `Makefile`, `Rakefile` and `Makevars*` carry a `#` marker but have no extension to match, and
/// `go.mod` is matched by name rather than by its `mod` extension deliberately — `.mod` is shared
/// with unrelated binary formats (Fortran module files, tracker music), so listing the extension
/// would pull those into the walk. ~keep
const VERIFY_SCAN_FILENAMES: &[&str] = &[
    ".gitignore",
    ".gitattributes",
    ".editorconfig",
    "Makefile",
    "GNUmakefile",
    "makefile",
    "go.mod",
    "Rakefile",
    "Makevars",
    "Makevars.in",
    "Makevars.win.in",
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
            let name_ok = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| VERIFY_SCAN_FILENAMES.contains(&n));
            let ext_ok = name_ok
                || path
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

/// A cross-artifact ABI-generation disagreement: two or more alef-owned files
/// in the tree carry different values for the same `alef:<key>:` stamp (see
/// [`crate::core::hash::inject_stamp_line`]/[`extract_stamp`]).
///
/// A single `alef generate` run stamps every file it touches with the same
/// value, so two different values coexisting can only mean files from two
/// different generation runs are mixed in the tree — e.g. an FFI header
/// regenerated after a handle-representation change sitting next to a
/// binding backend's opaque-handle file that was not. `alef verify`'s
/// per-file `alef:hash:` staleness check cannot see this: each file's hash is
/// compared against the *current* generation inputs in isolation, never
/// against what a *different* file on disk was stamped with. ~keep
///
/// [`extract_stamp`]: crate::core::hash::extract_stamp
pub(crate) struct StampDisagreement {
    pub(crate) key: String,
    /// One `(display_path, value)` pair per distinct value found, so the
    /// report can show a representative example from each side of the split.
    pub(crate) examples: Vec<(String, String)>,
}

/// Find whether every alef-owned file under `base_dir` that carries an
/// `alef:<key>:` stamp agrees on its value.
///
/// Returns `None` when zero or one distinct value is present. Both are
/// silently fine, deliberately: zero stamped files means no backend in this
/// tree emits `key` yet — every consumer repo today, since nothing calls
/// `inject_stamp_line` — and that tree cannot be verified this way, not that
/// it disagrees; one distinct value is the healthy up-to-date case. Only 2+
/// distinct values is a provable disagreement (see [`StampDisagreement`]).
/// This intentionally does not attempt to flag "some files stamped, others
/// plausibly should be but aren't" as a softer warning: that requires knowing
/// which unstamped files are ABI-relevant, which a generic content walk
/// cannot determine — only the backend that emits a given file knows whether
/// it encodes the handle representation. ~keep
pub(crate) fn find_stamp_disagreement(base_dir: &std::path::Path, key: &str) -> Option<StampDisagreement> {
    let mut by_value: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (path, _hash, content) in collect_alef_hashes(base_dir) {
        let Some(value) = crate::core::hash::extract_stamp(&content, key) else {
            continue;
        };
        by_value.entry(value).or_insert_with(|| path.display().to_string());
    }
    if by_value.len() < 2 {
        return None;
    }
    Some(StampDisagreement {
        key: key.to_string(),
        examples: by_value.into_iter().map(|(value, path)| (path, value)).collect(),
    })
}

/// Display paths (joined onto `base_dir`, matching `StaleMismatch::path`'s
/// convention) of every alef-owned file in `files` that is entirely absent
/// from disk.
///
/// Ownership is decided by [`crate::core::backend::GeneratedFile::carries_alef_marker`]
/// via [`crate::cli::pipeline::managed_generated_files`] — the same predicate
/// [`collect_alef_hashes`] uses to select existing files for the hash walk.
/// A file without the marker (scaffold-once `Cargo.toml`/`package.json`/gemspec
/// templates, lockfiles) is outside alef's freshness claim and is never
/// reported missing here, even when absent — matching [`verify_walk`]'s scope
/// so a clean repo whose scaffold hasn't been (re-)run never fails verify. ~keep
fn missing_managed_paths(files: &[crate::core::backend::GeneratedFile], base_dir: &std::path::Path) -> Vec<String> {
    crate::cli::pipeline::managed_generated_files(files)
        .into_iter()
        .filter(|file| !base_dir.join(&file.path).exists())
        .map(|file| base_dir.join(&file.path).display().to_string())
        .collect()
}

/// Display paths of every alef-owned generated file that generation would now
/// produce for `config` but that does not exist on disk at all.
///
/// `verify_walk`/`verify_walk_multi` only ever see files that already exist and
/// carry an `alef:hash:` header, so a file generation would emit but that was
/// never written — a new API item a backend maps to a brand-new per-type file
/// (`src/backends/java/gen_bindings/mod.rs`, `src/backends/csharp/gen_bindings/mod.rs`
/// both emit one file per public type), forgotten after adding the item — is
/// invisible to a pure disk walk. Closing that gap requires knowing what
/// generation would produce, which is IR-derived for those backends and cannot
/// be answered from `alef.toml` alone; this mirrors `alef diff`'s approach
/// ([`crate::cli::pipeline::diff_files`], `src/cli/pipeline/generate/diff.rs`)
/// and pays a comparable cost: bindings, type stubs, service API, public API, and
/// scaffold are regenerated in memory (never written to disk) for every configured
/// language.
///
/// `clean = false` is passed to [`crate::cli::pipeline::generate`] deliberately,
/// not to save cost in CI (a fresh checkout's `.alef/` cache is always cold, so
/// CI always pays full regeneration regardless of this flag) but because it is
/// free and safe on a warm local machine: `crate::cli::cache::is_lang_cached`
/// only reports a cache hit when the IR+config hash still matches *and* every
/// previously recorded output path still exists on disk, so skipping a
/// cache-hit language can never hide a genuinely missing file. ~keep
pub(crate) fn find_missing_generated_files(
    languages: &[crate::core::config::Language],
    api: &crate::core::ir::ApiSurface,
    config: &crate::core::config::ResolvedCrateConfig,
    config_path: &std::path::Path,
    base_dir: &std::path::Path,
) -> anyhow::Result<Vec<String>> {
    let mut missing = Vec::new();

    let bindings = crate::cli::pipeline::generate(api, config, languages, false, config_path, false)?;
    for (_, files) in &bindings {
        missing.extend(missing_managed_paths(files, base_dir));
    }

    let stubs = crate::cli::pipeline::generate_stubs(api, config, languages)?;
    for (_, files) in &stubs {
        missing.extend(missing_managed_paths(files, base_dir));
    }

    // `generate_service_api` already no-ops when `api.services` is empty, so it is
    // safe to call unconditionally. `generate_public_api` has no such internal
    // guard — mirror `Commands::Generate`'s `resolved_cfg.generate.public_api` gate
    // so this stays a pure read matching what `alef generate` would actually
    // produce, not a superset of it. Both were missing here even though
    // `Commands::Generate` calls them, so a backend's per-type service/public-API
    // file (Java, C#) that was never written was invisible to `alef verify`. ~keep
    let svc_files = crate::cli::pipeline::generate_service_api(api, config, languages)?;
    for (_, files) in &svc_files {
        missing.extend(missing_managed_paths(files, base_dir));
    }

    if config.generate.public_api {
        let public_api_files = crate::cli::pipeline::generate_public_api(api, config, languages, config_path)?;
        for (_, files) in &public_api_files {
            missing.extend(missing_managed_paths(files, base_dir));
        }
    }

    let scaffold_files = crate::cli::pipeline::scaffold(api, config, languages, config_path)?;
    missing.extend(missing_managed_paths(&scaffold_files, base_dir));

    missing.sort();
    missing.dedup();
    Ok(missing)
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

    /// Seed one stamped file per name and return what the ownership walk actually opened.
    fn scanned_names(names: &[&str]) -> Vec<String> {
        let directory = tempfile::tempdir().expect("temporary project");
        for name in names {
            let path = directory.path().join(name);
            let marker = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
            std::fs::write(&path, format!("{marker}\nseeded = true\n")).expect("seed stamped file");
        }
        let mut found: Vec<String> = collect_alef_hashes(directory.path())
            .into_iter()
            .filter_map(|(path, _, _)| path.file_name()?.to_str().map(str::to_owned))
            .collect();
        found.sort();
        found
    }

    /// THE CANARY. Every name here is stamped by `marker_header_syntax` on the emit side, so a
    /// file alef wrote carries a hash this walk must be able to re-read. Before the list was
    /// widened these were stamped and then never opened — which reads as covered rather than as
    /// missing, and is why the gap survived its own doc comment's warning. ~keep
    #[test]
    fn ownership_walk_opens_every_extension_the_emit_side_stamps() {
        assert_eq!(
            scanned_names(&[
                "foo-config.cmake",
                "app.csproj",
                "gem.gemspec",
                "build.zig.zon",
                "pom.xml"
            ]),
            vec![
                "app.csproj",
                "build.zig.zon",
                "foo-config.cmake",
                "gem.gemspec",
                "pom.xml"
            ],
        );
    }

    /// The makefiles and `Rakefile` have no extension at all, and `go.mod`'s is the far-too-broad
    /// `mod` — shared with unrelated binary formats — so all of them are keyed on file name on the
    /// emit side and must be keyed the same way here.
    ///
    /// Every entry of `VERIFY_SCAN_FILENAMES` that is not a dotfile appears below, so an addition
    /// to that list without a matching read-side check fails here rather than passing quietly.
    ///
    /// `makefile` gets its own directory: macOS and Windows resolve it and `Makefile` to the same
    /// path, so seeding both in one directory silently writes a single file and the lowercase
    /// entry would look unscanned when it is only unwritten. ~keep
    #[test]
    fn ownership_walk_opens_the_filename_keyed_files_the_emit_side_stamps() {
        assert_eq!(scanned_names(&["makefile"]), vec!["makefile"]);
        assert_eq!(
            scanned_names(&[
                "Makefile",
                "GNUmakefile",
                "Rakefile",
                "Makevars",
                "Makevars.in",
                "Makevars.win.in",
                "go.mod"
            ]),
            vec![
                "GNUmakefile",
                "Makefile",
                "Makevars",
                "Makevars.in",
                "Makevars.win.in",
                "Rakefile",
                "go.mod"
            ],
        );
    }

    /// The other half of the predicate: widening the allowlist must not turn the walk into
    /// "open everything". Without this, both tests above would still pass if the filter were
    /// deleted outright. ~keep
    #[test]
    fn ownership_walk_still_skips_an_extension_alef_never_stamps() {
        assert!(scanned_names(&["notes.rtf", "archive.tar"]).is_empty());
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
        let generated = "// This file is auto-generated by alef — DO NOT EDIT.\npackage x\n\nvar a = 1\n";
        std::fs::write(
            dir.path().join("binding.go"),
            "// This file is auto-generated by alef — DO NOT EDIT.\n// alef:hash:deadbeef\npackage x\n\nvar a = 1\n",
        )
        .unwrap();
        let files = vec![gen_file("binding.go", generated)];
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

    fn gen_file_unheadered(rel: &str, content: &str) -> crate::core::backend::GeneratedFile {
        crate::core::backend::GeneratedFile {
            path: std::path::PathBuf::from(rel),
            content: content.to_string(),
            generated_header: false,
        }
    }

    /// The defect this closes: a backend that would produce a file (e.g. one
    /// Java/C# file per public type) is invisible to a pure disk walk when the
    /// file was never written — `alef verify` must catch that, not just an
    /// existing file whose hash drifted.
    #[test]
    fn missing_managed_paths_reports_an_absent_headered_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

        let missing = missing_managed_paths(&files, dir.path());

        assert_eq!(missing, vec![dir.path().join("SomeType.java").display().to_string()]);
    }

    /// Positive control: an up-to-date tree (every generated path already
    /// present on disk) must report nothing missing, regardless of the file's
    /// actual content — content drift is `verify_walk`'s job, not this check's.
    #[test]
    fn missing_managed_paths_reports_nothing_when_every_headered_file_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("SomeType.java"),
            "final class SomeType { /* stale */ }\n",
        )
        .unwrap();
        let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

        assert!(missing_managed_paths(&files, dir.path()).is_empty());
    }

    /// The required negative control: a legitimately user-owned, unheadered
    /// scaffold-once file (`Cargo.toml`, `package.json`, gemspec, lockfiles —
    /// see `verify_walk`'s doc comment) that is absent must NOT be reported
    /// missing. Getting this wrong would fail verify on every clean repo whose
    /// scaffold-once files simply haven't been (re-)generated locally.
    #[test]
    fn missing_managed_paths_ignores_an_absent_unheadered_scaffold_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![gen_file_unheadered("Cargo.toml", "[package]\nname = \"demo\"\n")];

        assert!(missing_managed_paths(&files, dir.path()).is_empty());
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

    /// `find_stamp_disagreement` walks `collect_alef_hashes`, which only yields files that
    /// carry an `alef:hash:` line — so a fixture bearing only a stamp is invisible to it and
    /// every assertion over it passes vacuously. Both lines must be injected, in that order,
    /// to produce a file shaped like one a backend actually emits. ~keep
    fn write_stamped(dir: &std::path::Path, name: &str, key: &str, value: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        let body = "// This file is auto-generated by alef — DO NOT EDIT.\nfn generated() {}\n";
        let stamped = crate::core::hash::inject_stamp_line(body, key, value);
        let hash = crate::core::hash::compute_file_hash("test-inputs-hash", &stamped);
        std::fs::write(&path, crate::core::hash::inject_hash_line(&stamped, &hash)).expect("write stamped file");
        path
    }

    /// Guards the fixture itself, because the bug this replaces was a fixture bug, not a
    /// logic bug: if `write_stamped` ever stops producing a file the hash walk can see, the
    /// disagreement tests below go quietly vacuous instead of failing. ~keep
    #[test]
    fn write_stamped_produces_a_file_the_hash_walk_actually_collects() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_stamped(dir.path(), "header.h", "handle-abi", "1");

        let collected = collect_alef_hashes(dir.path());
        assert_eq!(
            collected.len(),
            1,
            "the stamped fixture must be visible to the hash walk"
        );
        assert_eq!(
            crate::core::hash::extract_stamp(&collected[0].2, "handle-abi").as_deref(),
            Some("1"),
            "the stamp must survive alongside the hash line"
        );
    }

    /// The concrete cross-artifact ABI straddle this closes: an FFI-side file
    /// stamped for one ABI generation coexisting with a binding-side file
    /// stamped for a different one must be reported, even though each file's
    /// own `alef:hash:` may be perfectly fresh relative to current inputs.
    #[test]
    fn find_stamp_disagreement_reports_two_distinct_values() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_stamped(dir.path(), "header.h", "handle-abi", "1");
        write_stamped(dir.path(), "binding.zig", "handle-abi", "2");

        let disagreement =
            find_stamp_disagreement(dir.path(), "handle-abi").expect("two distinct stamp values must be reported");

        assert_eq!(disagreement.key, "handle-abi");
        assert_eq!(disagreement.examples.len(), 2);
        let values: Vec<&str> = disagreement.examples.iter().map(|(_, v)| v.as_str()).collect();
        assert!(values.contains(&"1"));
        assert!(values.contains(&"2"));
    }

    /// Positive control: every stamped file agreeing must not be reported —
    /// this is the healthy, fully-regenerated-together state.
    #[test]
    fn find_stamp_disagreement_is_none_when_every_stamped_file_agrees() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_stamped(dir.path(), "header.h", "handle-abi", "2");
        write_stamped(dir.path(), "binding.zig", "handle-abi", "2");

        assert!(find_stamp_disagreement(dir.path(), "handle-abi").is_none());
    }

    /// The required negative control for the rollout gap the task describes:
    /// a tree where no backend has started emitting the stamp yet (every
    /// consumer repo today) must not be reported as disagreeing — there is
    /// nothing to compare, not a proven mismatch.
    #[test]
    fn find_stamp_disagreement_is_none_when_nothing_is_stamped() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("header.h"),
            "// This file is auto-generated by alef — DO NOT EDIT.\nfn generated() {}\n",
        )
        .expect("write unstamped file");

        assert!(find_stamp_disagreement(dir.path(), "handle-abi").is_none());
    }

    /// A file stamped under a different key must not be mistaken for a
    /// `handle-abi` disagreement — `find_stamp_disagreement` is keyed, not a
    /// blanket "does this file have any stamp" check.
    #[test]
    fn find_stamp_disagreement_ignores_a_different_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_stamped(dir.path(), "header.h", "some-other-marker", "1");
        write_stamped(dir.path(), "binding.zig", "some-other-marker", "2");

        assert!(find_stamp_disagreement(dir.path(), "handle-abi").is_none());
    }
}
