use crate::snippets::discovery::discover_snippets;
use crate::snippets::error::Result;
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
    /// `docs_dir.join(target)` when the list is empty or no path matches.
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
    let mut references: Vec<_> = discover_includes(&config.docs_dirs, &config.include_base_paths)?
        .into_iter()
        .filter(|reference| !is_excluded(&reference.source, &config.exclude))
        .collect();
    references.extend(config.configured_references.iter().map(|target| SnippetReference {
        source: target.clone(),
        target: target.clone(),
        line: 1,
    }));
    let snippet_files = snippet_files(&snippets);

    Ok(GapReport {
        missing_references: missing_references(&references),
        unreferenced_snippets: unreferenced_snippets(&snippet_files, &references),
        missing_language_variants: missing_language_variants(&snippets, &config.required_languages),
        skips_without_reason: skips_without_reason(&snippets),
        unknown_languages: unknown_languages(&config.snippet_dirs)?
            .into_iter()
            .filter(|unknown| !is_excluded(&unknown.path, &config.exclude))
            .collect(),
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
    let Some(snippets_dir) = &readme.snippets_dir else {
        return Vec::new();
    };
    let mut references = Vec::new();
    for (language, entry) in &readme.languages {
        let source_language = entry
            .get("snippet_language")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(language);
        if let Some(snippets) = entry.get("snippets") {
            collect_readme_snippet_paths(snippets, &mut |path| {
                let path = normalize_readme_snippet_path(path, language, source_language);
                references.push(normalize_path(&workspace_root.join(snippets_dir).join(path)));
            });
        }
    }
    references.sort();
    references.dedup();
    references
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
    let mut references = Vec::new();
    for snippet_root in snippet_dirs {
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
        for manifest in manifests {
            let output_root = manifest.parent().ok_or_else(|| {
                crate::snippets::error::Error::Other(format!(
                    "coverage ledger has no output root: {}",
                    manifest.display()
                ))
            })?;
            references.extend(read_coverage_ledger_references(output_root, &manifest)?);
        }
    }
    references.sort();
    references.dedup();
    Ok(references)
}

fn read_coverage_ledger_references(output_root: &Path, manifest: &Path) -> Result<Vec<PathBuf>> {
    let content = std::fs::read_to_string(manifest)?;
    let ledger: crate::e2e::snippets::SnippetCoverageLedger = serde_json::from_str(&content)?;
    crate::e2e::snippets::coverage::validate(&ledger)
        .map_err(|error| crate::snippets::error::Error::Other(format!("invalid coverage ledger: {error:#}")))?;
    if !ledger.missing.is_empty() {
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

fn collect_readme_snippet_paths(value: &serde_json::Value, collect: &mut impl FnMut(&str)) {
    match value {
        serde_json::Value::String(path) => collect(path),
        serde_json::Value::Array(values) => {
            for value in values {
                collect_readme_snippet_paths(value, collect);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                collect_readme_snippet_paths(value, collect);
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
    let mut references = Vec::new();
    for docs_dir in docs_dirs {
        for path in markdown_files(docs_dir) {
            let content = std::fs::read_to_string(&path)?;
            references.extend(parse_includes(&content, &path, docs_dir, include_base_paths));
            references.extend(parse_mdx_content_imports(&content, &path));
        }
    }
    references.sort_by(|left, right| left.source.cmp(&right.source).then(left.line.cmp(&right.line)));
    Ok(references)
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
/// deliberately does not take `include_base_paths`.
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
/// `missing_references` needs to detect).
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

fn missing_language_variants(snippets: &[Snippet], required_languages: &[Language]) -> Vec<MissingLanguageVariant> {
    if required_languages.is_empty() {
        return Vec::new();
    }

    let mut groups: BTreeMap<PathBuf, BTreeSet<Language>> = BTreeMap::new();
    for snippet in snippets {
        let Some(group) = language_group(&snippet.path, snippet.language) else {
            continue;
        };
        groups.entry(group).or_default().insert(snippet.language);
    }

    let mut missing = Vec::new();
    for (group, languages) in groups {
        for language in required_languages {
            if !languages.contains(language) {
                missing.push(MissingLanguageVariant {
                    group: group.clone(),
                    language: *language,
                });
            }
        }
    }
    missing
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
mod tests {
    use super::*;
    use crate::e2e::fixture::SideEffectClass;
    use crate::e2e::snippets::{
        COVERAGE_MANIFEST_VERSION, GeneratedSnippetMetadata, SnippetCoverageKey, SnippetCoverageLedger,
    };

    #[test]
    fn discovers_mkdocs_include_references() {
        let refs = parse_includes(
            r#"
    --8<-- "snippets/python/example.md"
"#,
            Path::new("/repo/docs/index.md"),
            Path::new("/repo/docs"),
            &[],
        );

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target, PathBuf::from("/repo/docs/snippets/python/example.md"));
        assert_eq!(refs[0].line, 2);
    }

    #[test]
    fn reports_fixture_tree_gaps() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        let snippets = docs.join("snippets");
        std::fs::create_dir_all(snippets.join("python")).unwrap();
        std::fs::create_dir_all(snippets.join("rust")).unwrap();
        std::fs::write(docs.join("index.md"), r#"--8<-- "snippets/python/example.md""#).unwrap();
        std::fs::write(snippets.join("python/example.md"), "```python\nprint('ok')\n```\n").unwrap();
        std::fs::write(
            snippets.join("rust/unused.md"),
            "<!-- snippet:skip -->\n```rust\nfn main() {}\n```\n",
        )
        .unwrap();

        let report = detect_gaps(&GapConfig {
            docs_dirs: vec![docs],
            snippet_dirs: vec![snippets],
            required_languages: vec![Language::Python, Language::Rust],
            include_base_paths: vec![],
            configured_references: vec![],
            exclude: vec![],
        })
        .unwrap();

        assert!(report.missing_references.is_empty());
        assert_eq!(report.unreferenced_snippets.len(), 1);
        assert_eq!(report.missing_language_variants.len(), 2);
        assert_eq!(report.skips_without_reason.len(), 1);
    }

    #[test]
    fn generated_ledger_references_preserve_manual_orphan_detection() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let snippets = directory.path().join("snippets");
        let generated_root = snippets.join("generated");
        let generated = generated_root.join("python/topic/generated.md");
        let manual = snippets.join("python/topic/manual.md");
        std::fs::create_dir_all(generated.parent().expect("generated parent")).expect("snippet directory");
        std::fs::create_dir_all(manual.parent().expect("manual parent")).expect("manual snippet directory");
        std::fs::write(&generated, "```python\nvalue = 1\n```\n").expect("generated snippet");
        std::fs::write(&manual, "```python\nvalue = 2\n```\n").expect("manual snippet");
        std::fs::write(
            generated_root.join(crate::e2e::snippets::COVERAGE_MANIFEST),
            serde_json::to_vec_pretty(&coverage_ledger(COVERAGE_MANIFEST_VERSION)).expect("coverage serializes"),
        )
        .expect("coverage manifest");

        let references = coverage_ledger_references(std::slice::from_ref(&snippets)).expect("current ledger");
        let report = detect_gaps(&GapConfig {
            snippet_dirs: vec![snippets],
            configured_references: references,
            ..GapConfig::default()
        })
        .expect("gap detection");

        assert_eq!(report.unreferenced_snippets, [manual]);
    }

    #[test]
    fn nested_coverage_ledger_rejects_paths_outside_its_output_root() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let snippets = directory.path().join("snippets");
        let generated_root = snippets.join("generated");
        std::fs::create_dir_all(&generated_root).expect("generated snippet directory");
        std::fs::write(snippets.join("outside.md"), "```python\nvalue = 1\n```\n").expect("outside snippet");
        let mut ledger = coverage_ledger(COVERAGE_MANIFEST_VERSION);
        let outside = PathBuf::from("../outside.md");
        ledger.generated_paths[0] = outside.clone();
        ledger.generated_metadata[0].path = outside;
        std::fs::write(
            generated_root.join(crate::e2e::snippets::COVERAGE_MANIFEST),
            serde_json::to_vec_pretty(&ledger).expect("coverage serializes"),
        )
        .expect("coverage manifest");

        let error = coverage_ledger_references(&[snippets]).expect_err("outside path must fail");

        assert!(error.to_string().contains("must stay beneath its output root"));
    }

    #[test]
    fn stale_coverage_ledger_is_rejected() {
        let directory = tempfile::tempdir().expect("temporary directory");
        std::fs::write(
            directory.path().join(crate::e2e::snippets::COVERAGE_MANIFEST),
            serde_json::to_vec_pretty(&coverage_ledger(0)).expect("coverage serializes"),
        )
        .expect("coverage manifest");

        let error = coverage_ledger_references(&[directory.path().to_path_buf()]).expect_err("stale ledger must fail");

        assert!(error.to_string().contains("coverage manifest version 0 is unsupported"));
    }

    fn coverage_ledger(format_version: u32) -> SnippetCoverageLedger {
        let key = SnippetCoverageKey {
            fixture_id: "generated".into(),
            language: "python".into(),
        };
        SnippetCoverageLedger {
            format_version,
            generated_paths: vec![PathBuf::from("python/topic/generated.md")],
            generated_metadata: vec![GeneratedSnippetMetadata {
                key: key.clone(),
                path: PathBuf::from("python/topic/generated.md"),
                language: "python".into(),
                target: "python".into(),
                session: "python".into(),
                requires: Vec::new(),
                side_effect: SideEffectClass::Safe,
            }],
            expected: vec![key.clone()],
            generated: vec![key],
            missing: Vec::new(),
            documented_exceptions: Vec::new(),
        }
    }

    #[test]
    fn resolves_changelog_include_via_project_root_base_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let docs = root.join("docs");
        let snippets = docs.join("snippets");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::create_dir_all(&snippets).unwrap();
        std::fs::write(root.join("CHANGELOG.md"), "# Changelog\n").unwrap();
        std::fs::write(docs.join("changelog.md"), r#"--8<-- "CHANGELOG.md""#).unwrap();

        let report = detect_gaps(&GapConfig {
            docs_dirs: vec![docs],
            snippet_dirs: vec![snippets],
            required_languages: vec![],
            include_base_paths: vec![root.to_path_buf()],
            configured_references: vec![],
            exclude: vec![],
        })
        .unwrap();

        assert!(
            report.missing_references.is_empty(),
            "expected no missing references, got: {:?}",
            report.missing_references
        );
    }

    #[test]
    fn parses_mdx_content_import() {
        let target = parse_mdx_content_import_target(
            r#"import { Content as Snip_cli_install_cargo } from "../../../snippets/cli/install_cargo.md";"#,
        );
        assert_eq!(target, Some("../../../snippets/cli/install_cargo.md"));
    }

    #[test]
    fn parses_bare_content_import_without_alias() {
        let target = parse_mdx_content_import_target(r#"import { Content } from "../snippets/x.md";"#);
        assert_eq!(target, Some("../snippets/x.md"));
    }

    #[test]
    fn ignores_non_content_imports() {
        assert_eq!(
            parse_mdx_content_import_target(r#"import { Card } from "@astrojs/starlight/components";"#),
            None
        );
        assert_eq!(
            parse_mdx_content_import_target(r#"import Layout from "../Layout.astro";"#),
            None
        );
    }

    #[test]
    fn parses_multiple_mdx_content_imports_in_one_file() {
        let content = concat!(
            "---\ntitle: Usage\n---\n",
            "import { Content as Snip_a } from \"../../../snippets/cli/a.md\";\n",
            "import { Content as Snip_b } from \"../../../snippets/cli/b.md\";\n",
        );
        let refs = parse_mdx_content_imports(content, Path::new("/repo/a/b/c/usage.mdx"));
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].target, PathBuf::from("/repo/snippets/cli/a.md"));
        assert_eq!(refs[0].line, 4);
        assert_eq!(refs[1].target, PathBuf::from("/repo/snippets/cli/b.md"));
        assert_eq!(refs[1].line, 5);
    }

    #[test]
    fn mdx_import_path_is_relative_to_importing_file_not_docs_dir() {
        // The importing file lives three directories below `docs_dir` (mirroring
        // the real `docs/<section>/<page>.mdx` -> `../../../snippets/...` shape),
        // so resolving against `docs_dir` directly (the MkDocs behavior) would
        // produce the wrong path. Resolution must instead be relative to the
        // importing file's own directory.
        let refs = parse_mdx_content_imports(
            r#"import { Content as Snip_x } from "../../../snippets/cli/install_cargo.md";"#,
            Path::new("/repo/docs-site/src/content/docs/cli/usage.mdx"),
        );
        assert_eq!(refs.len(), 1);
        assert_eq!(
            refs[0].target,
            PathBuf::from("/repo/docs-site/src/snippets/cli/install_cargo.md")
        );
    }

    #[test]
    fn walks_mdx_files_for_references() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("content").join("docs");
        let snippets = dir.path().join("content").join("snippets");
        std::fs::create_dir_all(docs.join("cli")).unwrap();
        std::fs::create_dir_all(snippets.join("cli")).unwrap();
        std::fs::write(
            docs.join("cli").join("usage.mdx"),
            "import { Content as Snip_x } from \"../../snippets/cli/install_cargo.md\";\n",
        )
        .unwrap();
        std::fs::write(
            snippets.join("cli").join("install_cargo.md"),
            "```bash\ncargo install alef\n```\n",
        )
        .unwrap();
        std::fs::write(snippets.join("cli").join("orphan.md"), "```bash\necho orphan\n```\n").unwrap();

        let report = detect_gaps(&GapConfig {
            docs_dirs: vec![docs],
            snippet_dirs: vec![snippets],
            required_languages: vec![],
            include_base_paths: vec![],
            configured_references: vec![],
            exclude: vec![],
        })
        .unwrap();

        assert_eq!(
            report.unreferenced_snippets.len(),
            1,
            "expected only orphan.md to be unreferenced, got: {:?}",
            report.unreferenced_snippets
        );
        assert!(report.unreferenced_snippets[0].ends_with("orphan.md"));
    }

    #[test]
    fn walks_astro_files_for_references() {
        let directory = tempfile::tempdir().expect("temporary docs directory");
        let docs = directory.path().join("docs");
        let snippets = directory.path().join("snippets");
        std::fs::create_dir_all(&docs).expect("create docs directory");
        std::fs::create_dir_all(&snippets).expect("create snippets directory");
        std::fs::write(
            docs.join("example.astro"),
            "import { Content as Example } from \"../snippets/example.md\";\n",
        )
        .expect("write Astro page");
        std::fs::write(snippets.join("example.md"), "```rust\nlet value = 1;\n```\n").expect("write snippet");

        let references = discover_includes(&[docs], &[]).expect("discover Astro imports");

        assert_eq!(references.len(), 1);
        assert_eq!(references[0].target, snippets.join("example.md"));
    }

    #[test]
    fn readme_only_references_honor_language_redirects_and_prefixes() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = dir.path().join("docs-site/src/snippets");
        std::fs::create_dir_all(snippets.join("c/api")).unwrap();
        let snippet = snippets.join("c/api/client.md");
        std::fs::write(&snippet, "```c\nint client(void);\n```\n").unwrap();
        let readme: crate::core::config::ReadmeConfig = serde_json::from_value(serde_json::json!({
            "template_dir": null,
            "snippets_dir": "docs-site/src/snippets",
            "config": null,
            "output_pattern": null,
            "discord_url": null,
            "banner_url": null,
            "languages": {
                "ffi": {
                    "snippet_language": "c",
                    "snippets": { "client": "ffi/api/client.md" }
                }
            },
            "targets": {}
        }))
        .unwrap();
        let references = readme_snippet_references(dir.path(), Some(&readme));
        assert_eq!(references, vec![snippet.clone()]);

        let report = detect_gaps(&GapConfig {
            snippet_dirs: vec![snippets],
            configured_references: references,
            ..GapConfig::default()
        })
        .unwrap();
        assert!(report.unreferenced_snippets.is_empty());
        assert!(report.missing_references.is_empty());
    }
}
