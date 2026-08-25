use crate::snippets::discovery::discover_snippets;
use crate::snippets::error::Result;
use crate::snippets::gap_coverage::GapCoverage;
use crate::snippets::parser;
use crate::snippets::types::{Language, Snippet, SnippetAnnotationKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Default)]
pub struct GapConfig {
    pub docs_dirs: Vec<PathBuf>,
    pub snippet_dirs: Vec<PathBuf>,
    pub required_languages: Vec<Language>,
    /// Additional base paths searched when resolving MkDocs `--8<--` include targets.
    ///
    /// Mirrors the `pymdownx.snippets` `base_path` list. Each target is resolved
    /// against these paths in order; the first match wins. Falls back to
    /// `docs_dir.join(target)` when the list is empty or no path matches. ~keep
    pub include_base_paths: Vec<PathBuf>,
    pub configured_references: Vec<PathBuf>,
    pub exclude: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetReference {
    pub source: PathBuf,
    pub target: PathBuf,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingLanguageVariant {
    pub group: PathBuf,
    pub language: Language,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetLocation {
    pub path: PathBuf,
    pub line: usize,
    pub block_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownLanguage {
    pub path: PathBuf,
    pub line: usize,
    pub tag: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapReport {
    pub missing_references: Vec<SnippetReference>,
    pub unreferenced_snippets: Vec<PathBuf>,
    pub missing_language_variants: Vec<MissingLanguageVariant>,
    pub skips_without_reason: Vec<SnippetLocation>,
    pub unknown_languages: Vec<UnknownLanguage>,
    /// What this run actually compared. Deliberately not part of [`Self::has_gaps`]: coverage
    /// is context for the verdict, never a finding of its own. ~keep
    #[serde(default)]
    pub coverage: GapCoverage,
}

impl GapReport {
    #[must_use]
    pub fn has_gaps(&self) -> bool {
        !self.missing_references.is_empty()
            || !self.unreferenced_snippets.is_empty()
            || !self.missing_language_variants.is_empty()
            || !self.skips_without_reason.is_empty()
            || !self.unknown_languages.is_empty()
    }

    /// Findings that are never intentional: missing include targets, missing required language
    /// variants, undocumented skips, and unknown fence languages.
    ///
    /// Deliberately excludes [`Self::unreferenced_snippets`] -- an extra hand-authored example
    /// with no include target can be deliberate, so whether it counts as a failure is left to
    /// [`Self::is_failure`]'s `strict` argument rather than folded in here unconditionally.
    #[must_use]
    pub fn has_structural_gaps(&self) -> bool {
        !self.missing_references.is_empty()
            || !self.missing_language_variants.is_empty()
            || !self.skips_without_reason.is_empty()
            || !self.unknown_languages.is_empty()
    }

    /// Whether this report should fail a run.
    ///
    /// Structural gaps ([`Self::has_structural_gaps`]) always fail. Unreferenced snippets only
    /// fail when `strict` is set. `alef snippets check` and `alef snippets gaps` both report the
    /// same [`GapReport`] shape and must reach the same verdict from it for the same `strict`
    /// setting -- before this method existed each command re-derived its own combination, and
    /// `gaps` failed unconditionally on ANY finding (including an unreferenced-only one) while
    /// `check` already gated that one finding class on `strict`. ~keep
    #[must_use]
    pub fn is_failure(&self, strict: bool) -> bool {
        self.has_structural_gaps() || (strict && !self.unreferenced_snippets.is_empty())
    }
}

/// Build a report for common documentation snippet coverage gaps.
///
/// # Errors
///
/// Returns an error when snippets or markdown files cannot be read.
pub fn detect_gaps(config: &GapConfig) -> Result<GapReport> {
    let snippets: Vec<_> = discover_snippets(&config.snippet_dirs, None)?
        .into_iter()
        .filter(|snippet| !is_excluded(&snippet.path, &config.exclude))
        .collect();
    let (discovered, docs_pages_scanned, mkdocs_include_references) =
        discover_includes_measured(&config.docs_dirs, &config.include_base_paths)?;
    let mut references: Vec<_> = discovered
        .into_iter()
        .filter(|reference| !is_excluded(&reference.source, &config.exclude))
        .collect();
    let include_references = references.len();
    references.extend(config.configured_references.iter().map(|target| SnippetReference {
        source: target.clone(),
        target: target.clone(),
        line: 1,
    }));
    let snippet_files = snippet_files(&snippets);
    let expectations = ledger_expectations(&config.snippet_dirs)?;
    let (missing_language_variants, language_groups) =
        missing_language_variants(&snippets, &config.required_languages, &expectations);

    Ok(GapReport {
        missing_references: missing_references(&references),
        unreferenced_snippets: unreferenced_snippets(&snippet_files, &references),
        missing_language_variants,
        skips_without_reason: skips_without_reason(&snippets),
        unknown_languages: unknown_languages(&config.snippet_dirs)?
            .into_iter()
            .filter(|unknown| !is_excluded(&unknown.path, &config.exclude))
            .collect(),
        coverage: GapCoverage {
            snippet_roots: config.snippet_dirs.len(),
            snippets_discovered: snippet_files.len(),
            docs_roots: config.docs_dirs.len(),
            docs_pages_scanned,
            include_references,
            mkdocs_include_references,
            configured_references: config.configured_references.len(),
            required_languages: config.required_languages.len(),
            language_groups,
            include_base_paths: config.include_base_paths.len(),
        },
    })
}

/// Resolve snippet paths named by `[crates.readme.languages.*].snippets`.
#[must_use]
pub fn readme_snippet_references(
    workspace_root: &Path,
    readme: Option<&crate::core::config::ReadmeConfig>,
) -> Vec<PathBuf> {
    let Some(readme) = readme else {
        return Vec::new();
    };
    let mut references = Vec::new();
    for (language, entry) in &readme.languages {
        let snippets_dir = entry
            .get("snippets_dir")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .or_else(|| readme.snippets_dir.clone());
        let source_language = entry
            .get("snippet_language")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(language);
        if let Some(snippets) = entry.get("snippets") {
            collect_readme_snippet_mappings(snippets, &mut |path, mapping_root| {
                let path = normalize_readme_snippet_path(path, language, source_language);
                if let Some(root) = mapping_root.map(PathBuf::from).or_else(|| snippets_dir.clone()) {
                    references.push(normalize_path(&workspace_root.join(root).join(path)));
                }
            });
        }
    }
    references.sort();
    references.dedup();
    references
}

/// Whether a coverage ledger that records missing fixture/language cells is
/// itself an error, or merely an incomplete ledger whose recorded paths are
/// still usable as references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingCells {
    Reject,
    Tolerate,
}

/// Resolve generated snippet paths recorded by current coverage ledgers.
///
/// Snippet roots without a ledger are left alone so ordinary documentation files still
/// participate in orphan detection.
///
/// # Errors
///
/// Returns an error when a discovered ledger is unreadable, stale, incomplete, or names
/// an invalid or missing generated file.
pub fn coverage_ledger_references(snippet_dirs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    collect_coverage_ledger_references(snippet_dirs, MissingCells::Reject)
}

/// Resolve generated snippet paths exactly like [`coverage_ledger_references`],
/// but accept a ledger that records missing fixture/language cells.
///
/// Callers that already surface missing cells through their own gate — `alef
/// snippets check` warns about them and only fails under `strict` — would
/// otherwise turn every incomplete coverage manifest into an unconditional
/// failure attributed to reference resolution.
///
/// # Errors
///
/// Returns an error when a discovered ledger is unreadable, stale, or names an
/// invalid or missing generated file. Only the missing-cell case is tolerated. ~keep
pub fn coverage_ledger_references_allowing_missing_cells(snippet_dirs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    collect_coverage_ledger_references(snippet_dirs, MissingCells::Tolerate)
}

/// Every `.alef-snippet-coverage.json` ledger found beneath `snippet_root`, sorted for stable
/// ordering. Shared by every caller that needs to locate ledgers before reading them, so the
/// walk semantics (symlinks never followed, one error format) live in exactly one place. ~keep
fn find_coverage_manifests(snippet_root: &Path) -> Result<Vec<PathBuf>> {
    let mut manifests = WalkDir::new(snippet_root)
        .follow_links(false)
        .into_iter()
        .map(|entry| {
            entry.map_err(|error| {
                crate::snippets::error::Error::Other(format!(
                    "walking snippet root {} for coverage ledgers: {error}",
                    snippet_root.display()
                ))
            })
        })
        .filter_map(|entry| match entry {
            Ok(entry)
                if entry.file_type().is_file() && entry.file_name() == crate::e2e::snippets::COVERAGE_MANIFEST =>
            {
                Some(Ok(entry.into_path()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>>>()?;
    manifests.sort();
    Ok(manifests)
}

fn manifest_output_root(manifest: &Path) -> Result<&Path> {
    manifest.parent().ok_or_else(|| {
        crate::snippets::error::Error::Other(format!("coverage ledger has no output root: {}", manifest.display()))
    })
}

fn collect_coverage_ledger_references(snippet_dirs: &[PathBuf], missing_cells: MissingCells) -> Result<Vec<PathBuf>> {
    let mut references = Vec::new();
    for snippet_root in snippet_dirs {
        for manifest in find_coverage_manifests(snippet_root)? {
            let output_root = manifest_output_root(&manifest)?;
            references.extend(read_coverage_ledger_references(output_root, &manifest, missing_cells)?);
        }
    }
    references.sort();
    references.dedup();
    Ok(references)
}

/// Which languages the e2e snippet pipeline actually expected for each fixture, and which
/// fixture generated each tracked snippet file.
///
/// Read from every coverage ledger beneath the configured snippet roots. A fixture/language
/// cell either `function_excluded_for_language` or `function_binding_excluded_for_language`
/// drops never enters `expected` in the first place -- see `e2e::snippets::mod`'s `~keep`
/// comment above `coverage.expected.push`. `function_excluded_for_language` reuses
/// [`crate::docs::language_pages::excludes::language_excludes`], which folds the crate-wide
/// `[crates.exclude].functions` list together with every per-language
/// `[crates.<lang>].exclude_functions` list `language_excludes` has an arm for (including the
/// FFI-derived families' own `[crates.ffi].exclude_functions` union) -- it only ever checks
/// function names, never the type half of `language_excludes`'s return value, so `[opaque_types]`
/// is still not consulted here. `function_binding_excluded_for_language` closes the gap this
/// comment used to document: it reads `#[alef::skip]`/`#[doc(hidden)]` (the `binding_excluded` IR
/// flag, which applies uniformly across every non-Rust language) directly off the IR, so a
/// `binding_excluded` function or method is now also absent from `expected` before this reader
/// ever sees the ledger. Reusing `language_excludes` here, rather than re-deriving which
/// functions are excluded from `alef.toml` a second time, is what keeps the language-parity
/// check from disagreeing with the generator that produced the very tree it is checking. ~keep
#[derive(Debug, Clone, Default)]
struct LedgerExpectations {
    expected_by_fixture: BTreeMap<String, BTreeSet<Language>>,
    fixture_by_path: BTreeMap<PathBuf, String>,
    /// Every language that appears in `expected` for *any* fixture, project-wide.
    ///
    /// A single fixture's `expected` set cannot by itself tell "this language was excluded for
    /// this function" apart from "this language is not one the project's e2e generation covers
    /// at all" -- both look like "language absent from this fixture's expected set". Only the
    /// first case is a false positive; the second is a language `required_languages` compares
    /// that e2e generation never touches (a hand-authored addition on top of generated
    /// snippets), and suppressing it there would silently zero out the parity check for that
    /// entire language across the whole project. Checking this project-wide set first tells the
    /// two apart: a language absent here was never covered by the ledger to begin with, so the
    /// per-fixture absence carries no information and the finding must stand. ~keep
    covered_languages: BTreeSet<Language>,
}

impl LedgerExpectations {
    /// The languages a tracked group is expected to provide, or `None` when no ledger tracks
    /// any snippet in the group -- a hand-authored tree the e2e pipeline never generated, for
    /// which the required-languages list itself remains the only source of truth.
    fn expected_languages(&self, group_fixture: Option<&str>) -> Option<&BTreeSet<Language>> {
        self.expected_by_fixture.get(group_fixture?)
    }

    /// Whether a missing `(group, language)` pair is explained by an intentional per-fixture
    /// exclusion rather than a real gap: the group is ledger-tracked, the language is one the
    /// ledger covers somewhere in the project, and this fixture's own `expected` set does not
    /// include it.
    fn excludes(&self, group_fixture: Option<&str>, language: Language) -> bool {
        self.covered_languages.contains(&language)
            && self
                .expected_languages(group_fixture)
                .is_some_and(|expected| !expected.contains(&language))
    }
}

fn ledger_expectations(snippet_dirs: &[PathBuf]) -> Result<LedgerExpectations> {
    let mut expectations = LedgerExpectations::default();
    for snippet_root in snippet_dirs {
        for manifest in find_coverage_manifests(snippet_root)? {
            let output_root = manifest_output_root(&manifest)?;
            let content = std::fs::read_to_string(&manifest)?;
            let ledger: crate::e2e::snippets::SnippetCoverageLedger = serde_json::from_str(&content)?;
            for key in &ledger.expected {
                let language = Language::from_session_target(&key.language);
                if language == Language::Unknown {
                    continue;
                }
                expectations
                    .expected_by_fixture
                    .entry(key.fixture_id.clone())
                    .or_default()
                    .insert(language);
                expectations.covered_languages.insert(language);
            }
            for metadata in &ledger.generated_metadata {
                if let Ok(path) = crate::e2e::snippets::ledger_paths::resolve_tracked_path(output_root, &metadata.path)
                {
                    expectations
                        .fixture_by_path
                        .insert(path, metadata.key.fixture_id.clone());
                }
            }
        }
    }
    Ok(expectations)
}

/// Resolve every snippet beneath an Astro content collection root when that
/// collection is queried from the configured documentation tree. ~keep
pub fn astro_collection_references(
    docs_dirs: &[PathBuf],
    collections: &BTreeMap<String, PathBuf>,
) -> Result<Vec<PathBuf>> {
    let mut referenced_collections = BTreeSet::new();
    for docs_dir in docs_dirs {
        for path in markdown_files(docs_dir) {
            let content = std::fs::read_to_string(&path)?;
            referenced_collections.extend(parse_astro_collection_queries(&content));
        }
    }

    let mut references = Vec::new();
    for collection in referenced_collections {
        let Some(root) = collections.get(&collection) else {
            continue;
        };
        references.extend(
            WalkDir::new(root)
                .follow_links(false)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.file_type().is_file())
                .map(walkdir::DirEntry::into_path),
        );
    }
    references.sort();
    references.dedup();
    Ok(references)
}

#[must_use]
pub fn parse_astro_collection_queries(content: &str) -> BTreeSet<String> {
    let mut collections = BTreeSet::new();
    for quote in ['"', '\''] {
        let marker = format!("getCollection({quote}");
        let mut remainder = content;
        while let Some((_, after_marker)) = remainder.split_once(&marker) {
            if let Some((name, after_name)) = after_marker.split_once(quote) {
                collections.insert(name.to_string());
                remainder = after_name;
            } else {
                break;
            }
        }
    }
    collections
}

fn read_coverage_ledger_references(
    output_root: &Path,
    manifest: &Path,
    missing_cells: MissingCells,
) -> Result<Vec<PathBuf>> {
    let content = std::fs::read_to_string(manifest)?;
    let ledger: crate::e2e::snippets::SnippetCoverageLedger = serde_json::from_str(&content)?;
    crate::e2e::snippets::coverage::validate(&ledger)
        .map_err(|error| crate::snippets::error::Error::Other(format!("invalid coverage ledger: {error:#}")))?;
    if missing_cells == MissingCells::Reject && !ledger.missing.is_empty() {
        return Err(crate::snippets::error::Error::Other(format!(
            "incomplete fixture-snippet coverage manifest at {}",
            manifest.display()
        )));
    }
    ledger
        .generated_paths
        .into_iter()
        .map(|relative| {
            let path = crate::e2e::snippets::ledger_paths::resolve_tracked_path(output_root, &relative)?;
            if !path.is_file() {
                return Err(crate::snippets::error::Error::Other(format!(
                    "fixture snippet recorded by the coverage ledger is missing: {}",
                    path.display()
                )));
            }
            Ok(path)
        })
        .collect()
}

fn collect_readme_snippet_mappings(value: &serde_json::Value, collect: &mut impl FnMut(&str, Option<&str>)) {
    match value {
        serde_json::Value::String(path) => collect(path, None),
        serde_json::Value::Array(values) => {
            for value in values {
                collect_readme_snippet_mappings(value, collect);
            }
        }
        serde_json::Value::Object(values) => {
            if let Some(path) = values.get("path").and_then(serde_json::Value::as_str) {
                collect(path, values.get("root").and_then(serde_json::Value::as_str));
            } else {
                for value in values.values() {
                    collect_readme_snippet_mappings(value, collect);
                }
            }
        }
        _ => {}
    }
}

fn normalize_readme_snippet_path(path: &str, language: &str, source_language: &str) -> PathBuf {
    let path = Path::new(path);
    let mut components = path.components();
    let first = components.next();
    let has_language_prefix = first
        .and_then(|component| component.as_os_str().to_str())
        .is_some_and(|component| component == language || Language::from_dir_name(component) != Language::Unknown);
    let remainder = if has_language_prefix {
        components.as_path()
    } else {
        path
    };
    Path::new(source_language).join(remainder)
}

fn is_excluded(path: &Path, exclude: &[PathBuf]) -> bool {
    exclude.iter().any(|excluded| path.starts_with(excluded))
}

/// Discover MkDocs `--8<-- "path"` include references beneath documentation roots.
///
/// `include_base_paths` mirrors the `pymdownx.snippets` `base_path` list. Each
/// target is resolved against those paths in order; the first match wins. When
/// empty or no path matches, falls back to `docs_dir.join(target)`.
///
/// # Errors
///
/// Returns an error when a markdown file cannot be read.
pub fn discover_includes(docs_dirs: &[PathBuf], include_base_paths: &[PathBuf]) -> Result<Vec<SnippetReference>> {
    Ok(discover_includes_measured(docs_dirs, include_base_paths)?.0)
}

/// [`discover_includes`], also reporting how many documentation pages were opened and how many
/// of the references found were MkDocs `--8<--` targets specifically.
///
/// The page count is the only honest answer to "did the include check look at anything?": a
/// configured docs root holding no markdown, or holding markdown the walk filters out, yields
/// zero references for the same reason an unconfigured root does, and the reference count
/// alone cannot tell the two apart. Measured here rather than by a second walk so the number
/// describes the walk the findings came from. ~keep
///
/// The `--8<--` count is split out from the combined reference total because only `--8<--`
/// targets resolve through `include_base_paths` -- an Astro/MDX content import never does (see
/// [`parse_mdx_content_imports`]). A docs tree that is all MDX imports and zero `--8<--` targets
/// can have a large combined reference count while `include_base_paths` is entirely
/// irrelevant to it; gating the unset-`include_base_paths` warning on this narrower count keeps
/// that warning quiet for exactly that tree. ~keep
fn discover_includes_measured(
    docs_dirs: &[PathBuf],
    include_base_paths: &[PathBuf],
) -> Result<(Vec<SnippetReference>, usize, usize)> {
    let mut references = Vec::new();
    let mut pages_scanned = 0;
    let mut mkdocs_include_references = 0;
    for docs_dir in docs_dirs {
        for path in markdown_files(docs_dir) {
            let content = std::fs::read_to_string(&path)?;
            pages_scanned += 1;
            let mkdocs_includes = parse_includes(&content, &path, docs_dir, include_base_paths);
            mkdocs_include_references += mkdocs_includes.len();
            references.extend(mkdocs_includes);
            references.extend(parse_mdx_content_imports(&content, &path));
        }
    }
    references.sort_by(|left, right| left.source.cmp(&right.source).then(left.line.cmp(&right.line)));
    Ok((references, pages_scanned, mkdocs_include_references))
}

/// Resolve a single include `target` string against the provided base paths.
///
/// Returns the first candidate path that exists on disk, or falls back to
/// `docs_dir.join(target)` so that the missing-references report still points
/// to a real candidate when nothing resolves.
#[must_use]
fn resolve_include_target(target: &str, docs_dir: &Path, include_base_paths: &[PathBuf]) -> PathBuf {
    for base in include_base_paths {
        let candidate = base.join(target);
        if candidate.exists() {
            return candidate;
        }
    }
    docs_dir.join(target)
}

#[must_use]
pub fn parse_includes(
    content: &str,
    source: &Path,
    docs_dir: &Path,
    include_base_paths: &[PathBuf],
) -> Vec<SnippetReference> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| parse_include_target(line).map(|target| (index, target)))
        .map(|(index, target)| SnippetReference {
            source: source.to_path_buf(),
            target: resolve_include_target(target, docs_dir, include_base_paths),
            line: index + 1,
        })
        .collect()
}

pub fn parse_include_target(line: &str) -> Option<&str> {
    let marker = "--8<--";
    let after_marker = line.trim().strip_prefix(marker)?.trim();
    let quoted = after_marker.strip_prefix('"')?;
    let end = quoted.find('"')?;
    Some(&quoted[..end])
}

/// Discover Astro/MDX `import { Content as X } from "..."` snippet references.
///
/// The consumer docs site (an Astro Starlight project) does not use MkDocs'
/// `--8<--` include syntax at all — every `.mdx` guide pulls a snippet's
/// rendered content in via a named import of the snippet file, e.g.:
///
/// ```text
/// import { Content as Snip_cli_install_cargo } from "../../../snippets/cli/install_cargo.md";
/// ```
///
/// Unlike a MkDocs include target (resolved against `docs_dir` /
/// `include_base_paths`), an ES module import path is always resolved
/// relative to the importing file's own directory, so this function
/// deliberately does not take `include_base_paths`. ~keep
#[must_use]
pub fn parse_mdx_content_imports(content: &str, source: &Path) -> Vec<SnippetReference> {
    let Some(source_dir) = source.parent() else {
        return Vec::new();
    };
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| parse_mdx_content_import_target(line).map(|target| (index, target)))
        .map(|(index, target)| SnippetReference {
            source: source.to_path_buf(),
            target: normalize_path(&source_dir.join(target)),
            line: index + 1,
        })
        .collect()
}

/// Extract the import path from a single `import { Content as X } from "...";`
/// line, or `None` if the line does not match that shape.
///
/// Recognizes both the specific `Content as <ident>` alias form actually used
/// by the docs site and the bare `import { Content } from "...";` form (no
/// alias), single- or double-quoted, with or without a trailing semicolon.
pub fn parse_mdx_content_import_target(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let after_import = trimmed.strip_prefix("import")?.trim_start();
    let after_brace = after_import.strip_prefix('{')?;
    let close_brace = after_brace.find('}')?;
    let binding = after_brace[..close_brace].trim();
    let is_content_import = binding == "Content" || binding.starts_with("Content ") && binding.contains(" as ");
    if !is_content_import {
        return None;
    }
    let after_close = after_brace[close_brace + 1..].trim_start();
    let after_from = after_close.strip_prefix("from")?.trim_start();
    let quote = after_from.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &after_from[1..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

/// Collapse `.`/`..` path components produced by joining a relative import
/// target onto its importing file's directory, without touching the
/// filesystem (the target may not exist yet, which is exactly the case
/// `missing_references` needs to detect). ~keep
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn markdown_files(base: &Path) -> Vec<PathBuf> {
    if !base.exists() {
        return Vec::new();
    }

    let mut files: Vec<PathBuf> = WalkDir::new(base)
        .follow_links(true)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| matches!(extension.to_lowercase().as_str(), "astro" | "md" | "markdown" | "mdx"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

fn snippet_files(snippets: &[Snippet]) -> BTreeSet<PathBuf> {
    snippets.iter().map(|snippet| snippet.path.clone()).collect()
}

fn missing_references(references: &[SnippetReference]) -> Vec<SnippetReference> {
    references
        .iter()
        .filter(|reference| !reference.target.exists())
        .cloned()
        .collect()
}

fn unreferenced_snippets(snippet_files: &BTreeSet<PathBuf>, references: &[SnippetReference]) -> Vec<PathBuf> {
    let referenced: BTreeSet<PathBuf> = references
        .iter()
        .filter(|reference| reference.target.exists())
        .map(|reference| reference.target.clone())
        .collect();
    snippet_files.difference(&referenced).cloned().collect()
}

/// The missing variants, and the number of snippet groups compared to find them.
///
/// The group count is returned because an empty finding list has two very different causes: no
/// group is missing a language, or no group was found at all. A group key exists only for a
/// snippet path carrying a recognised `{language}` directory component, so a tree laid out any
/// other way produces zero groups and a silently empty result. ~keep
fn missing_language_variants(
    snippets: &[Snippet],
    required_languages: &[Language],
    expectations: &LedgerExpectations,
) -> (Vec<MissingLanguageVariant>, usize) {
    if required_languages.is_empty() {
        return (Vec::new(), 0);
    }

    let mut groups: BTreeMap<PathBuf, BTreeSet<Language>> = BTreeMap::new();
    let mut group_fixture: BTreeMap<PathBuf, &str> = BTreeMap::new();
    for snippet in snippets {
        let Some(group) = language_group(&snippet.path, snippet.language) else {
            continue;
        };
        if let Some(fixture_id) = expectations.fixture_by_path.get(&snippet.path) {
            group_fixture.entry(group.clone()).or_insert(fixture_id.as_str());
        }
        groups.entry(group).or_default().insert(snippet.language);
    }

    let group_count = groups.len();
    let mut missing = Vec::new();
    for (group, languages) in groups {
        let fixture = group_fixture.get(&group).copied();
        for language in required_languages {
            if languages.contains(language) {
                continue;
            }
            // A ledger-tracked fixture that never expected this language for e2e generation --
            // dropped by `exclude_functions` or any surface it folds in -- has no gap here: the
            // variant was never going to exist. A group no ledger tracks at all (hand-authored,
            // or generated by a run with no coverage manifest), or a language the ledger never
            // covers for any fixture, keeps the original behaviour, since `required_languages`
            // is the only source of truth available for it. ~keep
            if expectations.excludes(fixture, *language) {
                continue;
            }
            missing.push(MissingLanguageVariant {
                group: group.clone(),
                language: *language,
            });
        }
    }
    (missing, group_count)
}

fn language_group(path: &Path, language: Language) -> Option<PathBuf> {
    let mut group = PathBuf::new();
    let mut replaced = false;

    for component in path.components() {
        let text = component.as_os_str().to_str()?;
        if !replaced && Language::from_dir_name(text) == language {
            group.push("{language}");
            replaced = true;
        } else {
            group.push(text);
        }
    }

    replaced.then_some(group)
}

fn skips_without_reason(snippets: &[Snippet]) -> Vec<SnippetLocation> {
    snippets
        .iter()
        .filter(|snippet| {
            snippet
                .annotation
                .as_ref()
                .map(|annotation| {
                    annotation.kind == SnippetAnnotationKind::Skip
                        && annotation.reason.as_deref().unwrap_or_default().is_empty()
                })
                .unwrap_or(false)
        })
        .map(|snippet| SnippetLocation {
            path: snippet.path.clone(),
            line: snippet.start_line,
            block_index: snippet.block_index,
        })
        .collect()
}

fn unknown_languages(snippet_dirs: &[PathBuf]) -> Result<Vec<UnknownLanguage>> {
    let mut unknown = Vec::new();
    for dir in snippet_dirs {
        for path in markdown_files(dir) {
            for block in parser::parse_code_blocks(&path)? {
                if Language::from_fence_tag(&block.lang) == Language::Unknown {
                    unknown.push(UnknownLanguage {
                        path: path.clone(),
                        line: block.start_line,
                        tag: block.lang,
                    });
                }
            }
        }
    }
    unknown.sort_by(|left, right| left.path.cmp(&right.path).then(left.line.cmp(&right.line)));
    Ok(unknown)
}

#[cfg(test)]
mod tests;
