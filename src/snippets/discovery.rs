use crate::snippets::error::Result;
use crate::snippets::parser;
use crate::snippets::types::{Language, Snippet, SnippetAnnotation, SnippetAnnotationKind, SourceOrigin};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Discover snippets beneath the provided directories.
///
/// # Errors
///
/// Returns an error when a source file cannot be parsed into snippet blocks.
pub fn discover_snippets(dirs: &[PathBuf], language_filter: Option<&[Language]>) -> Result<Vec<Snippet>> {
    let mut snippets = Vec::new();

    for dir in dirs {
        // A configured directory that does not exist is almost always a typo or a path that
        // generation has not populated yet -- never a legitimate "nothing to check" signal. Prior
        // behaviour silently skipped it, which made a snippets root that was repointed but never
        // populated indistinguishable from one that was fully validated: discovery would report
        // zero snippets and every caller (`alef snippets list`, `check`, `gaps`) treated that as a
        // clean run. A directory that exists but is empty is a different, legitimate case and is
        // left alone -- it still contributes zero snippets, but callers can tell the two apart
        // because this path only rejects a directory that is missing outright. ~keep
        if !dir.exists() {
            return Err(crate::snippets::error::Error::Other(format!(
                "configured snippet directory does not exist: {}",
                dir.display()
            )));
        }
        let ledger_metadata = load_ledger_metadata(dir)?;

        for entry in WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| {
                !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(".alef-"))
            })
        {
            let path = entry.path();
            let mut file_snippets = extract_snippets_from_file(path, dir)?;
            if let Ok(relative) = path.strip_prefix(dir)
                && let Some(metadata) = ledger_metadata.get(relative)
            {
                for snippet in &mut file_snippets {
                    enrich_from_ledger(snippet, metadata)?;
                }
            }

            for snippet in file_snippets {
                if let Some(filter) = language_filter
                    && !filter.contains(&snippet.language)
                {
                    continue;
                }

                snippets.push(snippet);
            }
        }
    }

    snippets.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.block_index.cmp(&right.block_index))
    });
    Ok(snippets)
}

fn load_ledger_metadata(directory: &Path) -> Result<HashMap<PathBuf, crate::e2e::snippets::GeneratedSnippetMetadata>> {
    let manifest = directory.join(crate::e2e::snippets::COVERAGE_MANIFEST);
    if !manifest.is_file() {
        return Ok(HashMap::new());
    }
    let content = std::fs::read_to_string(&manifest)?;
    let ledger: crate::e2e::snippets::SnippetCoverageLedger = serde_json::from_str(&content)?;
    if ledger.format_version != crate::e2e::snippets::COVERAGE_MANIFEST_VERSION
        || ledger.generated_metadata.len() != ledger.generated_paths.len()
    {
        return Err(crate::snippets::error::Error::Other(format!(
            "stale fixture-snippet metadata ledger at {}",
            manifest.display()
        )));
    }
    let expected: std::collections::BTreeSet<_> = ledger.generated_paths.into_iter().collect();
    let actual: std::collections::BTreeSet<_> = ledger
        .generated_metadata
        .iter()
        .map(|metadata| metadata.path.clone())
        .collect();
    if expected != actual {
        return Err(crate::snippets::error::Error::Other(format!(
            "invalid fixture-snippet metadata ledger at {}",
            manifest.display()
        )));
    }
    for path in &actual {
        crate::e2e::snippets::ledger_paths::resolve_tracked_path(directory, path).map_err(|_| {
            crate::snippets::error::Error::Other(format!(
                "invalid fixture-snippet metadata ledger at {}",
                manifest.display()
            ))
        })?;
    }
    Ok(ledger
        .generated_metadata
        .into_iter()
        .map(|metadata| (metadata.path.clone(), metadata))
        .collect())
}

fn enrich_from_ledger(snippet: &mut Snippet, metadata: &crate::e2e::snippets::GeneratedSnippetMetadata) -> Result<()> {
    let language = Language::from_fence_tag(&metadata.language);
    if language == Language::Unknown {
        return Err(crate::snippets::error::Error::Other(format!(
            "unknown ledger language `{}` for {}",
            metadata.language,
            metadata.path.display()
        )));
    }
    snippet.id = Some(metadata.key.fixture_id.clone());
    snippet.language = language;
    snippet.metadata.id = snippet.id.clone();
    snippet.metadata.language = Some(language);
    snippet.metadata.target = Some(metadata.session.clone());
    snippet.metadata.requires = metadata.requires.clone();
    snippet.metadata.side_effect = Some(match metadata.side_effect {
        crate::e2e::fixture::SideEffectClass::Safe => crate::snippets::types::SideEffectClass::Safe,
        crate::e2e::fixture::SideEffectClass::Network => crate::snippets::types::SideEffectClass::Network,
        crate::e2e::fixture::SideEffectClass::Process => crate::snippets::types::SideEffectClass::Process,
        crate::e2e::fixture::SideEffectClass::Install => crate::snippets::types::SideEffectClass::Install,
        crate::e2e::fixture::SideEffectClass::Server => crate::snippets::types::SideEffectClass::Server,
    });
    Ok(())
}

