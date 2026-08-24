use crate::snippets::discovery;
use crate::snippets::gaps::{discover_includes, parse_include_target};
use crate::snippets::parser::{self, FrontmatterStatus};
use crate::snippets::types::Language;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Default)]
pub struct AuditConfig {
    pub docs_dirs: Vec<PathBuf>,
    pub snippet_dirs: Vec<PathBuf>,
    pub require_frontmatter: bool,
    pub include_base_paths: Vec<PathBuf>,
    pub configured_references: Vec<PathBuf>,
    pub exclude: Vec<PathBuf>,
    /// Accounting inputs: which snippets alef generates, and which the project declares it
    /// hand-authors. See [`AuditIssueKind::UnaccountedSnippet`].
    pub accounting: SnippetAccounting,
}

/// The two claims that let an audit tell a deliberately curated snippet apart from a coverage
/// gap: what alef's own coverage ledgers say it generated, and what
/// `[crates.e2e.snippets].curated_snippets` declares as hand-authored on purpose.
///
/// Curated paths come from configuration rather than from the coverage ledger, because the
/// ledger structurally cannot carry them: `e2e::snippets::ledger_paths::resolve_tracked_path`
/// refuses any recorded path that leaves the ledger's own `output` root, and hand-authored
/// snippets characteristically live outside `output`. Configuration is also the declaration's
/// source of truth, so an audit run after an `alef.toml` edit sees the edit without a
/// regeneration in between. ~keep
///
/// All three path lists must be spelled the same way the audit's `snippet_dirs` are, since
/// that is what the walk produces.
#[derive(Debug, Clone, Default)]
pub struct SnippetAccounting {
    /// Files a coverage ledger records as alef-generated.
    pub generated_paths: Vec<PathBuf>,
    /// Files a `curated_snippets` declaration claims as hand-authored.
    pub curated_paths: Vec<PathBuf>,
    /// Whether the accounting check runs at all.
    ///
    /// Off leaves the audit exactly as it was, so a caller with no configuration to read
    /// cannot report an accounting verdict it never computed. The CLI names the skip rather
    /// than printing a bare "Audit clean" over a check that did not run. ~keep
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditIssueKind {
    BrokenFrontmatter,
    MissingFrontmatter,
    BrokenFence,
    MissingInclude,
    InvalidInclude,
    UnknownLanguage,
    UnreadableFile,
    MissingDirectory,
    /// A snippet under an audited root that neither a coverage ledger claims as generated nor
    /// a `curated_snippets` declaration claims as hand-authored.
    ///
    /// Reported as a warning, not an error: an unaccounted snippet is a coverage observation,
    /// and one measured consumer tree carries 96 of them. Failing the audit on first sight
    /// would turn an informational gap into a red CI run for every project that has not
    /// declared its curated files yet. ~keep
    UnaccountedSnippet,
    /// A `curated_snippets` declaration claims a path a coverage ledger says alef generates.
    ///
    /// An error, unlike [`Self::UnaccountedSnippet`]: this is a declaration actively laying
    /// claim to alef's own output, which would let it mask a real coverage gap.
    CuratedGeneratedSnippet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditIssue {
    pub kind: AuditIssueKind,
    pub severity: AuditSeverity,
    pub path: PathBuf,
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReport {
    pub issues: Vec<AuditIssue>,
    /// Audited snippets a `curated_snippets` declaration accounts for.
    ///
    /// Carried positively rather than left as the mere absence of an
    /// [`AuditIssueKind::UnaccountedSnippet`]: "this file is curated" and "this file was
    /// never examined" are different facts, and only one of them is a verdict. ~keep
    #[serde(default)]
    pub curated: Vec<PathBuf>,
}

impl AuditReport {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|issue| issue.severity == AuditSeverity::Error)
    }
}

/// Audit documentation snippets and include references for structural errors.
///
/// # Errors
///
/// This function reports unreadable files and configured roots that do not exist as audit
/// issues rather than returning an error, so callers can see every problem found in one run.
/// ~keep
#[must_use]
pub fn audit(config: &AuditConfig) -> AuditReport {
    let mut issues = Vec::new();
    issues.extend(missing_directory_issues(
        discovery::SNIPPET_DIRECTORY_KIND,
        &config.snippet_dirs,
    ));
    issues.extend(missing_directory_issues(
        discovery::DOCUMENTATION_DIRECTORY_KIND,
        &config.docs_dirs,
    ));
    for snippet_dir in &config.snippet_dirs {
        issues.extend(audit_snippets(snippet_dir, config.require_frontmatter, &config.exclude));
    }
    for docs_dir in &config.docs_dirs {
        issues.extend(audit_docs(docs_dir, &config.include_base_paths, &config.exclude));
    }
    for path in &config.configured_references {
        if !path.exists() {
            issues.push(issue(
                AuditIssueKind::MissingInclude,
                path,
                1,
                format!("configured README snippet does not exist: {}", path.display()),
            ));
        }
    }
    let accounting = account_snippets(config);
    issues.extend(accounting.issues);
    issues.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.message.cmp(&right.message))
    });
    AuditReport {
        issues,
        curated: accounting.curated,
    }
}

