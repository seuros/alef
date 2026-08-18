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
    // A nested git worktree (`git worktree add .claude/worktrees/<name>`) is a second, complete
    // checkout of the same repository. Walking it would report another branch's stamps as this
    // tree's, and it only became reachable once `.claude` was taken off the blanket dot-directory
    // prune below. A worktree's `.git` is a FILE, not a directory, so the `.git` entry above does
    // not stop the descent. ~keep
    "worktrees",
];

/// Dot-directories [`collect_alef_hashes`] descends into despite its blanket dot-directory prune.
///
/// The prune exists to keep the walk out of tool caches, but it is a proxy — "starts with a dot"
/// is not "is a cache" — and alef writes stamped, alef-owned output into several dot-directories:
/// `.cargo/config.toml` from [`crate::scaffold::scaffold`], and every `SKILL.md` under an agent
/// skills root. Those files were stamped and then never read back: the walk pruned their parent
/// before opening them, so `alef verify` could not report them stale no matter how far they
/// drifted. Refusing to *stamp* them instead would be worse — the stamp is also what makes poly's
/// built-in generated-file skip leave them alone, so unstamping them hands their formatting to
/// poly and their staleness to nobody.
///
/// Incomplete by construction, and knowingly so: skills roots are pure configuration
/// (`DocsSkillsConfig::outputs`), so a consumer that writes skills into a dot-directory not named
/// here is still invisible to the walk. Closing that fully requires the walk to consult the
/// resolved config, which it currently has no access to. ~keep
const VERIFY_SCAN_DOT_DIRS: &[&str] = &[
    ".cargo", ".github",
    // Agent-skill roots observed in consumer ownership records (`.alef-ownership.toml` lists
    // `.agents/skills/*/SKILL.md` and `.claude/skills/*/SKILL.md`), so these are not speculative:
    // alef is already writing stamped `SKILL.md` files under them. ~keep
    ".agents", ".claude", ".codex", ".cursor", ".gemini",
];