fn extract_snippets_from_file(path: &Path, base_dir: &Path) -> Result<Vec<Snippet>> {
    let blocks = parser::parse_code_blocks(path)?;
    let dir_language = infer_language_from_path(path, base_dir);
    let mut snippets = Vec::new();

    for (index, block) in blocks.into_iter().enumerate() {
        let metadata = block.metadata;
        // Precedence: explicit fence tag > front-matter `language` > directory-derived > Unknown.
        // A fence's own tag is per-block ground truth; front matter and the directory are
        // both file-level defaults for blocks whose fence is absent or unrecognized. ~keep
        let from_fence = Language::from_fence_tag(&block.lang);
        let language = if from_fence != Language::Unknown {
            from_fence
        } else {
            metadata.language.or(dir_language).unwrap_or(Language::Unknown)
        };

        if language == Language::Unknown {
            continue;
        }

        let annotation = block.preceding_comment.as_deref().and_then(parse_annotation);
        let title = metadata.title.clone().or(block.title);
        // A front-matter `level:` is a contract (see `snippet.metadata.level`), not a suppression
        // annotation — collapsing it into `annotation` made it indistinguishable from a `<!--
        // snippet:syntax-only -->` comment, so the runner charged an author who declared exactly
        // the level they wanted as if they had asked for less than the run requested. Only an
        // explicit `skip` still synthesizes an annotation here. ~keep
        let annotation = if metadata.skip {
            Some(SnippetAnnotation {
                kind: SnippetAnnotationKind::Skip,
                reason: metadata.reason.clone(),
            })
        } else {
            annotation
        };

        let source_origin = SourceOrigin {
            path: path.to_path_buf(),
            line: block.start_line,
            block_index: index,
        };
        snippets.push(Snippet {
            id: metadata.id.clone(),
            path: path.to_path_buf(),
            language,
            title,
            code: block.code,
            start_line: block.start_line,
            block_index: index,
            annotation,
            metadata,
            source_origin,
        });
    }

    Ok(snippets)
}

fn infer_language_from_path(path: &Path, base_dir: &Path) -> Option<Language> {
    let relative = path.strip_prefix(base_dir).ok()?;
    for component in relative.components() {
        let dir_name = component.as_os_str().to_str()?;
        let language = Language::from_dir_name(dir_name);
        if language != Language::Unknown {
            return Some(language);
        }
    }

    None
}

pub fn parse_annotation(comment: &str) -> Option<SnippetAnnotation> {
    let inner = comment.trim().strip_prefix("<!--")?.strip_suffix("-->")?.trim();
    let mut parts = inner.split_whitespace();
    let kind = match parts.next()? {
        "snippet:skip" => SnippetAnnotationKind::Skip,
        "snippet:compile-only" => SnippetAnnotationKind::CompileOnly,
        "snippet:syntax-only" => SnippetAnnotationKind::SyntaxOnly,
        "snippet:typecheck-only" => SnippetAnnotationKind::TypeCheckOnly,
        _ => return None,
    };
    let reason = parse_reason_attr(inner);

    Some(SnippetAnnotation { kind, reason })
}

fn parse_reason_attr(inner: &str) -> Option<String> {
    let marker = "reason=";
    let start = inner.find(marker)? + marker.len();
    let rest = inner[start..].trim_start();

    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        return Some(stripped[..end].to_string());
    }

    if let Some(stripped) = rest.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        return Some(stripped[..end].to_string());
    }

    let value: String = rest.chars().take_while(|ch| !ch.is_whitespace()).collect();
    if value.is_empty() { None } else { Some(value) }
}