/// What the accounting pass produced: issues to merge, and the curated files it recognised.
struct AccountingOutcome {
    issues: Vec<AuditIssue>,
    curated: Vec<PathBuf>,
}

/// Classify every audited snippet as generated, curated, or unaccounted.
///
/// This is the check that gives `alef snippets audit` a curated concept at all: without it a
/// hand-authored snippet and a genuine coverage gap are the same silence. A file is accounted
/// for when a coverage ledger records it as generated, or when a `curated_snippets`
/// declaration claims it; anything else under an audited snippet root is a gap nobody has
/// spoken for. ~keep
///
/// A curated declaration that claims a generated path is an error rather than an
/// accounting outcome -- see [`AuditIssueKind::CuratedGeneratedSnippet`].
fn account_snippets(config: &AuditConfig) -> AccountingOutcome {
    if !config.accounting.enabled {
        return AccountingOutcome {
            issues: Vec::new(),
            curated: Vec::new(),
        };
    }
    let generated: BTreeSet<&Path> = config.accounting.generated_paths.iter().map(PathBuf::as_path).collect();
    let curated: BTreeSet<&Path> = config.accounting.curated_paths.iter().map(PathBuf::as_path).collect();
    let mut issues = Vec::new();
    for claimed in curated.intersection(&generated) {
        issues.push(issue(
            AuditIssueKind::CuratedGeneratedSnippet,
            claimed,
            1,
            format!(
                "curated_snippets claims `{}`, which a coverage ledger records as alef-generated; \
                 a curated declaration must never claim a path alef writes",
                claimed.display()
            ),
        ));
    }
    let mut recognised_curated = Vec::new();
    for snippet_dir in &config.snippet_dirs {
        for path in markdown_files(snippet_dir, &config.exclude) {
            if generated.contains(path.as_path()) {
                continue;
            }
            if curated.contains(path.as_path()) {
                recognised_curated.push(path);
                continue;
            }
            issues.push(issue(
                AuditIssueKind::UnaccountedSnippet,
                &path,
                1,
                "snippet is neither recorded as alef-generated by a coverage ledger nor declared in \
                 [crates.e2e.snippets].curated_snippets; declare it curated or let alef generate it"
                    .to_string(),
            ));
        }
    }
    recognised_curated.sort();
    recognised_curated.dedup();
    AccountingOutcome {
        issues,
        curated: recognised_curated,
    }
}

/// Report every configured root that does not exist on disk.
///
/// Without this the audit reads as clean over a root that is not there: `markdown_files` walks
/// nothing and contributes no issues, so a `docs_dirs` entry pointing at a path that was renamed
/// or never created reports "Audit clean: no issues found" having examined not one file. The
/// missing-directory policy itself lives in `discovery::missing_configured_directories`; an audit
/// reports it as an issue rather than an error because that is how this module surfaces every
/// other unreadable input (see [`audit`]). ~keep
fn missing_directory_issues(kind: &str, dirs: &[PathBuf]) -> Vec<AuditIssue> {
    discovery::missing_configured_directories(dirs)
        .into_iter()
        .map(|directory| {
            issue(
                AuditIssueKind::MissingDirectory,
                directory,
                1,
                discovery::missing_directory_message(kind, directory),
            )
        })
        .collect()
}

fn audit_snippets(snippet_dir: &Path, require_frontmatter: bool, exclude: &[PathBuf]) -> Vec<AuditIssue> {
    markdown_files(snippet_dir, exclude)
        .into_iter()
        .flat_map(|path| audit_snippet_file(&path, require_frontmatter))
        .collect()
}

fn audit_snippet_file(path: &Path, require_frontmatter: bool) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            issues.push(issue(
                AuditIssueKind::UnreadableFile,
                path,
                1,
                format!("failed to read snippet file: {err}"),
            ));
            return issues;
        }
    };

    match parser::frontmatter_status(&content) {
        FrontmatterStatus::Missing if require_frontmatter => issues.push(issue(
            AuditIssueKind::MissingFrontmatter,
            path,
            1,
            "snippet markdown is missing YAML frontmatter".to_string(),
        )),
        FrontmatterStatus::Malformed(message) => {
            issues.push(issue(AuditIssueKind::BrokenFrontmatter, path, 1, message))
        }
        FrontmatterStatus::Present => {}
        FrontmatterStatus::Missing => {}
    }

    issues.extend(audit_fences(path, &content));
    issues
}

