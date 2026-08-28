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
    /// A fence tag that names no language alef generates bindings for, resolves to nothing,
    /// and is not one of the standard display languages `is_recognized_display_language`
    /// knows by name (`html`, `css`, `ini`, `makefile`, `mdx`, `nginx`, `groovy`, ...) -- those
    /// audit clean with no finding at all, because alef cannot *validate* them but does
    /// *recognize* them; this kind is for what is left after that.
    ///
    /// A warning, not an error: a human-authored docs page may legitimately fence `astro`,
    /// `hcl`, or any other prose vocabulary this module has not been taught by name, and
    /// failing the run on those made consumers relabel their own documentation. But a typo
    /// (`pythn`) is indistinguishable from an untaught prose vocabulary without maintaining an
    /// allowlist of every language in existence, so staying SILENT here makes snippet
    /// validation falsely green. Warning keeps the typo actionable while letting untaught
    /// prose fences through. ~keep
    UnrecognizedFenceLanguage,
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
            } else if let Some((kind, message)) = unrecognized_fence_finding(&tag) {
                issues.push(issue(kind, path, index + 1, message));
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
        AuditIssueKind::UnaccountedSnippet | AuditIssueKind::UnrecognizedFenceLanguage => AuditSeverity::Warning,
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

/// True when `tag` (a fence's raw info string, e.g. `rust,no_run` or `astro`) names a
/// language alef actually generates bindings for, even though `tag` as a whole did not
/// resolve cleanly via [`Language::from_fence_info`] -- for example a rustdoc-attribute-style
/// fence combining `rust` with an unrecognized extra token. A fence in that state is
/// claiming a real binding-target language and getting it wrong, which is worth flagging.
///
/// A tag that names no binding-target language at all -- `astro`, `hcl`, or any other
/// prose-decoration language a human-authored docs page may legitimately fence -- returns
/// `false`: that fence is display-only, and must not fail validation just because nobody
/// happened to add it to a hand-maintained allowlist. This replaced a hardcoded
/// "known display tag" allowlist that was itself a second, drifting vocabulary alongside
/// `Language`'s own -- ask [`Language::is_binding_target`], the single authority, instead of
/// growing a parallel list here. ~keep
fn tag_claims_a_binding_target_language(tag: &str) -> bool {
    tag.split(',')
        .map(str::trim)
        .any(|token| Language::from_fence_tag(token).is_binding_target())
}

/// True when `tag` names a real, standard documentation fence language that alef has no
/// validator for and never will -- as opposed to a tag nobody can identify at all.
///
/// This is a distinct fact from [`tag_claims_a_binding_target_language`] returning `false`:
/// that question asks "is this a broken/leaked binding-target tag," and covers every
/// non-target string with a uniform warning, including a genuine typo like `pythn`.
/// This question asks "is this specific tag a known quantity," so a real label -- `html`,
/// `css`, `ini`, `makefile`, `mdx`, `nginx`, `groovy` -- can audit clean while an unrecognized
/// string still warns. Recognizing everything here instead would erase that distinction and
/// let `pythn` pass silently, which is exactly the falsely-green outcome
/// [`AuditIssueKind::UnrecognizedFenceLanguage`] exists to prevent.
///
/// This vocabulary is not new and was not invented here: it is the `is_known_display_tag`
/// allowlist that `097724925` deleted. That commit was fixing a different bug -- the allowlist
/// was being used to *error* on anything absent from it, which failed a run on a legitimate
/// ```astro fence -- and dropping it was the right fix for that. But it also discarded the
/// curated vocabulary, and once `a83a1ce44` added the warning tier, every name the list had
/// covered started warning instead of passing silently. Restored here in the tier that only
/// suppresses a finding, so it can no longer fail anything. Entries that now resolve through
/// `Language::from_fence_tag` on their own (`json`, `yaml`, `xml`, `mermaid`, `text`,
/// `console`, ...) are deliberately left out -- this function runs only after
/// `from_fence_info` has already returned `Unknown`, so those would be dead arms.
///
/// `htm` is the one addition, alongside `html`, for the same reason `Language::from_fence_tag`
/// already pairs `docker`/`dockerfile` and `csharp`/`cs`/`c#`: one language, more than one
/// conventional spelling.
///
/// Deliberately not folded into `Language` itself: that enum also drives snippet *discovery*
/// and *coverage* accounting (`snippets::discovery`, `snippets::gaps::missing_language_variants`),
/// and giving these markup/config/build languages a `Language` variant would pull them into
/// coverage bookkeeping meant for binding-target and structured-data languages. This vocabulary
/// answers exactly one question -- does this fence label deserve a finding -- and nothing else
/// reads it. ~keep
fn is_recognized_display_language(tag: &str) -> bool {
    tag.split(',').map(str::trim).any(|token| {
        matches!(
            token.to_lowercase().as_str(),
            "apache"
                | "cmake"
                | "css"
                | "csv"
                | "d2"
                | "diff"
                | "dot"
                | "env"
                | "gql"
                | "gradle"
                | "graphql"
                | "graphviz"
                | "groovy"
                | "htm"
                | "html"
                | "ini"
                | "latex"
                | "log"
                | "make"
                | "makefile"
                | "markdown"
                | "md"
                | "mdx"
                | "nginx"
                | "output"
                | "patch"
                | "plaintext"
                | "plantuml"
                | "properties"
                | "rst"
                | "sass"
                | "scss"
                | "sql"
                | "svg"
                | "tex"
                | "tsv"
        )
    })
}

/// Classify a fence tag that failed to resolve to a concrete [`Language`], and decide whether
/// that failure is worth an audit finding at all.
///
/// Three outcomes, in priority order: a tag that claims a real binding-target language and
/// gets it wrong is an error (someone leaked or mistyped a target-language fence); a tag this
/// module recognizes as a standard, unvalidated display language is not a finding at all; and
/// everything else -- genuinely unidentified, which is indistinguishable from a typo -- is a
/// warning. Returns `None` only for the middle case. ~keep
fn unrecognized_fence_finding(tag: &str) -> Option<(AuditIssueKind, String)> {
    if Language::from_fence_info(tag) != Language::Unknown {
        return None;
    }
    if tag_claims_a_binding_target_language(tag) {
        return Some((
            AuditIssueKind::UnknownLanguage,
            format!("unknown fenced code language: {tag}"),
        ));
    }
    if is_recognized_display_language(tag) {
        return None;
    }
    Some((
        AuditIssueKind::UnrecognizedFenceLanguage,
        format!(
            "fenced code language `{tag}` names no binding target and is not a recognized \
             display tag; it will not be validated. If that is a typo, correct it -- if it \
             is prose (`astro`, `hcl`, ...), this line is informational."
        ),
    ))
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

    /// task: a human-authored docs page may legitimately fence a language alef does not
    /// generate bindings for (`astro`, `mdx`, `hcl`, ...). Such a fence is prose decoration
    /// and must never fail `alef all`'s docs/snippet validation just because nobody happened
    /// to add its tag to a hand-maintained allowlist.
    /// The reason the prose-fence allowance cannot be silent. `pythn` is a typo for a real
    /// binding target, but nothing distinguishes it from a legitimate prose vocabulary without
    /// an allowlist of every language in existence -- so it must surface as a warning rather
    /// than pass unnoticed and make snippet validation falsely green. ~keep
    #[test]
    fn a_typo_of_a_real_language_still_surfaces_as_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("guide.md"), "```pythn\nprint(1)\n```\n").unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: vec![docs],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
            ..AuditConfig::default()
        });

        assert_eq!(
            report.issues.iter().map(|issue| &issue.kind).collect::<Vec<_>>(),
            vec![&AuditIssueKind::UnrecognizedFenceLanguage],
            "a typo'd language tag must be reported: {:?}",
            report.issues
        );
        assert!(
            report.issues[0].message.contains("pythn"),
            "the diagnostic must name the offending tag so it is actionable: {:?}",
            report.issues
        );
        assert!(
            !report.has_errors(),
            "but it must not fail the run: {:?}",
            report.issues
        );
    }

    #[test]
    fn unknown_fence_language_that_claims_no_binding_target_warns_but_does_not_fail() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("development.md"), "```astro\n<Foo client:load />\n```\n").unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: vec![docs],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
            ..AuditConfig::default()
        });

        assert!(
            !report.has_errors(),
            "an `astro` fence names no language alef targets, so it must never fail the run: {:?}",
            report.issues
        );
        assert_eq!(
            report.issues.iter().map(|issue| &issue.kind).collect::<Vec<_>>(),
            vec![&AuditIssueKind::UnrecognizedFenceLanguage],
            "it must still be reported at warning severity -- staying silent is what let a typo \
             like `pythn` make validation falsely green: {:?}",
            report.issues
        );
    }

    /// Control, paired with the test above: the audit must still flag a fence info string
    /// that genuinely claims a real binding-target language and gets it wrong -- for example
    /// a leaked rustdoc-attribute-style fence combining `rust` with a token that is not a
    /// recognized rustdoc doctest attribute either. This is the "prove the check still
    /// fires" half required alongside the astro case: without it, the fix above would pass
    /// equally well with `UnknownLanguage` disabled outright.
    #[test]
    fn fence_language_that_claims_a_binding_target_still_fails() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("api-rust.md"),
            "```rust,definitely_bogus\nfn main() {}\n```\n",
        )
        .unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: vec![docs],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
            ..AuditConfig::default()
        });

        assert_eq!(report.issues.len(), 1, "issues: {:?}", report.issues);
        assert_eq!(report.issues[0].kind, AuditIssueKind::UnknownLanguage);
        assert_eq!(
            report.issues[0].message,
            "unknown fenced code language: rust,definitely_bogus"
        );
    }

    /// task: fixing the false positive on decorative tags (`astro`) must not disable real
    /// structural validation for a fence that DOES claim a binding-target language. A
    /// `python`-tagged fence with no closing fence is exactly the kind of broken snippet the
    /// audit must still catch -- if this stopped failing, the fix above would have passed
    /// equally well with the whole fence audit silently disabled.
    #[test]
    fn python_tagged_fence_with_broken_snippet_still_fails() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("usage.md"), "```python\nprint('unterminated'\n").unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: vec![docs],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
            ..AuditConfig::default()
        });

        assert_eq!(report.issues.len(), 1, "issues: {:?}", report.issues);
        assert_eq!(report.issues[0].kind, AuditIssueKind::BrokenFence);
    }

    /// task #560: every one of these is a real, standard documentation fence label alef simply
    /// has no validator for -- not an unknown language. Each must audit clean with zero
    /// findings, not merely a downgraded warning: recognizing a label and being unable to
    /// validate it are different facts, and only the first one belongs in this table.
    ///
    /// The table is the `is_known_display_tag` vocabulary `097724925` deleted, minus the names
    /// `Language::from_fence_tag` resolves on its own, plus `htm`. It is written out in full
    /// rather than looped over the function's own `matches!` arms so that deleting an arm fails
    /// here instead of silently agreeing with itself.
    #[test]
    fn recognized_display_languages_audit_clean() {
        let recognized = [
            "apache",
            "cmake",
            "css",
            "csv",
            "d2",
            "diff",
            "dot",
            "env",
            "gql",
            "gradle",
            "graphql",
            "graphviz",
            "groovy",
            "htm",
            "html",
            "ini",
            "latex",
            "log",
            "make",
            "makefile",
            "markdown",
            "md",
            "mdx",
            "nginx",
            "output",
            "patch",
            "plaintext",
            "plantuml",
            "properties",
            "rst",
            "sass",
            "scss",
            "sql",
            "svg",
            "tex",
            "tsv",
        ];
        for tag in recognized {
            let dir = tempfile::tempdir().unwrap();
            let docs = dir.path().join("docs");
            std::fs::create_dir_all(&docs).unwrap();
            std::fs::write(docs.join("guide.md"), format!("```{tag}\nexample\n```\n")).unwrap();

            let report = audit(&AuditConfig {
                docs_dirs: vec![docs],
                snippet_dirs: Vec::new(),
                require_frontmatter: false,
                ..AuditConfig::default()
            });

            assert_eq!(
                report.issues,
                Vec::new(),
                "`{tag}` is a real, standard fence label alef cannot validate but must \
                 recognize -- it must produce no finding at all: {:?}",
                report.issues
            );
        }
    }

    /// Control, paired with the table above: recognizing the standard display vocabulary must
    /// not widen into accepting every string. A bogus label that resembles none of the
    /// recognized display languages, and claims no binding target either, must still surface a
    /// warning -- otherwise this fix would pass equally well with the audit accepting anything,
    /// which is exactly the "acknowledge the false positive" outcome the task rejected.
    #[test]
    fn a_bogus_label_resembling_no_recognized_language_still_warns() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("guide.md"), "```pyhton\nprint(1)\n```\n").unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: vec![docs],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
            ..AuditConfig::default()
        });

        assert_eq!(
            report.issues.iter().map(|issue| &issue.kind).collect::<Vec<_>>(),
            vec![&AuditIssueKind::UnrecognizedFenceLanguage],
            "a bogus tag must still be reported, or a real typo could pass unnoticed: {:?}",
            report.issues
        );
        assert!(
            report.issues[0].message.contains("pyhton"),
            "the diagnostic must name the offending tag so it is actionable: {:?}",
            report.issues
        );
        assert!(
            !report.has_errors(),
            "an unrecognized tag warns rather than fails the run: {:?}",
            report.issues
        );
    }
}