/// Extensions the ownership walk will open. A generated file whose extension is absent here is
/// invisible to `alef verify` entirely — not reported stale, not reported missing, and not
/// visible to [`find_stamp_disagreement`] either.
///
/// This list is only ONE of two filters. [`collect_alef_hashes`] needs a scanned extension AND
/// an `alef:hash:` line, so adding an extension does nothing for a language whose emitted files
/// carry no stamp at all — measured in a consumer repo, `packages/java` and `packages/go` had
/// ZERO stamped files while `java`/`go` were already listed here. Those are unreachable by any
/// extension change; see the task tracking per-file stamping.
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
                let pruned_as_dotfile = name.starts_with('.') && !VERIFY_SCAN_DOT_DIRS.contains(&name);
                if VERIFY_SKIP_DIRS.contains(&name) || pruned_as_dotfile {
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

/// A generated file alef would own and mark, that already exists on disk but
/// carries no provenance marker at all.
///
/// This is a different, unrecoverable condition from a stale [`StaleMismatch`]
/// or a [`missing_managed_paths`] entry: the write guard in
/// `crate::cli::pipeline::generate::write::write_files_report` and
/// `crate::cli::pipeline::generate::scaffold::write_scaffold_files_report`
/// refuses to touch a pre-existing file that carries no marker (it cannot tell
/// a hand-written file from an alef output that predates the marker system),
/// and the marker can only ever be added *by* a write the guard has already
/// authorised — so an unmarked pre-existing file is frozen forever. Running
/// `alef generate` again does nothing; a human must read the file, then either
/// adopt it (paste `remedy` in and rerun `alef generate`) or delete it so
/// generation can write it cleanly. ~keep
pub(crate) struct FrozenFile {
    pub(crate) path: String,
    /// The literal marker line to add to the top of the file, or `None` when
    /// the format has no comment syntax to carry one (`.json`, lockfiles).
    pub(crate) remedy: Option<String>,
    /// A leading line in the existing file that looks like a failed attempt at a marker --
    /// see [`crate::core::hash::near_miss_marker`] -- so the report can point at what's already
    /// there instead of only showing what should be there. `None` when the file's leading lines
    /// don't mention alef and generation at all (a plain hand-written file). ~keep
    pub(crate) near_miss: Option<String>,
}

/// The line within `content` that [`crate::core::hash::content_has_alef_marker`]
/// would recognize as the provenance marker, if any.
///
/// Delegates the actual match to `content_has_alef_marker` itself, applied one
/// line at a time, instead of re-implementing its marker text here — so the
/// two can never drift apart. ~keep
fn marker_line(content: &str) -> Option<&str> {
    content
        .lines()
        .find(|line| crate::core::hash::content_has_alef_marker(line))
}

/// [`FrozenFile`] entries for every alef-owned file in `files` that already
/// exists on disk but carries no marker.
///
/// Uses the same ownership predicate as [`missing_managed_paths`] — a
/// scaffold-once file alef never marks is excluded here exactly as it is from
/// the missing-file check, so a hand-edited `Cargo.toml`/`package.json`
/// template is never mistaken for a frozen generated file.
///
/// The remedy text is read straight from the in-memory `GeneratedFile::content`
/// first, because a self-marking backend (custom Swift/Kotlin/Dart/Gleam/Zig
/// headers, `docs::render`'s HTML-commented `.md` pages) already bakes its
/// literal header into `content` regardless of `generated_header`. Only when
/// that content carries no marker yet — the common case, where the header is
/// added later by `write_files_report`'s `ensure_generated_header` pass — does
/// this fall back to reconstructing it from the path via
/// [`crate::cli::pipeline::provenance_header_for_path`]. ~keep
fn frozen_managed_paths(files: &[crate::core::backend::GeneratedFile], base_dir: &std::path::Path) -> Vec<FrozenFile> {
    crate::cli::pipeline::managed_generated_files(files)
        .into_iter()
        .filter_map(|file| {
            let full_path = base_dir.join(&file.path);
            let existing = std::fs::read_to_string(&full_path).ok()?;
            if crate::core::hash::content_has_alef_marker(&existing) {
                return None;
            }
            let remedy = marker_line(&file.content).map(str::to_owned).or_else(|| {
                let header = crate::cli::pipeline::provenance_header_for_path(&file.path)?;
                marker_line(&header).map(str::to_owned)
            });
            let near_miss = crate::core::hash::near_miss_marker(&existing).map(str::to_owned);
            Some(FrozenFile {
                path: full_path.display().to_string(),
                remedy,
                near_miss,
            })
        })
        .collect()
}

/// Missing and frozen generated files found for one crate — see
/// [`find_missing_and_frozen_generated_files`].
#[derive(Default)]
pub(crate) struct MissingAndFrozenFiles {
    pub(crate) missing: Vec<String>,
    pub(crate) frozen: Vec<FrozenFile>,
    /// [`StageFailure`]s [`collect_managed_surface`] tolerated while still building the
    /// rest of the surface, rendered as `[<stage>] <message>`. `alef verify` is
    /// read-only, so it has no target to decide "does this affect me" the way `alef
    /// adopt` does -- every tolerated failure is real debt and belongs in the report,
    /// never silently absorbed into a clean-looking zero. The `missing`/`frozen` lists
    /// above are still accurate despite these: the failing stage's own files are
    /// absorbed into the surface regardless of its error (see
    /// [`collect_managed_surface`]'s doc), so this is additional signal, not a
    /// disclaimer that the rest of the report cannot be trusted. ~keep
    pub(crate) stage_failures: Vec<String>,
}

/// Find both generated files that generation would now produce for `config`
/// but that do not exist on disk at all ([`missing_managed_paths`]), and
/// generated files that do exist but were never marked
/// ([`frozen_managed_paths`]).
///
/// `verify_walk`/`verify_walk_multi` only ever see files that already exist and
/// carry an `alef:hash:` header, so a file generation would emit but that was
/// never written — a new API item a backend maps to a brand-new per-type file
/// (`src/backends/java/gen_bindings/mod.rs`, `src/backends/csharp/gen_bindings/mod.rs`
/// both emit one file per public type), forgotten after adding the item — is
/// invisible to a pure disk walk. Same for a frozen file: it fails ownership,
/// not freshness, so a walk that only opens marker-bearing files by
/// construction cannot see it either (see [`collect_alef_hashes`]'s doc). Both
/// gaps require knowing what generation would produce, which is IR-derived for
/// some backends and cannot be answered from `alef.toml` alone; this mirrors
/// `alef diff`'s approach ([`crate::cli::pipeline::diff_files`],
/// `src/cli/pipeline/generate/diff.rs`) and pays a comparable cost: every stage
/// in [`collect_managed_surface`] is regenerated in memory (never written to
/// disk) for every configured language — paid once here for both checks
/// together, since paying it twice (one full regeneration pass per check) would
/// double the cost of every `alef verify` run for no benefit. ~keep
///
/// `clean = false` is what [`collect_managed_surface`] passes to
/// `crate::cli::pipeline::generate` deliberately,
/// not to save cost in CI (a fresh checkout's `.alef/` cache is always cold, so
/// CI always pays full regeneration regardless of this flag) but because it is
/// free and safe on a warm local machine: `crate::cli::cache::is_lang_cached`
/// only reports a cache hit when the IR+config hash still matches *and* every
/// previously recorded output path still exists on disk, so skipping a
/// cache-hit language can never hide a genuinely missing or frozen file. ~keep
pub(crate) fn find_missing_and_frozen_generated_files(
    languages: &[crate::core::config::Language],
    api: &crate::core::ir::ApiSurface,
    config: &crate::core::config::ResolvedCrateConfig,
    config_path: &std::path::Path,
    base_dir: &std::path::Path,
) -> anyhow::Result<MissingAndFrozenFiles> {
    let (surface, stage_failures) = collect_managed_surface(languages, api, config, config_path, base_dir)?;
    let mut result = MissingAndFrozenFiles {
        missing: missing_managed_paths(&surface, base_dir),
        frozen: frozen_managed_paths(&surface, base_dir),
        stage_failures: stage_failures
            .into_iter()
            .map(|failure| format!("[{}] {}", failure.stage, failure.message))
            .collect(),
    };
    result.missing.sort();
    result.missing.dedup();
    result.frozen.sort_by(|a, b| a.path.cmp(&b.path));
    result.frozen.dedup_by(|a, b| a.path == b.path);
    result.stage_failures.sort();
    result.stage_failures.dedup();
    Ok(result)
}

/// Insert `files` into `surface`, letting a later stage win the path.
///
/// Last-write-wins mirrors disk: `alef all` runs these stages in sequence and each
/// one overwrites whatever the previous stage left at the same path, so the bytes a
/// reader is asked to consent to (and the bytes `alef verify` reasons about) must be
/// the *last* stage's, not the first's. The only stages that actually collide are
/// local-mode e2e and registry-mode test apps, which share the snippet output root. ~keep
fn absorb_stage(
    surface: &mut std::collections::BTreeMap<std::path::PathBuf, crate::core::backend::GeneratedFile>,
    files: Vec<crate::core::backend::GeneratedFile>,
) {
    for file in files {
        surface.insert(file.path.clone(), file);
    }
}

/// One [`collect_managed_surface`] stage that rendered files and also reported a
/// failure alongside them.
///
/// Only the two e2e stages can produce one of these today: they are the only stages
/// whose `Result` already separates "files it rendered" from "whether it is happy with
/// them" (see `generate_e2e`'s doc) — every other stage's failure has no partial output
/// to report and stays a hard `collect_managed_surface` error via `?`. `paths` is what
/// that stage's own invocation actually rendered, kept so a caller with a specific
/// target (`alef adopt`) can decide whether this failure is even about a path it asked
/// for, via [`Self::affects_any`]. `alef verify` has no target and reports every one of
/// these unconditionally instead. ~keep
pub(crate) struct StageFailure {
    pub(crate) stage: &'static str,
    pub(crate) message: String,
    pub(crate) paths: Vec<std::path::PathBuf>,
}

impl StageFailure {
    /// Whether any of `targets` could have come from this stage — derived from
    /// [`crate::cli::commands::adopt::matches_target`], the identical predicate `alef
    /// adopt`'s own candidate selection trusts, rather than a second, hand-maintained
    /// notion of what counts as a match. ~keep
    pub(crate) fn affects_any(&self, targets: &[String]) -> bool {
        self.paths.iter().any(|path| {
            targets
                .iter()
                .any(|target| crate::cli::commands::adopt::matches_target(target, path))
        })
    }
}

/// Every file this crate's configuration would cause alef to emit, from **all**
/// generation stages, deduplicated by path.
///
/// This is the single answer to "what does alef own here", and it has exactly two
/// consumers: [`find_missing_and_frozen_generated_files`] (`alef verify`'s report)
/// and `bin_cli::aux_commands`'s `Commands::Adopt` (the remedy the report points
/// at). They are the report and the fix for the same fact, so they must not be able
/// to disagree — and they did. Both were built from hand-maintained stage lists
/// that each omitted different stages: adopt covered bindings + stubs + scaffold and
/// verify covered those plus service/public API, and *neither* covered e2e, test
/// apps, READMEs or docs. Consumer repos then committed regenerated e2e snippet
/// `.md` files that carried no marker (`e2e::snippets::render_snippet_markdown` did
/// not route through `docs::render::with_html_header` at the time), so the write
/// guard froze 15,677 of them in one repo and 9,139 in another, `alef verify` was
/// structurally blind to every one, and `alef adopt` on a snippet glob bailed with
/// "no alef-managed output matches" — the designed way out did not reach the files
/// that needed it. A new emitter added to `alef all` must be added here too, or it
/// re-opens exactly that hole. ~keep
///
/// Every stage below is a pure in-memory render; nothing here writes to disk. Docs
/// are the one stage whose failure is downgraded: `docs::generate_docs_stage`
/// deliberately returns the pages it rendered *alongside* an error from a later
/// sub-step (snippet validation, CLI/MCP extraction), and neither of this function's
/// consumers should lose the whole managed set — or refuse to unfreeze a binding
/// file — because a docs sub-step is unhappy. ~keep
///
/// Returns the surface alongside every [`StageFailure`] this call tolerated rather than
/// aborting on. Both consumers used to see a stage failure as `Err` for the *whole*
/// function -- including `alef verify`, a read-only report, which would abort before
/// its own frozen-file walk ever ran, and `alef adopt`, which would refuse a path with
/// no relationship whatsoever to the failing stage. Neither reads a `Result::Err` here
/// as license to skip reporting the failure: `alef verify` prints every
/// [`StageFailure`] as its own section (see [`find_missing_and_frozen_generated_files`]);
/// `alef adopt` bails with it when [`StageFailure::affects_any`] says the operator's own
/// target could have come from that stage, and otherwise logs it at DEBUG and proceeds.
/// A caller that dropped the returned `Vec<StageFailure>` on the floor would silently
/// reintroduce the exact bug this return shape exists to prevent -- a report that looks
/// clean because the thing that would have said otherwise never ran. ~keep
pub(crate) fn collect_managed_surface(
    languages: &[crate::core::config::Language],
    api: &crate::core::ir::ApiSurface,
    config: &crate::core::config::ResolvedCrateConfig,
    config_path: &std::path::Path,
    base_dir: &std::path::Path,
) -> anyhow::Result<(Vec<crate::core::backend::GeneratedFile>, Vec<StageFailure>)> {
    // Every stage below reads the same `&api`/`&config` and returns owned files, so rendering
    // them is embarrassingly parallel -- and rendering is where the time goes: the two e2e
    // stages alone emit several thousand files each on a full consumer tree, which is most of
    // what made a single-path `alef adopt` cost half a minute.
    //
    // ABSORPTION is not parallel-safe, and the distinction is the whole point of this shape:
    // `absorb_stage` is last-wins on a path collision, so the sequence stages are folded into
    // `surface` is what decides which stage owns a contested path. Render concurrently, fold in
    // the original order -- `rayon`'s indexed `collect` preserves it. ~keep
    type StageOutcome = anyhow::Result<(Vec<crate::core::backend::GeneratedFile>, Vec<StageFailure>)>;
    type Stage<'a> = Box<dyn Fn() -> StageOutcome + Send + Sync + 'a>;

    // `generate_service_api` already no-ops when `api.services` is empty, so it is
    // safe to call unconditionally. `generate_public_api` has no such internal
    // guard — mirror `Commands::Generate`'s `config.generate.public_api` gate so
    // this stays a faithful picture of what `alef generate` would produce, not a
    // superset of it. ~keep
    let bindings_stage: Stage<'_> = Box::new(|| {
        let mut files = Vec::new();
        for (_, produced) in crate::cli::pipeline::generate(api, config, languages, false, config_path, false)? {
            files.extend(produced);
        }
        for (_, produced) in crate::cli::pipeline::generate_service_api(api, config, languages)? {
            files.extend(produced);
        }
        Ok((files, Vec::new()))
    });
    let scaffold_stage: Stage<'_> = Box::new(|| {
        let mut files = crate::cli::pipeline::scaffold(api, config, languages, config_path)?;
        for (_, produced) in crate::cli::pipeline::generate_stubs(api, config, languages)? {
            files.extend(produced);
        }
        if config.generate.public_api {
            for (_, produced) in crate::cli::pipeline::generate_public_api(api, config, languages, config_path)? {
                files.extend(produced);
            }
        }
        Ok((files, Vec::new()))
    });
    // Both e2e modes, because they emit to different roots (`e2e.output` versus
    // `e2e.registry.output`) and a file can be frozen under either. `generate_e2e`
    // returns its per-backend generator failure alongside the files it did produce
    // rather than folding it into this `Result`, so a failure here no longer aborts
    // `collect_managed_surface` (as it used to, with `alef verify`'s frozen-file walk
    // and every later stage as collateral): it is absorbed into `stage_failures`
    // instead, and every caller of this function is required to look at that list --
    // see this function's own doc for why neither consumer may drop it silently. ~keep
    let e2e_local_stage: Stage<'_> = Box::new(|| {
        let Some(e2e_config) = &config.e2e else {
            return Ok((Vec::new(), Vec::new()));
        };
        let (files, generator_error) =
            crate::e2e::generate_e2e(config, e2e_config, None, &api.types, &api.enums, &api.functions)
                .context("failed to render the e2e stage of alef's managed output")?;
        Ok(stage_failure_for("e2e", generator_error, files))
    });
    let e2e_registry_stage: Stage<'_> = Box::new(|| {
        let Some(e2e_config) = &config.e2e else {
            return Ok((Vec::new(), Vec::new()));
        };
        let mut registry_config = e2e_config.clone();
        registry_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
        let (files, generator_error) =
            crate::e2e::generate_e2e(config, &registry_config, None, &api.types, &api.enums, &api.functions)
                .context("failed to render the registry-mode test-app stage of alef's managed output")?;
        Ok(stage_failure_for("test-apps (registry mode)", generator_error, files))
    });
    let readme_stage: Stage<'_> = Box::new(|| {
        let readme_languages = crate::readme::expand_configured_readme_languages(config, languages);
        Ok((
            crate::cli::pipeline::readme(api, config, &readme_languages)?,
            Vec::new(),
        ))
    });
    // Neither `alef adopt` nor `alef verify` asks whether a snippet compiles, type-checks, or
    // runs -- they ask what file surface alef's configuration owns, and that compile step
    // produces no files (see `generate_docs_stage_without_snippet_compile_validation`'s doc).
    // Running it here was the entire cost of the 90-minute `alef adopt` on a single `Cargo.toml`:
    // thousands of per-backend toolchain subprocess invocations to answer an ownership question
    // that never looked at their result, since a docs-stage `Err` is already downgraded to the
    // debug log below regardless of which sub-step produced it. ~keep
    let docs_stage: Stage<'_> = Box::new(|| {
        let doc_languages = resolve_doc_languages(config, None)?;
        let (doc_files, doc_result) = crate::docs::generate_docs_stage_without_snippet_compile_validation(
            api,
            config,
            &doc_languages,
            None,
            base_dir,
        );
        if let Err(error) = doc_result {
            tracing::debug!("docs stage reported {error:#}; using the pages it rendered before the failure");
        }
        Ok((doc_files, Vec::new()))
    });

    let stages = [
        bindings_stage,
        scaffold_stage,
        e2e_local_stage,
        e2e_registry_stage,
        readme_stage,
        docs_stage,
    ];
    let outcomes: Vec<StageOutcome> = {
        use rayon::prelude::*;
        stages.par_iter().map(|stage| stage()).collect()
    };

    let mut surface = std::collections::BTreeMap::new();
    let mut stage_failures = Vec::new();
    for outcome in outcomes {
        let (files, failures) = outcome?;
        stage_failures.extend(failures);
        absorb_stage(&mut surface, files);
    }

    Ok((surface.into_values().collect(), stage_failures))
}