fn audit_docs(docs_dir: &Path, include_base_paths: &[PathBuf], exclude: &[PathBuf]) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    for path in markdown_files(docs_dir, exclude) {
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                issues.push(issue(
                    AuditIssueKind::UnreadableFile,
                    &path,
                    1,
                    format!("failed to read documentation file: {err}"),
                ));
                continue;
            }
        };

        issues.extend(audit_fences(&path, &content));
        issues.extend(audit_includes(&path, &content));
    }

    match discover_includes(&[docs_dir.to_path_buf()], include_base_paths) {
        Ok(references) => {
            for reference in references
                .into_iter()
                .filter(|reference| !is_excluded(&reference.source, exclude))
            {
                if !reference.target.exists() {
                    issues.push(issue(
                        AuditIssueKind::MissingInclude,
                        &reference.source,
                        reference.line,
                        format!("included snippet does not exist: {}", reference.target.display()),
                    ));
                }
            }
        }
        Err(err) => issues.push(issue(
            AuditIssueKind::UnreadableFile,
            docs_dir,
            1,
            format!("failed to discover include references: {err}"),
        )),
    }

    issues
}

fn audit_includes(path: &Path, content: &str) -> Vec<AuditIssue> {
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("--8<--") && parse_include_target(line).is_none())
        .map(|(index, _)| {
            issue(
                AuditIssueKind::InvalidInclude,
                path,
                index + 1,
                "invalid MkDocs include syntax, expected --8<-- \"path\"".to_string(),
            )
        })
        .collect()
}

fn audit_fences(path: &Path, content: &str) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    let mut open: Option<(usize, String)> = None;

    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("```") else {
            continue;
        };

        if rest.starts_with('`') {
            continue;
        }

        if open.is_some() && (rest.is_empty() || rest.chars().all(|ch| ch == '`')) {
            open = None;
            continue;
        }

        if open.is_none() {
            let tag = rest.split_whitespace().next().unwrap_or_default().to_string();
            if tag.is_empty() {
                issues.push(issue(
                    AuditIssueKind::UnknownLanguage,
                    path,
                    index + 1,
                    "fenced code block is missing a language tag".to_string(),
                ));
            } else if Language::from_fence_tag(&tag) == Language::Unknown && !is_known_display_tag(&tag) {
                issues.push(issue(
                    AuditIssueKind::UnknownLanguage,
                    path,
                    index + 1,
                    format!("unknown fenced code language: {tag}"),
                ));
            }
            open = Some((index + 1, tag));
        }
    }

    if let Some((line, _)) = open {
        issues.push(issue(
            AuditIssueKind::BrokenFence,
            path,
            line,
            "fenced code block is missing a closing fence".to_string(),
        ));
    }

    issues
}