#[must_use]
pub fn count_by_language(snippets: &[Snippet]) -> Vec<(Language, usize)> {
    let mut counts: HashMap<Language, usize> = HashMap::new();
    for snippet in snippets {
        *counts.entry(snippet.language).or_default() += 1;
    }

    let mut result: Vec<_> = counts.into_iter().collect();
    result.sort_by_key(|(language, _)| language.to_string());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_annotations() {
        assert_eq!(
            parse_annotation("<!-- snippet:skip -->").map(|annotation| annotation.kind),
            Some(SnippetAnnotationKind::Skip)
        );
        assert_eq!(
            parse_annotation("<!-- snippet:compile-only -->").map(|annotation| annotation.kind),
            Some(SnippetAnnotationKind::CompileOnly)
        );
        assert_eq!(
            parse_annotation("<!-- snippet:syntax-only -->").map(|annotation| annotation.kind),
            Some(SnippetAnnotationKind::SyntaxOnly)
        );
        assert_eq!(
            parse_annotation("<!-- snippet:typecheck-only -->").map(|annotation| annotation.kind),
            Some(SnippetAnnotationKind::TypeCheckOnly)
        );
    }

    #[test]
    fn parses_annotation_reason_attribute() {
        let annotation = parse_annotation(r#"<!-- snippet:skip reason="requires service" -->"#).unwrap();
        assert_eq!(annotation.kind, SnippetAnnotationKind::Skip);
        assert_eq!(annotation.reason.as_deref(), Some("requires service"));
    }

    #[test]
    fn infers_language_from_nested_snippet_path() {
        let base = Path::new("/repo/docs");
        let path = Path::new("/repo/docs/snippets/python/example.md");
        assert_eq!(infer_language_from_path(path, base), Some(Language::Python));
    }

    #[test]
    fn does_not_infer_language_from_non_language_directories() {
        let base = Path::new("/repo/docs");
        let path = Path::new("/repo/docs/cli/usage.md");
        assert_eq!(infer_language_from_path(path, base), None);
    }

    /// A configured directory that does not exist on disk must fail discovery outright, not
    /// silently contribute zero snippets. Prior behaviour (`if !dir.exists() { continue; }`)
    /// made a snippets root that was repointed but never populated indistinguishable from one
    /// that was fully validated: every caller (`alef snippets list`, `check`, `gaps`) saw an
    /// empty `Ok(vec![])` and reported a clean run having examined nothing. ~keep
    #[test]
    fn a_missing_configured_directory_is_an_error_not_zero_snippets() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let missing = directory.path().join("does-not-exist");

        let error = discover_snippets(std::slice::from_ref(&missing), None)
            .expect_err("a configured directory that does not exist must fail discovery");

        assert!(
            error.to_string().contains(&missing.display().to_string()),
            "the error must name the missing path so the misconfiguration is actionable: {error}"
        );
    }

    /// The opposite case is distinct and must stay a legitimate, silent zero: a directory that
    /// exists but genuinely holds no snippets (yet) is not a typo, and `discover_snippets` alone
    /// cannot tell "not generated yet" from "intentionally empty" -- only `is_incomplete_status`
    /// and callers with more context should decide whether that is a warning or a failure.
    #[test]
    fn an_existing_empty_directory_still_discovers_zero_snippets_without_erroring() {
        let directory = tempfile::tempdir().expect("temporary directory");

        let discovered = discover_snippets(&[directory.path().to_path_buf()], None)
            .expect("an existing, empty directory must not error");

        assert!(
            discovered.is_empty(),
            "an empty directory legitimately contributes zero snippets"
        );
    }

    #[test]
    fn discovery_ignores_alef_metadata_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let snippet = directory.path().join("example.md");
        let coverage = directory.path().join(".alef-snippet-coverage.json");
        std::fs::write(&snippet, "```python\nprint('hello')\n```\n").expect("write snippet");
        let ledger = crate::e2e::snippets::SnippetCoverageLedger {
            format_version: crate::e2e::snippets::COVERAGE_MANIFEST_VERSION,
            ..Default::default()
        };
        std::fs::write(&coverage, serde_json::to_vec(&ledger).expect("serialize ledger")).expect("write coverage");

        let discovered = discover_snippets(&[directory.path().to_path_buf()], None).expect("discover snippets");

        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].path, snippet);
    }

    #[test]
    fn bare_fence_has_no_implicit_validation_limit() {
        let directory = tempfile::tempdir().expect("temporary snippet directory");
        let path = directory.path().join("example.md");
        std::fs::write(&path, "```rust\nlet value = 1;\n```\n").expect("write bare snippet");

        let snippets = discover_snippets(&[directory.path().to_path_buf()], None).expect("discover bare snippet");

        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].annotation, None);
    }

    /// A front-matter `level:` is a validation contract tracked on `metadata.level`; a `<!--
    /// snippet:*-only -->` comment is a separate suppression tracked on `annotation`. The two
    /// used to collapse into the same field, which made an author who declared exactly the level
    /// they wanted indistinguishable from one who suppressed validation below what was
    /// requested — see `runner::finalize_result` for how the distinction is used downstream. ~keep
    #[test]
    fn frontmatter_level_is_independent_of_inline_annotation() {
        let directory = tempfile::tempdir().expect("temporary snippet directory");
        let path = directory.path().join("example.md");
        std::fs::write(
            &path,
            "---\nlevel: typecheck\n---\n<!-- snippet:syntax-only -->\n```rust\nlet value = 1;\n```\n",
        )
        .expect("write annotated snippet");

        let snippets = discover_snippets(&[directory.path().to_path_buf()], None).expect("discover annotated snippet");

        assert_eq!(
            snippets[0].annotation.as_ref().map(|value| value.kind),
            Some(SnippetAnnotationKind::SyntaxOnly)
        );
        assert_eq!(
            snippets[0].metadata.level,
            Some(crate::snippets::types::ValidationLevel::TypeCheck)
        );
    }

    /// A bare front-matter `level:` with no inline comment must not synthesize an annotation —
    /// only `metadata.level` carries the declared contract.
    #[test]
    fn frontmatter_level_alone_does_not_synthesize_an_annotation() {
        let directory = tempfile::tempdir().expect("temporary snippet directory");
        let path = directory.path().join("example.md");
        std::fs::write(&path, "---\nlevel: syntax\n---\n```rust\nlet value = 1;\n```\n").expect("write snippet");

        let snippets = discover_snippets(&[directory.path().to_path_buf()], None).expect("discover snippet");

        assert_eq!(snippets[0].annotation, None);
        assert_eq!(
            snippets[0].metadata.level,
            Some(crate::snippets::types::ValidationLevel::Syntax)
        );
    }

    #[test]
    fn explicit_fence_tag_wins_over_frontmatter_language_per_block() {
        let directory = tempfile::tempdir().expect("temporary snippet directory");
        let path = directory.path().join("stdio.md");
        std::fs::write(
            &path,
            "---\nlanguage: toml\ntarget: toml\nlevel: syntax\n---\n\n\
             ```toml title=\"example-proxy.toml\"\n[mcp]\nstdio_trust_local = true\n```\n\n\
             ```json title=\"claude_desktop_config.json (stdio)\"\n{\n  \"mcpServers\": {}\n}\n```\n",
        )
        .expect("write mixed-language snippet file");

        let snippets = discover_snippets(&[directory.path().to_path_buf()], None).expect("discover snippets");

        assert_eq!(snippets.len(), 2);
        assert_eq!(snippets[0].language, Language::Toml);
        assert_eq!(snippets[1].language, Language::Json);
    }

    #[test]
    fn fence_without_recognized_tag_falls_back_to_frontmatter_language() {
        let directory = tempfile::tempdir().expect("temporary snippet directory");
        let path = directory.path().join("example.md");
        std::fs::write(
            &path,
            "---\nlanguage: python\n---\n\n```output\nsome interpreter output\n```\n",
        )
        .expect("write unrecognized-fence snippet file");

        let snippets = discover_snippets(&[directory.path().to_path_buf()], None).expect("discover snippets");

        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].language, Language::Python);
    }

    #[test]
    fn generated_markdown_is_enriched_from_authoritative_ledger() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let relative = PathBuf::from("wasm/topic/example.md");
        let snippet = directory.path().join(&relative);
        std::fs::create_dir_all(snippet.parent().expect("snippet parent")).expect("create snippet directory");
        std::fs::write(&snippet, "```typescript title=\"TypeScript\"\nexample();\n```\n").expect("write snippet");
        let key = crate::e2e::snippets::SnippetCoverageKey {
            fixture_id: "sample_request".into(),
            language: "wasm".into(),
        };
        let ledger = crate::e2e::snippets::SnippetCoverageLedger {
            format_version: crate::e2e::snippets::COVERAGE_MANIFEST_VERSION,
            generated_paths: vec![relative.clone()],
            generated_metadata: vec![crate::e2e::snippets::GeneratedSnippetMetadata {
                key: key.clone(),
                path: relative,
                language: "typescript".into(),
                target: "wasm".into(),
                session: "wasm".into(),
                requires: vec!["feature:web".into()],
                side_effect: crate::e2e::fixture::SideEffectClass::Network,
            }],
            expected: vec![key.clone()],
            generated: vec![key],
            missing: Vec::new(),
            documented_exceptions: Vec::new(),
        };
        std::fs::write(
            directory.path().join(crate::e2e::snippets::COVERAGE_MANIFEST),
            serde_json::to_vec(&ledger).expect("serialize ledger"),
        )
        .expect("write ledger");

        let discovered = discover_snippets(&[directory.path().to_path_buf()], None).expect("discover snippets");

        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].id.as_deref(), Some("sample_request"));
        assert_eq!(discovered[0].metadata.target.as_deref(), Some("wasm"));
        assert_eq!(discovered[0].metadata.requires, vec!["feature:web"]);
        assert_eq!(
            discovered[0].metadata.side_effect,
            Some(crate::snippets::types::SideEffectClass::Network)
        );
    }
}