/// Pair a stage's rendered files with the [`StageFailure`] its generator reported alongside them,
/// if any. Split out only so the two e2e stage closures state the pairing once instead of twice.
fn stage_failure_for(
    stage: &'static str,
    generator_error: Option<anyhow::Error>,
    files: Vec<crate::core::backend::GeneratedFile>,
) -> (Vec<crate::core::backend::GeneratedFile>, Vec<StageFailure>) {
    let failures = generator_error
        .map(|error| StageFailure {
            stage,
            message: format!("{error:#}"),
            paths: files.iter().map(|file| file.path.clone()).collect(),
        })
        .into_iter()
        .collect();
    (files, failures)
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

    /// Seed one stamped file per relative path (creating parent directories) and return the
    /// repository-relative paths the ownership walk actually opened, sorted.
    fn scanned_relative_paths(relative_paths: &[&str]) -> Vec<String> {
        let directory = tempfile::tempdir().expect("temporary project");
        for relative in relative_paths {
            let path = directory.path().join(relative);
            std::fs::create_dir_all(path.parent().expect("seeded path has a parent")).expect("seed parent directory");
            let marker = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
            std::fs::write(&path, format!("{marker}\nseeded = true\n")).expect("seed stamped file");
        }
        let mut found: Vec<String> = collect_alef_hashes(directory.path())
            .into_iter()
            .filter_map(|(path, _, _)| {
                path.strip_prefix(directory.path())
                    .ok()?
                    .to_str()
                    .map(|value| value.replace('\\', "/"))
            })
            .collect();
        found.sort();
        found
    }

    /// alef stamps files inside dot-directories (`.cargo/config.toml`, agent-skill `SKILL.md`s)
    /// that the walk used to prune wholesale, so those stamps were written and never read: no
    /// amount of drift could make `alef verify` report them.
    ///
    /// Paired control in one run, because a walk that finds nothing and a walk that finds
    /// everything are the same green: a stamped file in an ordinary directory must be found (the
    /// walk works at all), a stamped file in `.cargo` must now also be found (the fix), and a
    /// stamped file in `.venv` must still be missed (the prune still keeps the walk out of tool
    /// caches — the fix is an allowlist, not a removal).
    #[test]
    fn the_ownership_walk_reaches_the_dot_directories_alef_stamps() {
        let found = scanned_relative_paths(&[
            "packages/reachable.toml",
            ".cargo/config.toml",
            ".github/skills/api/SKILL.md",
            ".venv/lib/cached.toml",
        ]);

        assert!(
            found.contains(&"packages/reachable.toml".to_string()),
            "control: a stamped file outside every dot-directory must be found, else this test \
             proves nothing about the dot-directory cases; walk returned {found:?}"
        );
        assert!(
            found.contains(&".cargo/config.toml".to_string()),
            "alef writes and stamps `.cargo/config.toml` itself; a stamp nothing ever reads back \
             is not a freshness check. Walk returned {found:?}"
        );
        assert!(
            found.contains(&".github/skills/api/SKILL.md".to_string()),
            "generated agent skills are stamped alef output and must be verifiable; walk returned \
             {found:?}"
        );
        assert!(
            !found.contains(&".venv/lib/cached.toml".to_string()),
            "the dot-directory prune must still keep the walk out of tool caches -- the fix is an \
             allowlist of the dot-directories alef writes into, not a removal of the prune. Walk \
             returned {found:?}"
        );
    }

    /// A nested git worktree is a second checkout of the same repository. It became reachable the
    /// moment `.claude` came off the blanket prune, and walking it reports another branch's
    /// stamps as this tree's.
    #[test]
    fn the_ownership_walk_does_not_descend_into_a_nested_worktree() {
        let found = scanned_relative_paths(&[".claude/skills/api/SKILL.md", ".claude/worktrees/other/config.toml"]);

        assert!(
            found.contains(&".claude/skills/api/SKILL.md".to_string()),
            "control: `.claude` must be walked, else the exclusion below is vacuous; walk \
             returned {found:?}"
        );
        assert!(
            !found.contains(&".claude/worktrees/other/config.toml".to_string()),
            "a nested worktree is a different checkout of this repository; its stamps are not \
             this tree's. Walk returned {found:?}"
        );
    }

    /// THE AGREEMENT CANARY. `alef verify`'s frozen-file report and `alef adopt`'s candidate
    /// set are the report and the remedy for one fact, so a path in one and not the other
    /// sends a reader to a command that refuses them. They diverged exactly that way: each
    /// was built from its own hand-maintained stage list, adopt's missing service/public API
    /// and both missing e2e, test apps, READMEs and docs — which is why `alef adopt` on an
    /// e2e snippet glob bailed with "no alef-managed output matches" while 15,677 snippets
    /// sat frozen and unreported.
    ///
    /// Asserting on behaviour would need a full extraction + generation pass against a real
    /// crate, which is what made the divergence invisible to the test suite in the first
    /// place. This asserts on the structure that produced it instead: each consumer derives
    /// its set from [`collect_managed_surface`] and enumerates no stage of its own. It fails
    /// the moment either one grows a private list again, which is the regression. ~keep
    #[test]
    fn both_consumers_build_their_managed_set_only_from_the_shared_surface() {
        // Every stage entry point `collect_managed_surface` composes. A consumer that
        // names one inside its own region is re-deriving the surface instead of sharing
        // it. `generate(` carries its parenthesis because `generate_` prefixes several
        // of the others. ~keep
        let stage_calls = [
            "pipeline::generate(",
            "pipeline::generate_stubs(",
            "pipeline::generate_service_api(",
            "pipeline::generate_public_api(",
            "pipeline::scaffold(",
            "pipeline::readme(",
            "e2e::generate_e2e(",
            "docs::generate_docs_stage(",
        ];
        // Each region is the consumer's own code, cut so it excludes the shared collector
        // itself (which must name every stage) and, for `aux_commands`, the unrelated
        // `Commands::Init` arm, which legitimately generates and writes. ~keep
        let regions = [
            (
                "alef verify's frozen report",
                include_str!("helpers.rs")
                    .split("pub(crate) fn collect_managed_surface")
                    .next()
                    .expect("helpers splits on the shared collector"),
            ),
            (
                "alef adopt's candidate set",
                include_str!("aux_commands.rs")
                    .split("Commands::Adopt {")
                    .nth(1)
                    .and_then(|rest| rest.split("Commands::Migrate {").next())
                    .expect("aux_commands splits on the adopt arm"),
            ),
        ];
        for (name, region) in regions {
            for call in stage_calls {
                assert!(
                    !region.contains(call),
                    "{name} calls {call} directly -- the frozen report and the candidate set \
                     must not enumerate generation stages separately, or they disagree again"
                );
            }
            assert!(
                region.contains("collect_managed_surface("),
                "{name} must derive its managed set from the shared surface"
            );
        }
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
    fn marker_line_finds_the_line_carrying_the_provenance_marker() {
        let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);

        assert_eq!(
            marker_line(&header),
            Some("// This file is auto-generated by alef — DO NOT EDIT.")
        );
    }

    #[test]
    fn marker_line_finds_nothing_in_content_without_a_marker() {
        assert_eq!(marker_line("final class SomeType {}\n"), None);
    }

    /// The defect this closes: a pre-existing file at a path alef would emit
    /// and mark, but that predates the marker system, deadlocks the write
    /// guard forever (see `FrozenFile`'s doc). `alef verify` must surface it
    /// even though it never carries a hash to compare, which is why this is a
    /// distinct check from `verify_walk`'s stale-hash comparison.
    #[test]
    fn frozen_managed_paths_reports_an_unmarked_pre_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("SomeType.java"), "final class SomeType {}\n").unwrap();
        let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

        let frozen = frozen_managed_paths(&files, dir.path());

        assert_eq!(frozen.len(), 1);
        assert_eq!(frozen[0].path, dir.path().join("SomeType.java").display().to_string());
        assert_eq!(
            frozen[0].remedy.as_deref(),
            Some("// This file is auto-generated by alef — DO NOT EDIT.")
        );
        assert_eq!(
            frozen[0].near_miss, None,
            "plain hand-written content has no near miss to report"
        );
    }

    /// A pre-existing file whose leading lines look like a failed attempt at a marker (mentions
    /// both "alef" and "generated" without matching `content_has_alef_marker`) is still frozen,
    /// but the report should name what's already there, not just what's missing.
    #[test]
    fn frozen_managed_paths_reports_a_near_miss_when_one_is_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("SomeType.java"),
            "// This alef-generated file should not be edited.\nfinal class SomeType {}\n",
        )
        .unwrap();
        let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

        let frozen = frozen_managed_paths(&files, dir.path());

        assert_eq!(frozen.len(), 1);
        assert_eq!(
            frozen[0].near_miss.as_deref(),
            Some("// This alef-generated file should not be edited.")
        );
    }

    /// A managed file that already carries the marker is stale-or-fresh
    /// territory (`verify_walk`'s job), never frozen — the guard that would
    /// deadlock a write never engages once a marker is present.
    #[test]
    fn frozen_managed_paths_reports_nothing_when_the_existing_file_already_carries_the_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marked = format!(
            "{}final class SomeType {{}}\n",
            crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash)
        );
        std::fs::write(dir.path().join("SomeType.java"), &marked).unwrap();
        let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

        assert!(frozen_managed_paths(&files, dir.path()).is_empty());
    }

    /// A managed file that does not yet exist is `missing_managed_paths`'
    /// territory, not frozen's -- there is nothing on disk to be frozen.
    #[test]
    fn frozen_managed_paths_reports_nothing_when_the_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

        assert!(frozen_managed_paths(&files, dir.path()).is_empty());
    }

    /// The required negative control: a legitimately user-owned, unmarked
    /// scaffold-once file (`Cargo.toml`, `package.json`, gemspec, lockfiles)
    /// must never be reported frozen, even though it exists on disk without a
    /// marker -- exactly the shape a naive "unmarked file that looks
    /// generated" heuristic would misfire on. Getting this wrong would tell
    /// users to hand ownership of their own hand-edited files to alef.
    #[test]
    fn frozen_managed_paths_ignores_a_hand_written_scaffold_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        let files = vec![gen_file_unheadered("Cargo.toml", "[package]\nname = \"demo\"\n")];

        assert!(frozen_managed_paths(&files, dir.path()).is_empty());
    }

    /// A self-marking backend (custom Swift/Kotlin/Dart/Gleam/Zig headers,
    /// `docs::render`'s `.md` pages) bakes its literal header straight into
    /// `GeneratedFile::content` regardless of `generated_header`. The remedy
    /// must be read from that content, not reconstructed from the path -- a
    /// path-derived generic header would be the wrong text to hand back here.
    #[test]
    fn frozen_managed_paths_reads_the_remedy_from_self_marked_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Foo.swift"), "struct Foo {}\n").unwrap();
        let files = vec![gen_file_unheadered(
            "Foo.swift",
            "// Generated by alef. Do not edit by hand.\nstruct Foo {}\n",
        )];

        let frozen = frozen_managed_paths(&files, dir.path());

        assert_eq!(frozen.len(), 1);
        assert_eq!(
            frozen[0].remedy.as_deref(),
            Some("// Generated by alef. Do not edit by hand.")
        );
    }

    /// A managed path whose format has no comment syntax at all (`.json`)
    /// still gets reported frozen when `generated_header` claims ownership,
    /// but with no literal line to hand back -- there is nothing to paste in.
    #[test]
    fn frozen_managed_paths_reports_no_remedy_for_an_unmarkable_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("manifest.json"), "{}\n").unwrap();
        let files = vec![gen_file("manifest.json", "{}\n")];

        let frozen = frozen_managed_paths(&files, dir.path());

        assert_eq!(frozen.len(), 1);
        assert_eq!(frozen[0].remedy, None);
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

    /// Regression coverage for a hole reported against `alef verify`: "the inputs-hash
    /// covers generation inputs, not output bytes, so a dependency bumped inside a
    /// stamped, alef-generated manifest reports fresh." That does not hold for markable,
    /// stamped files -- `compute_file_hash` folds the file's own content into the embedded
    /// hash (see its doc and `core::hash`'s module doc), so a hand-edited dependency
    /// version -- e.g. `cargo upgrade --incompatible` bumping `base64` in place inside a
    /// generated JNI/FFI `Cargo.toml` -- is exactly the "content changed, inputs did not"
    /// case this test pins down: `inputs_hash` is identical before and after, only the
    /// on-disk bytes move. If `compute_file_hash`/`verify_walk` ever regress to hashing
    /// `inputs_hash` alone, this must start failing. ~keep
    #[test]
    fn verify_walk_detects_a_hand_edited_dependency_version_in_a_generated_manifest() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("Cargo.toml");
        let inputs_hash = "generation-inputs";
        let original = "# This file is auto-generated by alef — DO NOT EDIT.\n\
                         [dependencies]\n\
                         base64 = \"0.22\"\n";
        let hash = crate::core::hash::compute_file_hash(inputs_hash, original);
        let finalized = crate::core::hash::inject_hash_line(original, &hash);
        // Same generation inputs throughout -- only the on-disk bytes are hand-edited,
        // as `cargo upgrade --incompatible` would do to a generated manifest.
        std::fs::write(&path, finalized.replace("0.22", "0.23")).expect("hand-edit generated manifest");

        let stale = verify_walk(directory.path(), inputs_hash).expect("verify generated files");

        assert_eq!(
            stale.len(),
            1,
            "a hand-edited dependency version must be reported stale even though inputs_hash is unchanged"
        );
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

    fn stage_failure(paths: &[&str]) -> StageFailure {
        StageFailure {
            stage: "e2e",
            message: "56 e2e assertion(s) reference a field the availability oracle cannot resolve".to_owned(),
            paths: paths.iter().map(std::path::PathBuf::from).collect(),
        }
    }

    /// THE REGRESSION. `alef adopt packages/dart/rust/Cargo.toml` deadlocked because a
    /// pending e2e strict-assertion failure aborted `collect_managed_surface` before the
    /// ownership-only `Cargo.toml` target was ever considered, even though that target
    /// has no relationship to e2e. `affects_any` is the predicate that now lets `Commands::Adopt`
    /// tell the two cases apart: this asserts the tolerant half -- an e2e failure whose
    /// rendered paths are all snippet/test-app output must not be judged to affect an
    /// unrelated `Cargo.toml` target, whatever glob shape the operator typed. ~keep
    #[test]
    fn a_stage_failure_confined_to_e2e_paths_does_not_affect_an_unrelated_ownership_target() {
        let failure = stage_failure(&["e2e/python/test_smoke.py", "e2e/go/smoke_test.go"]);

        assert!(!failure.affects_any(&["packages/dart/rust/Cargo.toml".to_owned()]));
        assert!(!failure.affects_any(&["packages/**/*.gemspec".to_owned()]));
    }

    /// The control for the test above: when a requested target genuinely falls under the
    /// failing stage's own output, `affects_any` must say so, literal path or glob alike,
    /// so `alef adopt` still refuses to answer for a target it cannot render correctly
    /// rather than silently tolerating every e2e failure regardless of relevance.
    #[test]
    fn a_stage_failure_that_rendered_the_requested_target_does_affect_it() {
        let failure = stage_failure(&["e2e/python/test_smoke.py", "e2e/go/smoke_test.go"]);

        assert!(failure.affects_any(&["e2e/python/test_smoke.py".to_owned()]));
        assert!(failure.affects_any(&["e2e/python/*.py".to_owned()]));
        // Mixed: one target unrelated, one that matches -- still affects, because a
        // multi-target `alef adopt` run answers for every target it was given. ~keep
        assert!(failure.affects_any(&[
            "packages/dart/rust/Cargo.toml".to_owned(),
            "e2e/go/smoke_test.go".to_owned(),
        ]));
    }

    /// `alef verify` passes no targets at all -- every tolerated failure is unconditional
    /// debt for a read-only report, never excused by "no target asked for it". An empty
    /// `targets` slice must therefore never affect anything, which is the same fact
    /// `Commands::Adopt` relies on for a target list that turned out to filter down to
    /// nothing upstream.
    #[test]
    fn a_stage_failure_never_affects_an_empty_target_list() {
        let failure = stage_failure(&["e2e/python/test_smoke.py"]);

        assert!(!failure.affects_any(&[]));
    }
}