fn markdown_files(base: &Path, exclude: &[PathBuf]) -> Vec<PathBuf> {
    // Not the silent-empty this module used to have: a root that is absent is reported by
    // `missing_directory_issues` before any walk happens, so an empty list here can no longer be
    // mistaken for an audited-and-clean root. ~keep
    if !base.exists() {
        return Vec::new();
    }

    let mut files: Vec<PathBuf> = WalkDir::new(base)
        .follow_links(true)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|path| !is_excluded(path, exclude))
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| matches!(extension.to_lowercase().as_str(), "md" | "markdown" | "mdx"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

fn is_excluded(path: &Path, exclude: &[PathBuf]) -> bool {
    exclude.iter().any(|excluded| path.starts_with(excluded))
}

fn issue(kind: AuditIssueKind, path: &Path, line: usize, message: String) -> AuditIssue {
    AuditIssue {
        severity: severity_for(&kind),
        kind,
        path: path.to_path_buf(),
        line,
        message,
    }
}

/// Every kind's severity in one place, so a new kind cannot silently inherit `Error` from a
/// constructor that hard-coded it. ~keep
fn severity_for(kind: &AuditIssueKind) -> AuditSeverity {
    match kind {
        AuditIssueKind::UnaccountedSnippet => AuditSeverity::Warning,
        AuditIssueKind::BrokenFrontmatter
        | AuditIssueKind::MissingFrontmatter
        | AuditIssueKind::BrokenFence
        | AuditIssueKind::MissingInclude
        | AuditIssueKind::InvalidInclude
        | AuditIssueKind::UnknownLanguage
        | AuditIssueKind::UnreadableFile
        | AuditIssueKind::MissingDirectory
        | AuditIssueKind::CuratedGeneratedSnippet => AuditSeverity::Error,
    }
}

/// Returns true for fence tags that are valid display-only markup the audit
/// should accept without flagging as `UnknownLanguage`. These tags do not map
/// to executable validators in `Language::from_fence_tag`, but they are
/// well-known in the Markdown / docs ecosystem (data formats, diagram DSLs,
/// shell session transcripts, third-party JVM build files, etc.). ~keep
fn is_known_display_tag(tag: &str) -> bool {
    matches!(
        tag.trim().to_lowercase().as_str(),
        "json"
            | "yaml"
            | "yml"
            | "xml"
            | "ini"
            | "csv"
            | "tsv"
            | "properties"
            | "env"
            | "diff"
            | "patch"
            | "html"
            | "css"
            | "scss"
            | "sass"
            | "svg"
            | "markdown"
            | "md"
            | "mdx"
            | "rst"
            | "tex"
            | "latex"
            | "mermaid"
            | "plantuml"
            | "graphviz"
            | "dot"
            | "d2"
            | "groovy"
            | "gradle"
            | "make"
            | "makefile"
            | "cmake"
            | "nginx"
            | "apache"
            | "text"
            | "txt"
            | "plain"
            | "plaintext"
            | "output"
            | "log"
            | "console"
            | "sql"
            | "graphql"
            | "gql"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_missing_frontmatter_and_broken_fence() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = dir.path().join("snippets");
        std::fs::create_dir_all(&snippets).unwrap();
        std::fs::write(snippets.join("example.md"), "```python\nprint('ok')\n").unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: Vec::new(),
            snippet_dirs: vec![snippets],
            require_frontmatter: true,
            ..AuditConfig::default()
        });

        assert!(report.has_errors());
        assert_eq!(report.issues.len(), 2);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == AuditIssueKind::MissingFrontmatter)
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == AuditIssueKind::BrokenFence)
        );
    }

    /// A configured `docs_dirs` root that does not exist must be reported, not walked over in
    /// silence. `markdown_files` returns an empty list for a path that is not there, so the
    /// audit previously reported zero issues -- "Audit clean: no issues found" -- for a
    /// documentation tree it never opened, which is indistinguishable from one that was fully
    /// audited. Unlike `snippet_dirs`, nothing upstream of this walks `docs_dirs` eagerly, so
    /// this was live, not merely theoretical. ~keep
    #[test]
    fn reports_a_docs_directory_that_does_not_exist() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let missing = dir.path().join("docs-never-created");

        let report = audit(&AuditConfig {
            docs_dirs: vec![missing.clone()],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
            ..AuditConfig::default()
        });

        assert!(
            report.has_errors(),
            "a documentation root that does not exist must fail the audit, not read as clean"
        );
        assert_eq!(
            report.issues.len(),
            1,
            "exactly one issue is expected for one missing root: {:?}",
            report.issues
        );
        assert_eq!(report.issues[0].kind, AuditIssueKind::MissingDirectory);
        assert!(
            report.issues[0].message.contains(&missing.display().to_string()),
            "the issue must name the missing path so the misconfiguration is actionable: {}",
            report.issues[0].message
        );
    }

    /// The same policy `discover_snippets` established: a root that exists but holds nothing is
    /// legitimate and stays silent. Only a root that is missing outright is a misconfiguration.
    #[test]
    fn an_existing_empty_docs_directory_audits_clean() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).expect("create empty docs directory");

        let report = audit(&AuditConfig {
            docs_dirs: vec![docs],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
            ..AuditConfig::default()
        });

        assert_eq!(
            report.issues,
            Vec::new(),
            "an existing but empty documentation root is not a misconfiguration"
        );
    }

    #[test]
    fn reports_invalid_and_missing_includes() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("index.md"),
            "--8<-- snippets/python/example.md\n--8<-- \"snippets/python/missing.md\"\n",
        )
        .unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: vec![docs],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
            ..AuditConfig::default()
        });

        assert_eq!(report.issues.len(), 2);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == AuditIssueKind::InvalidInclude)
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == AuditIssueKind::MissingInclude)
        );
    }

    #[test]
    fn audits_fences_in_mdx_docs_pages() {
        // The consumer docs site is Astro Starlight, whose pages are `.mdx`, not
        // `.md`. `gaps::markdown_files` walks `.mdx` for snippet-reference
        // discovery, so `audit_docs` must walk the same extensions for the same
        // `docs_dirs` or it silently skips every real docs page.
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("usage.mdx"), "```python\nprint('ok')\n").unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: vec![docs],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
            ..AuditConfig::default()
        });

        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, AuditIssueKind::BrokenFence);
    }
}
