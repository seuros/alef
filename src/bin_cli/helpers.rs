use anyhow::{Context, Result};

mod post_build;
pub(crate) use post_build::languages_have_post_build_steps;
use post_build::run_required_post_builds;

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
/// `alef:hash:<hex>` stamp. Skips build/cache directories, every directory git considers
/// ignored (see [`super::verify_gitignore::gitignored_dirs`]), and files without the Alef
/// ownership marker. Shared by [`verify_walk`] and [`verify_walk_multi`] so both see the same
/// file set, and by [`super::verify_orphans::find_orphaned_generated_files`] so the orphan
/// check sees the identical disk-side file set too. ~keep
pub(crate) fn collect_alef_hashes(base_dir: &std::path::Path) -> Vec<(std::path::PathBuf, Option<String>, String)> {
    let mut found = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![base_dir.to_path_buf()];
    // A gitignored dependency-fetch cache (a zig package manager's local package cache) or
    // build-output directory (wasm-pack's own `pkg/`, which copies the crate's real,
    // alef-marked README into a tree this walk otherwise has no reason to open) is neither
    // this run's generated output nor its generation input -- it must never be read as either.
    // See that module's doc for the incident this closes. ~keep
    let gitignored_dirs = super::verify_gitignore::gitignored_dirs(base_dir);

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
                let pruned_as_gitignored = path
                    .strip_prefix(base_dir)
                    .is_ok_and(|relative| gitignored_dirs.contains(relative));
                if VERIFY_SKIP_DIRS.contains(&name) || pruned_as_dotfile || pruned_as_gitignored {
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
/// For a format [`crate::cli::pipeline::marker_comment_style`] has no comment syntax for
/// (`.json`, `DESCRIPTION`, a pre-widening `.clang-format`), a missing marker is not by
/// itself evidence of foreign authorship — [`crate::cli::pipeline::is_owned_by_ownership_record`]
/// is consulted exactly as `write_files_report`'s and `write_scaffold_files_report`'s write
/// guards consult it, so this report agrees with what those guards would actually accept.
/// Before this fell back to the marker check alone, a file the write guard would happily
/// (re)write on the strength of its committed `.alef-ownership.toml` record — including one
/// `alef adopt` or a delete-and-regenerate had just recorded — stayed reported "frozen"
/// forever, because this function never looked at the record the guard relies on. ~keep
///
/// The remedy text is read straight from the in-memory `GeneratedFile::content`
/// first, because a self-marking backend (custom Swift/Kotlin/Dart/Gleam/Zig
/// headers, `docs::render`'s HTML-commented `.md` pages) already bakes its
/// literal header into `content` regardless of `generated_header`. Only when
/// that content carries no marker yet — the common case, where the header is
/// added later by `write_files_report`'s `ensure_generated_header` pass — does
/// this fall back to reconstructing it from the path via
/// [`crate::cli::pipeline::provenance_header_for_path`]. ~keep
///
/// Runs over two candidate sets, not only [`crate::cli::pipeline::managed_generated_files`]'s
/// marker-carrying subset: `carries_alef_marker()` is `generated_header ||
/// content_has_alef_marker`, so a file emitted with `generated_header: false` whose content
/// embeds no marker at all — the PHP backend's `config.m4`
/// (`backends::php::gen_bindings::rust_items::generate_config_m4`) is the shipped case —
/// never reaches the ownership-record fallback a few lines below, even though
/// `write_files_report`'s guard already refuses to overwrite that exact path once it exists
/// without a committed `.alef-ownership.toml` record. [`unmarkable_unclaimed_files`] recovers
/// that second set: it is deliberately narrower than "every `generated_header: false` file" —
/// see its own doc for why only the genuinely unmarkable ones qualify. ~keep
fn frozen_managed_paths(files: &[crate::core::backend::GeneratedFile], base_dir: &std::path::Path) -> Vec<FrozenFile> {
    crate::cli::pipeline::managed_generated_files(files)
        .into_iter()
        .chain(unmarkable_unclaimed_files(files, base_dir))
        .filter_map(|file| {
            let full_path = base_dir.join(&file.path);
            let existing = std::fs::read_to_string(&full_path).ok()?;
            if crate::core::hash::content_has_alef_marker(&existing) {
                return None;
            }
            let is_markable = crate::cli::pipeline::marker_comment_style(&full_path).is_some();
            if !is_markable && crate::cli::pipeline::is_owned_by_ownership_record(base_dir, &full_path) {
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

/// Every file in `files` that [`crate::cli::pipeline::managed_generated_files`] excludes
/// (`carries_alef_marker()` is false — no `generated_header: true` claim, no marker baked
/// into `content`) but that is genuinely incapable of ever carrying one
/// ([`crate::cli::pipeline::marker_comment_style`] answers `None` for its path).
///
/// Scoped this narrowly on purpose: widening it to every `generated_header: false` file
/// would also pull in a markable file a backend simply forgot to self-mark, which
/// `write_files_report`'s guard treats differently — a markable path with no marker is
/// refused regardless of any ownership record (see that function's `owned` computation),
/// so folding it into this ownership-record-checked set would wrongly clear it once a
/// record existed. Only the genuinely unmarkable subset is where alef's write guard has
/// ever accepted an ownership record as proof, and this mirrors exactly that. ~keep
fn unmarkable_unclaimed_files(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
) -> Vec<crate::core::backend::GeneratedFile> {
    files
        .iter()
        .filter(|file| {
            !file.carries_alef_marker()
                && crate::cli::pipeline::marker_comment_style(&base_dir.join(&file.path)).is_none()
        })
        .cloned()
        .collect()
}

/// Missing and frozen generated files found for one crate — see
/// [`find_missing_and_frozen_generated_files`].
#[derive(Default)]
pub(crate) struct MissingAndFrozenFiles {
    pub(crate) missing: Vec<String>,
    pub(crate) frozen: Vec<FrozenFile>,
    /// Absolute paths of every file this crate's configuration would produce this run, from the
    /// same `surface` `missing`/`frozen` are derived from -- deliberately every path, not only
    /// [`crate::cli::pipeline::managed_output_paths`]'s marker-carrying subset: a self-marking
    /// backend (pyo3's `lib.rs`, `docs::render`'s HTML-commented pages) bakes its header into
    /// `content` at write time via `ensure_generated_header`, not at this in-memory render, so
    /// `carries_alef_marker()` reads `false` here for a file that is unquestionably still
    /// produced this run -- filtering by it would misreport that file as an orphan the moment it
    /// left the marker-carrying subset. A multi-crate caller unions this across every crate
    /// before handing it to [`super::verify_orphans::find_orphaned_generated_files`], so a file
    /// legitimately owned by crate B is never mistaken for an orphan while diffing crate A alone. ~keep
    pub(crate) managed_paths: std::collections::HashSet<std::path::PathBuf>,
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
    // `surface` alone omits post-build-owned paths; see `verify_orphans::post_build_owned_paths`. ~keep
    let mut managed_paths: std::collections::HashSet<std::path::PathBuf> =
        surface.iter().map(|file| base_dir.join(&file.path)).collect();
    managed_paths.extend(super::verify_orphans::post_build_owned_paths(
        languages, config, base_dir,
    ));
    let mut result = MissingAndFrozenFiles {
        missing: missing_managed_paths(&surface, base_dir),
        frozen: frozen_managed_paths(&surface, base_dir),
        managed_paths,
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
    // superset of it.
    //
    // `clean: true`, not `false`: `pipeline::generate`'s own `clean` parameter gates a
    // language-level cache short-circuit (`!clean && cache::is_lang_cached(..)` in
    // `generate/generation.rs`) that returns zero files for a language whose lang-hash
    // already matches the on-disk `.alef/<crate>/hashes/<lang>.hash` -- exactly the state
    // every language is in immediately after the `alef generate` run that wrote it, which is
    // the single most common moment `alef verify` runs. `clean: false` here silently dropped
    // every one of that language's bindings-stage files from `surface` in precisely that
    // steady state, which `missing_managed_paths`/`frozen_managed_paths` tolerate (an absent
    // `surface` entry just means that path is not checked) but which made every one of that
    // language's self-marking bindings files -- `packages/python/lib.rs` and its pyo3
    // equivalents on every other self-marking backend -- a guaranteed, permanent orphan false
    // positive on exactly the run `alef verify` is normally expected to pass. `clean: true`
    // forces full in-memory regeneration regardless of cache state, matching this function's
    // own contract: `surface` is "what this run's backends would produce," not "what they
    // would bother producing given whatever happens to already be cached." ~keep
    let bindings_stage: Stage<'_> = Box::new(|| {
        let mut files = Vec::new();
        for (_, produced) in crate::cli::pipeline::generate(api, config, languages, true, config_path, false)? {
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
        let (files, generator_error) = crate::e2e::generate_e2e(
            config,
            e2e_config,
            None,
            &api.types,
            &api.enums,
            &api.functions,
            &api.errors,
        )
        .context("failed to render the e2e stage of alef's managed output")?;
        Ok(stage_failure_for("e2e", generator_error, files))
    });
    let e2e_registry_stage: Stage<'_> = Box::new(|| {
        let Some(e2e_config) = &config.e2e else {
            return Ok((Vec::new(), Vec::new()));
        };
        let mut registry_config = e2e_config.clone();
        registry_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
        let (files, generator_error) = crate::e2e::generate_e2e(
            config,
            &registry_config,
            None,
            &api.types,
            &api.enums,
            &api.functions,
            &api.errors,
        )
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
/// `.alef/`, `parsers/`, `dist/`, `vendor/`, `.git/`), and every directory git considers
/// ignored (see [`super::verify_gitignore::gitignored_dirs`]) — a consumer's own
/// dependency-fetch cache or build-output directory is neither this run's generated output
/// nor its generation input — so verify stays fast and never claims foreign content. Files
/// without the alef header marker are skipped silently — those are user-owned (scaffold-once
/// Cargo.toml templates, composer.json, gemspec, package.json, lockfiles, etc.) and alef has
/// no claim.
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
mod tests;
