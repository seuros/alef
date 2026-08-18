use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::{E2eConfig, SnippetConfig};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::codegen::{E2eCodegen, all_generators};
use crate::e2e::fixture::{Fixture, FixtureDocs, SideEffectClass};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub mod coverage;
pub(crate) mod ledger_paths;
pub mod migration;
pub mod ownership;
mod recipe_policy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnippetInclusion {
    Include,
    Exclude { missing_requirements: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct GeneratedSnippet {
    pub file: GeneratedFile,
    pub fixture_id: String,
    pub fixture_source: String,
    pub language: String,
    pub requirements: Vec<String>,
    pub side_effects: SideEffectClass,
}

pub const COVERAGE_MANIFEST: &str = ".alef-snippet-coverage.json";
pub const COVERAGE_MANIFEST_VERSION: u32 = 2;

/// True when `path`'s file name is alef's own snippet-coverage ledger.
///
/// The ledger is strict JSON, so it can never carry an `alef:hash:` provenance marker, and
/// `write_scaffold_files_report`'s write-time ownership guard falls back to the committed
/// `.alef-ownership.toml` record for exactly that reason. That record is populated by the
/// guard itself the first time it *creates* a path, but a ledger written before this
/// mechanism existed — or one whose only prior writes happened to leave content
/// byte-identical to disk, which records nothing by design (byte-equality is never proof of
/// authorship) — reaches this guard already `exists()` and unrecorded, and is refused
/// forever: refusing means the write never happens, so the record that would unblock the
/// *next* write is never established either.
///
/// This is deliberately not folded into a general "unmarkable extension" filename
/// allowlist the way `orphans.rs`'s `UNMARKABLE_ALEF_MANIFESTS` covers
/// `composer.json`/`package.json` for orphan reclaim: those names a human plausibly authors
/// independently of alef, so trusting the name alone there would risk silently overwriting
/// hand-written content. This ledger's dotfile name has no meaning or use to anything but
/// alef's own coverage bookkeeping — nothing else ever reads or writes it — so a name match
/// here is sufficient proof of exclusive alef authorship without weakening the guard's
/// protection for any other path. Consulted from
/// `cli::pipeline::generate::scaffold::write_scaffold_files_report`'s ownership check; once
/// it lets the write through once, the guard's own write-time registration records the path
/// durably and this predicate is never needed again for that tree. ~keep
pub fn is_snippet_coverage_manifest_path(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some(COVERAGE_MANIFEST)
}

/// Requirement namespace for a Cargo crate a generated Rust snippet body names directly. ~keep
/// The Rust snippet validator resolves these into `[dependencies]` of the check project.
pub const CRATE_REQUIREMENT_PREFIX: &str = "crate:";

const SERDE_JSON_REQUIREMENT: &str = "crate:serde_json";

/// `rust/snippet_body.rs.jinja` emits `#[tokio::main]` for an async fixture, so the snippet ~keep
/// carries a tokio dependency the fixture's own config never declares. Without this requirement
/// the check project has no `tokio` in `[dependencies]` and every async Rust snippet fails to
/// resolve the attribute macro (E0433) before any of its actual content is checked.
const TOKIO_REQUIREMENT: &str = "crate:tokio";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SnippetCoverageKey {
    pub fixture_id: String,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedSnippetMetadata {
    pub key: SnippetCoverageKey,
    pub path: PathBuf,
    pub language: String,
    pub target: String,
    pub session: String,
    pub requires: Vec<String>,
    pub side_effect: SideEffectClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingSnippet {
    pub key: SnippetCoverageKey,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentedSnippetException {
    pub key: SnippetCoverageKey,
    pub reason: String,
    pub reference: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetCoverageLedger {
    #[serde(default)]
    pub format_version: u32,
    #[serde(default)]
    pub generated_paths: Vec<PathBuf>,
    #[serde(default)]
    pub generated_metadata: Vec<GeneratedSnippetMetadata>,
    pub expected: Vec<SnippetCoverageKey>,
    pub generated: Vec<SnippetCoverageKey>,
    pub missing: Vec<MissingSnippet>,
    pub documented_exceptions: Vec<DocumentedSnippetException>,
}

/// A snippet body the mock-harness guard refused, kept out of the coverage ledger. ~keep
///
/// This deliberately does not live on [`SnippetCoverageLedger`]: the ledger is the
/// serialized manifest, and a guard rejection is never a durable state a run may come to
/// rest in — it aborts generation. Carrying it on the in-memory report instead keeps the
/// on-disk manifest format unchanged while still giving the caller per-language,
/// per-marker attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetGuardRejection {
    pub key: SnippetCoverageKey,
    pub marker: String,
}

/// The guard's typed failure, so a caller can tell "this body leaked harness scaffolding" ~keep
/// apart from every other reason a recipe can fail to render. Without the type the two are
/// indistinguishable strings, and a `coverage_exceptions` entry authored for an unrelated
/// capability gap silently absorbs a leak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockHarnessLeak {
    pub marker: String,
    pub fixture_id: String,
    pub language: String,
}

impl std::fmt::Display for MockHarnessLeak {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "`{}` snippet for fixture `{}` leaks e2e mock-server scaffolding (`{}`); \
             a documentation snippet must construct its client the way a reader would",
            self.language, self.fixture_id, self.marker
        )
    }
}

impl std::error::Error for MockHarnessLeak {}

#[derive(Debug, Clone, Default)]
pub struct SnippetGenerationReport {
    pub snippets: Vec<GeneratedSnippet>,
    pub coverage: SnippetCoverageLedger,
    /// Always empty on a successful run: a non-empty value aborts generation. ~keep
    pub guard_rejections: Vec<SnippetGuardRejection>,
}

struct SnippetRenderContext<'a> {
    e2e: &'a E2eConfig,
    crate_config: &'a ResolvedCrateConfig,
    type_defs: &'a [TypeDef],
    enums: &'a [EnumDef],
    functions: &'a [crate::core::ir::FunctionDef],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentationLanguage {
    Binding(Language),
    Shell,
}

impl DocumentationLanguage {
    fn code_fence(self) -> &'static str {
        match self {
            Self::Binding(language) => crate::docs::naming::lang_code_fence(language),
            Self::Shell => "bash",
        }
    }

    fn canonical_name(self) -> &'static str {
        self.code_fence()
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Binding(language) => crate::docs::naming::lang_display_name(language),
            Self::Shell => "Shell",
        }
    }
}

#[expect(clippy::too_many_arguments, reason = "preserves the public snippet generation API")]
pub fn generate_snippets(
    fixtures: &[Fixture],
    languages: &[String],
    e2e: &E2eConfig,
    snippets: &SnippetConfig,
    crate_config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<Vec<GeneratedFile>> {
    Ok(generate_snippet_report(
        fixtures,
        languages,
        e2e,
        snippets,
        crate_config,
        type_defs,
        enums,
        functions,
    )?
    .snippets
    .into_iter()
    .map(|snippet| snippet.file)
    .collect())
}

#[expect(clippy::too_many_arguments, reason = "preserves the public snippet generation API")]
pub fn generate_snippet_report(
    fixtures: &[Fixture],
    languages: &[String],
    e2e: &E2eConfig,
    snippets: &SnippetConfig,
    crate_config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<SnippetGenerationReport> {
    crate::with_extensions(|extensions| {
        let context = SnippetRenderContext {
            e2e,
            crate_config,
            type_defs,
            enums,
            functions,
        };
        generate_snippet_report_with_extensions(fixtures, languages, snippets, &context, extensions)
    })
}

fn generate_snippet_report_with_extensions(
    fixtures: &[Fixture],
    languages: &[String],
    snippets: &SnippetConfig,
    context: &SnippetRenderContext<'_>,
    extensions: &[Box<dyn crate::Extension>],
) -> Result<SnippetGenerationReport> {
    validate_relative_path(Path::new(&snippets.output), "snippet output")?;
    // Pin the *previous* run's ownership record before this run computes, let alone writes,
    // anything. `e2e::run` hands the freshly computed ledger to the same write batch as the
    // snippets, and `.alef-snippet-coverage.json` sorts ahead of every sibling snippet directory
    // in that batch's `BTreeMap`, so reading it any later would read this run's intentions and
    // silently degrade `ownership::is_ledger_owned_snippet_path` to bare path identity. ~keep
    ownership::snapshot_pre_run_ledger(Path::new(&snippets.output));
    let generators = snippet_generators(languages)?;
    let mut generated = BTreeMap::<PathBuf, GeneratedSnippet>::new();
    let mut guard_rejections = Vec::<SnippetGuardRejection>::new();
    let mut coverage = SnippetCoverageLedger {
        format_version: COVERAGE_MANIFEST_VERSION,
        ..SnippetCoverageLedger::default()
    };
    for fixture in fixtures {
        validate_requirements(fixture)?;
        validate_coverage_exceptions(fixture)?;
        validate_docs_paths(fixture, languages)?;
        for (language, generator) in &generators {
            // A function this language's `exclude_functions` (or the crate-wide
            // `[crates.exclude].functions` that `language_excludes` folds in) drops can
            // never be emitted here, so the cell must not enter `expected` at all --
            // pushing it and then failing to generate it is exactly the ledger/emitter
            // disagreement this check exists to prevent. See
            // `function_excluded_for_language`'s doc comment for why this reuses the
            // docs generator's exclusion accessor instead of re-deriving the rule. ~keep
            if function_excluded_for_language(fixture, language, generator.language_name(), context) {
                continue;
            }
            let key = SnippetCoverageKey {
                fixture_id: fixture.id.clone(),
                language: language.to_string(),
            };
            coverage.expected.push(key.clone());
            let Some(docs) = fixture.docs.as_ref() else {
                coverage.missing.push(MissingSnippet {
                    key,
                    reason: "fixture has no documentation metadata".to_string(),
                });
                continue;
            };
            let capabilities = capabilities(language, snippets, context.crate_config);
            let capability_decision = snippet_inclusion(fixture, &capabilities);
            let exclusion_reason = match &capability_decision {
                SnippetInclusion::Exclude { missing_requirements } => {
                    Some(format!("missing requirements: {}", missing_requirements.join(", ")))
                }
                SnippetInclusion::Include => None,
            };
            if let Some(reason) = exclusion_reason {
                if let Some(exception) = docs.coverage_exceptions.get(*language) {
                    coverage.documented_exceptions.push(DocumentedSnippetException {
                        key,
                        reason: exception.reason.clone(),
                        reference: exception.documentation.clone(),
                    });
                } else {
                    coverage.missing.push(MissingSnippet { key, reason });
                }
                continue;
            }
            let lang = parse_language(generator.language_name()).ok_or_else(|| {
                anyhow::anyhow!(
                    "e2e code generator `{}` has no documentation language mapping",
                    generator.language_name()
                )
            })?;
            let path = snippet_path(&snippets.output, docs, &fixture.id, language, lang)?;
            let body = match render_snippet_body(extensions, generator.as_ref(), fixture, language, context) {
                Ok(body) => body,
                Err(error) => {
                    // A guard rejection is a generator defect, not a documented limitation, so ~keep
                    // it is recorded separately and is deliberately *not* eligible for the
                    // coverage-exception branch below. Routing it there is what turned a
                    // rejected snippet into a silent deletion.
                    if let Some(leak) = error.downcast_ref::<MockHarnessLeak>() {
                        guard_rejections.push(SnippetGuardRejection {
                            key: key.clone(),
                            marker: leak.marker.clone(),
                        });
                        coverage.missing.push(MissingSnippet {
                            key,
                            reason: format!("{error:#}"),
                        });
                        continue;
                    }
                    if let Some(exception) = docs.coverage_exceptions.get(*language) {
                        coverage.documented_exceptions.push(DocumentedSnippetException {
                            key,
                            reason: exception.reason.clone(),
                            reference: exception.documentation.clone(),
                        });
                    } else {
                        coverage.missing.push(MissingSnippet {
                            key,
                            reason: format!("{error:#}"),
                        });
                    }
                    continue;
                }
            };
            let content = render_snippet_markdown(&body, fixture, docs, language, lang);
            let requirements = snippet_requirements(fixture, language, &body);
            let file = GeneratedFile {
                path: path.clone(),
                content,
                generated_header: false,
            };
            let snippet = GeneratedSnippet {
                file,
                fixture_id: fixture.id.clone(),
                fixture_source: fixture.source.clone(),
                language: language.to_string(),
                requirements: requirements.clone(),
                side_effects: docs.side_effects,
            };
            if generated.insert(path.clone(), snippet).is_some() {
                bail!("snippet output collision at {}", path.display());
            }
            let relative_path = path
                .strip_prefix(&snippets.output)
                .context("generated snippet path escaped the configured output root")?
                .to_path_buf();
            coverage.generated_paths.push(relative_path.clone());
            coverage.generated_metadata.push(GeneratedSnippetMetadata {
                key: key.clone(),
                path: relative_path,
                language: lang.canonical_name().to_string(),
                target: language.to_string(),
                session: language.to_string(),
                requires: requirements,
                side_effect: docs.side_effects,
            });
            coverage.generated.push(key);
        }
    }
    // Abort before the caller reports coverage, prunes orphans, or writes anything: a run ~keep
    // that deleted the stale files and *then* failed would still have destroyed published
    // documentation.
    guard_rejections.sort_by(|left, right| left.key.cmp(&right.key));
    ensure_no_guard_rejections(&guard_rejections)?;
    coverage = coverage::normalize(coverage);
    coverage::validate(&coverage)?;
    Ok(SnippetGenerationReport {
        snippets: generated.into_values().collect(),
        coverage,
        guard_rejections,
    })
}

fn validate_coverage_exceptions(fixture: &Fixture) -> Result<()> {
    let Some(docs) = &fixture.docs else {
        return Ok(());
    };
    for (language, exception) in &docs.coverage_exceptions {
        if language.trim().is_empty() || exception.reason.trim().is_empty() {
            bail!(
                "fixture `{}` has invalid coverage exception for language `{language}`: language and reason must be non-empty",
                fixture.id
            );
        }
        validate_documentation_reference(&exception.documentation).map_err(|error| {
            anyhow::anyhow!(
                "fixture `{}` has invalid coverage exception documentation for language `{language}`: {error}",
                fixture.id
            )
        })?;
    }
    Ok(())
}

fn validate_documentation_reference(reference: &str) -> Result<()> {
    if reference.trim() != reference || reference.is_empty() {
        bail!("reference must be non-empty and have no surrounding whitespace");
    }
    if reference.starts_with("https://") || reference.starts_with("http://") {
        if reference.chars().any(char::is_whitespace) {
            bail!("URL reference must not contain whitespace");
        }
        return Ok(());
    }
    validate_relative_path(Path::new(reference), "documentation reference")
}

fn render_snippet_body(
    extensions: &[Box<dyn crate::Extension>],
    generator: &dyn E2eCodegen,
    fixture: &Fixture,
    language: &str,
    context: &SnippetRenderContext<'_>,
) -> Result<String> {
    let docs_fixture = fixture.docs_call_fixture();
    let fixture = &docs_fixture;
    for extension in extensions {
        if let Some(body) = extension
            .render_e2e_snippet(
                fixture,
                context.e2e,
                context.crate_config,
                language,
                context.type_defs,
                context.enums,
            )
            .map_err(|error| anyhow::anyhow!("extension `{}` could not render snippet: {error:#}", extension.name()))?
        {
            if body.trim().is_empty() {
                bail!("extension `{}` returned an empty snippet body", extension.name());
            }
            reject_mock_harness_scaffolding(&body, fixture, language)?;
            return Ok(body);
        }
    }
    let call = context.e2e.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    if let Some(kind) = recipe_policy::extension_owned_recipe_kind(fixture, fixture.resolved_args(call)) {
        bail!("{kind} fixture requires an extension-owned documentation recipe");
    }
    let effective_function = call
        .effective_function(language)
        .or_else(|| {
            // The naive identity fallback below derives a symbol name from the raw
            // `fixture.call` config text (`register_fn`/`unregister_fn`/`clear_fn`), which
            // can diverge from what the FFI backend actually generates (see
            // `trait_bridge_function_identity`'s doc comment). A fixture author who set
            // `skip.languages` for this language has already declared that the harness
            // (and by extension this naive fallback) cannot speak for it here; only trust
            // the fallback when the fixture is not skipped for this language. Fixtures with
            // a real, extension-owned recipe never reach this branch — the extension loop
            // above already returned their body, and `recipe_policy::extension_owned_recipe_kind`
            // already bailed for fixtures that require one but lack it.
            let skipped_for_language = fixture.skip.as_ref().is_some_and(|skip| skip.should_skip(language));
            (!skipped_for_language && matches!(language, "c" | "c_ffi" | "ffi"))
                .then(|| crate::e2e::codegen::recipe::trait_bridge_function_identity(context.crate_config, fixture))
                .flatten()
        })
        .unwrap_or_default();
    if effective_function.trim().is_empty() {
        bail!(
            "built-in `{language}` snippet recipe has no function identity; configure a call function or provide an extension-owned documentation recipe"
        );
    }
    let body = generator
        .render_snippet_body_with_functions(
            fixture,
            context.e2e,
            context.crate_config,
            context.type_defs,
            context.enums,
            context.functions,
        )
        .map_err(|error| anyhow::anyhow!("built-in `{language}` snippet recipe is incompatible: {error:#}"))?;
    if body.trim().is_empty() {
        bail!("built-in `{language}` snippet recipe returned an empty body");
    }
    reject_mock_harness_scaffolding(&body, fixture, language)?;
    Ok(body)
}

/// Substrings that only ever appear in e2e mock-server wiring.
///
/// Each is a name the harness itself owns: the environment variables the mock server
/// exports (`MOCK_SERVER_URL`, `MOCK_SERVERS`, the per-fixture `MOCK_SERVER_<ID>`) and
/// the JVM system properties the Java/Kotlin suites read them through.
const MOCK_HARNESS_MARKERS: &[&str] = &[
    "MOCK_SERVER_URL",
    "MOCK_SERVERS",
    "MOCK_SERVER_",
    "mockServerUrl",
    "mockServer.",
];

/// Reject a snippet body that carries e2e mock-server scaffolding.
///
/// Snippet bodies are published verbatim into the docs site, so a body that still points
/// at the mock server documents the test harness rather than the library. Every language
/// — built-in or extension-supplied — funnels through [`render_snippet_body`], so placing
/// the check here means a new backend inherits the guarantee instead of having to
/// re-derive it. The `Err` carries a typed [`MockHarnessLeak`] so the caller can route it
/// to a hard, attributed failure rather than to a coverage gap that a `coverage_exceptions`
/// entry would silently absorb.
fn reject_mock_harness_scaffolding(body: &str, fixture: &Fixture, language: &str) -> Result<()> {
    let fixture_route = format!("/fixtures/{}", fixture.id);
    let marker = MOCK_HARNESS_MARKERS
        .iter()
        .copied()
        .chain(std::iter::once(fixture_route.as_str()))
        .find(|marker| body.contains(marker));
    if let Some(marker) = marker {
        return Err(anyhow::Error::new(MockHarnessLeak {
            marker: marker.to_string(),
            fixture_id: fixture.id.clone(),
            language: language.to_string(),
        }));
    }
    Ok(())
}

/// Turn every guard rejection this run produced into one aborting, attributed error. ~keep
///
/// A rejection must never come to rest as a coverage gap: `missing` cells can be retired by
/// writing a `docs.coverage_exceptions` entry, and an exception authored for an unrelated
/// capability gap would then also retire a leak — deleting the snippet from the docs tree
/// with no signal at all. Failing here, before the caller prunes orphans or writes any
/// file, is what makes the denylist's failure mode loud instead of a silent deletion.
fn ensure_no_guard_rejections(rejections: &[SnippetGuardRejection]) -> Result<()> {
    if rejections.is_empty() {
        return Ok(());
    }
    let mut by_language: BTreeMap<&str, BTreeMap<&str, Vec<&str>>> = BTreeMap::new();
    for rejection in rejections {
        by_language
            .entry(rejection.key.language.as_str())
            .or_default()
            .entry(rejection.marker.as_str())
            .or_default()
            .push(rejection.key.fixture_id.as_str());
    }
    let mut detail = String::new();
    for (language, markers) in &by_language {
        let language_total: usize = markers.values().map(Vec::len).sum();
        detail.push_str(&format!("\n  {language} ({language_total}):"));
        for (marker, fixtures) in markers {
            detail.push_str(&format!(
                "\n    `{marker}` ({}): {}",
                fixtures.len(),
                fixtures.join(", ")
            ));
        }
    }
    bail!(
        "{} documentation snippet(s) were rejected by the mock-harness guard; each would otherwise \
         disappear from the docs tree with no report. Fix the generator so the snippet constructs \
         its client the way a reader would — a `docs.coverage_exceptions` entry cannot retire a \
         guard rejection.{detail}",
        rejections.len()
    )
}

fn snippet_generators(languages: &[String]) -> Result<Vec<(&str, Box<dyn E2eCodegen>)>> {
    let mut available = BTreeMap::new();
    for generator in all_generators() {
        let name = generator.language_name();
        if available.insert(name, generator).is_some() {
            bail!("duplicate e2e code generator registered for snippet language `{name}`");
        }
    }
    let mut requested = BTreeSet::new();
    languages
        .iter()
        .map(|language| {
            let generator_name = generator_name(language);
            if !requested.insert(generator_name) {
                bail!("duplicate snippet language resolves to e2e code generator `{generator_name}`");
            }
            available
                .remove(generator_name)
                .map(|generator| (language.as_str(), generator))
                .ok_or_else(|| anyhow::anyhow!("no e2e code generator registered for snippet language `{language}`"))
        })
        .collect()
}

fn generator_name(language: &str) -> &str {
    match language {
        "core" | "rust_core" => "rust",
        "c_ffi" | "ffi" => "c",
        other => other,
    }
}

/// The CLI invocation that produces fixture snippets, embedded verbatim in every
/// snippet's provenance header by [`render_snippet_markdown`].
const SNIPPET_REGENERATE_COMMAND: &str = "alef e2e generate";

/// Render one fixture snippet as a self-marking Markdown document.
///
/// The provenance header comes from [`crate::docs::with_html_header`] — the same emitter
/// `readme::template` and `docs::render` use — rather than a second marker producer, so the
/// bytes the ownership guard reads back are byte-identical across every `.md` alef writes.
///
/// Why it is needed here at all: `write::marker_comment_style`'s doc excludes `.md` on the
/// stated grounds that "`readme::template` and `docs::render` both route content through
/// `docs::render::with_html_header`". That is true of READMEs and docs pages and false of
/// fixture snippets, which are assembled here and never touch `docs::render`. The result was
/// a `.md` that no side stamps: `generated_header` is `false`, `marker_header_syntax` is
/// `None` for `.md`, and the snippet output root sits outside any path
/// `cache::record_scaffold_owned_path` had recorded — so once a snippet existed on disk the
/// write guard could prove nothing and refused it forever (15,677 refusals in one consumer
/// repo, 9,139 in another). ~keep
///
/// Placement is load-bearing and has no slack. `with_html_header` puts the marker after the
/// YAML front matter (it must: Astro/Starlight imports these files as content and requires the
/// opening `---` to be the first bytes) with one blank line between, so with
/// `snippets/file.md.jinja`'s 8-line front matter the marker lands on line 10 — the last line
/// `hash::content_has_alef_marker`'s 10-line scan window reads. **Adding a ninth front-matter
/// line pushes the marker out of that window and silently restores the deadlock**; the marker
/// would still be in the file and nothing would read it. `snippet_marker_lands_inside_the_read_side_scan_window`
/// and its control fail if that budget is spent. ~keep
fn render_snippet_markdown(
    body: &str,
    fixture: &Fixture,
    docs: &FixtureDocs,
    target: &str,
    language: DocumentationLanguage,
) -> String {
    let snippet_id = format!("fixture_{target}_{}", fixture.id);
    let requirements = snippet_requirements(fixture, target, body);
    let requires = serde_json::to_string(&requirements).unwrap_or_else(|_| "[]".to_string());
    let rendered = crate::e2e::template_env::render(
        "snippets/file.md.jinja",
        minijinja::context! {
            description => docs.description.as_deref().unwrap_or(&fixture.description),
            fence => language.code_fence(),
            id => snippet_id,
            language => language.canonical_name(),
            level => level_stamp(docs.side_effects),
            requires => requires,
            side_effect => side_effect_name(docs.side_effects),
            target => target,
            title => language.display_name(),
            body => body,
        },
    );
    crate::docs::with_html_header(rendered, SNIPPET_REGENERATE_COMMAND)
}

fn snippet_requirements(fixture: &Fixture, target: &str, body: &str) -> Vec<String> {
    let mut requirements = fixture.requirements.clone();
    if target == "rust" && fixture.visitor.is_some() && !requirements.iter().any(|value| value == "feature:visitor") {
        requirements.push("feature:visitor".to_string());
    }
    if generator_name(target) == "rust"
        && body.contains("serde_json::")
        && !requirements.iter().any(|value| value == SERDE_JSON_REQUIREMENT)
    {
        requirements.push(SERDE_JSON_REQUIREMENT.to_string());
    }
    if generator_name(target) == "rust"
        && body.contains("#[tokio::main]")
        && !requirements.iter().any(|value| value == TOKIO_REQUIREMENT)
    {
        requirements.push(TOKIO_REQUIREMENT.to_string());
    }
    requirements
}

/// The front-matter `level` a generated snippet declares. `effective_validation_level`
/// (`src/snippets/runner.rs`) folds this with the requested level by `min`, so any concrete
/// value here can only ever lower validation, never raise it.
///
/// `94d09809d` ("fix(e2e): typecheck fixture snippets") made this stamp unconditional, replacing
/// a `syntax` ceiling with `typecheck` specifically because fixtures with side effects the e2e
/// harness cannot safely execute unattended (network calls, process/install/server side effects)
/// were being validated no deeper than syntax. That protection is still needed for exactly those
/// fixtures. It was never needed for `Safe` ones, and stamping them anyway silently capped every
/// generated snippet at `typecheck` regardless of what the workspace and the snippet's own
/// capabilities could actually support. A `Safe` snippet renders `level: null` — parsed back as
/// `SnippetMetadata::level == None` — so it has nothing to fold against `requested` and validates
/// at whatever level the workspace and validator achieve on their own. ~keep
fn level_stamp(side_effects: SideEffectClass) -> &'static str {
    if side_effects == SideEffectClass::Safe {
        "null"
    } else {
        "typecheck"
    }
}

fn side_effect_name(side_effect: SideEffectClass) -> &'static str {
    match side_effect {
        SideEffectClass::Safe => "safe",
        SideEffectClass::Network => "network",
        SideEffectClass::Process => "process",
        SideEffectClass::Install => "install",
        SideEffectClass::Server => "server",
    }
}

fn validate_requirements(fixture: &Fixture) -> Result<()> {
    for requirement in &fixture.requirements {
        let valid = requirement.split_once(':').is_some_and(|(kind, value)| {
            matches!(kind, "feature" | "model" | "service" | "credential") && !value.is_empty() && !value.contains(':')
        });
        if !valid {
            bail!("fixture `{}` has invalid requirement token `{requirement}`", fixture.id);
        }
    }
    Ok(())
}

pub fn snippet_inclusion(fixture: &Fixture, capabilities: &BTreeSet<String>) -> SnippetInclusion {
    let missing_requirements: Vec<_> = fixture
        .requirements
        .iter()
        .filter(|requirement| !capabilities.contains(*requirement))
        .cloned()
        .collect();
    if missing_requirements.is_empty() {
        SnippetInclusion::Include
    } else {
        SnippetInclusion::Exclude { missing_requirements }
    }
}

fn capabilities(language: &str, snippets: &SnippetConfig, crate_config: &ResolvedCrateConfig) -> BTreeSet<String> {
    let mut values = snippets.capabilities.for_language(language);
    values.extend(crate_config.features.iter().map(|feature| format!("feature:{feature}")));
    values
}

/// Whether the function a fixture's call resolves to for `language` is excluded for that
/// language, and therefore can never be rendered into a snippet.
///
/// Reuses [`crate::docs::language_pages::excludes::language_excludes`] -- the accessor the
/// docs generator already consults for the same question -- rather than re-deriving the
/// per-language `exclude_functions` union here. A second copy of that rule is exactly how a
/// ledger and its emitter drift apart: one path evolves (a language gains an override, a new
/// per-language config field is added) and the other silently keeps checking the old shape.
/// [`CallConfig::core_lookup_name`] gives the Rust-spelled identity `exclude_functions`
/// entries are keyed by, matching every built-in snippet recipe's own resolution (see
/// `e2e/codegen/go/snippet.rs`, `kotlin/snippet.rs`, `php/snippet.rs`, `ruby/snippet.rs`, and
/// the WASM-specific `rust_identity_for_wasm_symbol`, which resolves the same identity for the
/// one target that also accepts the JS spelling of an override). ~keep
fn function_excluded_for_language(
    fixture: &Fixture,
    language: &str,
    generator_language_name: &str,
    context: &SnippetRenderContext<'_>,
) -> bool {
    let Some(DocumentationLanguage::Binding(lang)) = parse_language(generator_language_name) else {
        return false;
    };
    let docs_fixture = fixture.docs_call_fixture();
    let call = context.e2e.resolve_call_for_fixture(
        docs_fixture.call.as_deref(),
        &docs_fixture.id,
        &docs_fixture.resolved_category(),
        &docs_fixture.tags,
        &docs_fixture.input,
    );
    let Some(function_name) = call.core_lookup_name(language) else {
        return false;
    };
    let (excluded_functions, _) = crate::docs::language_pages::excludes::language_excludes(context.crate_config, lang);
    excluded_functions.contains(function_name.as_ref())
}

fn snippet_path(
    output: &str,
    docs: &FixtureDocs,
    fixture_id: &str,
    target_language: &str,
    language: DocumentationLanguage,
) -> Result<PathBuf> {
    if let Some(relative) = docs.paths.get(target_language) {
        let relative = Path::new(relative);
        validate_relative_path(relative, "fixture docs target path")?;
        if relative.extension().and_then(|value| value.to_str()) != Some("md") {
            bail!("fixture docs target path must end in .md: {}", relative.display());
        }
        return Ok(Path::new(output)
            .join(snippet_output_slug(target_language, language))
            .join(relative));
    }
    validate_component(&docs.topic, "snippet topic")?;
    let stem = docs.stem.as_deref().unwrap_or(fixture_id);
    validate_component(stem, "snippet stem")?;
    Ok(Path::new(output)
        .join(snippet_output_slug(target_language, language))
        .join(&docs.topic)
        .join(format!("{stem}.md")))
}

fn snippet_output_slug(target_language: &str, language: DocumentationLanguage) -> &'static str {
    match target_language {
        "node" => "typescript",
        "wasm" => "wasm",
        "kotlin_android" => "kotlin-android",
        "brew" => "brew",
        "homebrew" => "homebrew",
        _ => language.canonical_name(),
    }
}

fn validate_component(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || Path::new(value).components().count() != 1 || matches!(value, "." | "..") {
        bail!("unsafe {label} `{value}`");
    }
    Ok(())
}

fn validate_relative_path(path: &Path, label: &str) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("{label} must be a safe relative path: {}", path.display());
    }
    Ok(())
}

fn validate_docs_paths(fixture: &Fixture, languages: &[String]) -> Result<()> {
    let Some(docs) = &fixture.docs else {
        return Ok(());
    };
    for target in docs.paths.keys() {
        if !languages.iter().any(|language| language == target) {
            bail!(
                "fixture `{}` docs path targets unconfigured language `{target}`",
                fixture.id
            );
        }
    }
    Ok(())
}

fn parse_language(value: &str) -> Option<DocumentationLanguage> {
    let language = match value {
        "python" => Language::Python,
        "node" => Language::Node,
        "wasm" => Language::Wasm,
        "ruby" => Language::Ruby,
        "php" | "php_ext" => Language::Php,
        "elixir" => Language::Elixir,
        "go" => Language::Go,
        "java" => Language::Java,
        "csharp" => Language::Csharp,
        "r" => Language::R,
        "rust" | "rust_core" | "core" => Language::Rust,
        "kotlin" => Language::Kotlin,
        "kotlin_android" => Language::KotlinAndroid,
        "swift" => Language::Swift,
        "dart" => Language::Dart,
        "gleam" => Language::Gleam,
        "zig" => Language::Zig,
        "c" | "ffi" | "c_ffi" => Language::C,
        "brew" | "homebrew" => return Some(DocumentationLanguage::Shell),
        _ => return None,
    };
    Some(DocumentationLanguage::Binding(language))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_snippet_coverage_manifest_path_matches_only_the_exact_ledger_name() {
        assert!(is_snippet_coverage_manifest_path(Path::new(
            "docs/snippets/.alef-snippet-coverage.json"
        )));
        assert!(is_snippet_coverage_manifest_path(Path::new(COVERAGE_MANIFEST)));
        assert!(
            !is_snippet_coverage_manifest_path(Path::new("docs/snippets/.alef-snippet-coverage.json.bak")),
            "a name that merely contains the ledger name must not match"
        );
        assert!(
            !is_snippet_coverage_manifest_path(Path::new("packages/php/composer.json")),
            "an unrelated unmarkable manifest must not match"
        );
    }

    /// The provenance block `docs::render::with_html_header` prepends to every snippet,
    /// spelled out literally so the whole-document assertions below stay whole-document.
    ///
    /// Written out rather than derived from `with_html_header` so the tests fail if that
    /// emitter's text changes: the marker string is what the ownership guard and
    /// `alef verify` match on, so it is a contract with the read side, not an
    /// implementation detail either side may re-spell. ~keep
    const SNIPPET_HEADER: &str = "<!-- This file is auto-generated by alef — DO NOT EDIT. -->\n\
<!-- To regenerate: alef e2e generate -->\n\
<!-- To verify freshness: alef verify -->\n\n";

    struct FixtureExtension {
        body: &'static str,
    }

    impl crate::Extension for FixtureExtension {
        fn name(&self) -> &str {
            "fixture"
        }

        fn render_e2e_snippet(
            &self,
            _fixture: &Fixture,
            _e2e_config: &E2eConfig,
            _config: &ResolvedCrateConfig,
            _language: &str,
            _type_defs: &[TypeDef],
            _enums: &[EnumDef],
        ) -> Result<Option<String>> {
            Ok(Some(self.body.to_string()))
        }
    }

    fn documented_fixture() -> Fixture {
        Fixture {
            id: "extension_owned".into(),
            description: "Extension-owned example".into(),
            docs: Some(FixtureDocs {
                topic: "api".into(),
                stem: None,
                paths: BTreeMap::new(),
                title: None,
                description: None,
                input: None,
                shows: Vec::new(),
                error: None,
                presentation: None,
                client: None,
                side_effects: SideEffectClass::Safe,
                coverage_exceptions: BTreeMap::new(),
            }),
            ..Fixture::default()
        }
    }

    #[test]
    fn mock_harness_scaffolding_is_rejected_for_every_language() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            ..Fixture::default()
        };
        let leaks = [
            "var url = System.getenv(\"MOCK_SERVER_URL\") + \"/fixtures/rate_limit_429\";",
            "let url = std.c.getenv(\"MOCK_SERVER_RATE_LIMIT_429\");",
            "var url = System.getProperty(\"mockServerUrl\");",
            "let base = System.getProperty(\"mockServer.rate_limit_429\");",
            "let hosts = process.env.MOCK_SERVERS;",
            "let url = \"https://api.example.com/fixtures/rate_limit_429\";",
        ];
        for leak in leaks {
            let error = reject_mock_harness_scaffolding(leak, &fixture, "zig")
                .expect_err("mock-server scaffolding must not reach a published snippet");
            let message = format!("{error:#}");
            assert!(message.contains("rate_limit_429"), "error omits the fixture: {message}");
            assert!(message.contains("zig"), "error omits the language: {message}");
        }
    }

    #[test]
    fn a_reader_facing_snippet_passes_the_mock_harness_guard() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            ..Fixture::default()
        };
        let body = "var apiKey = System.getenv(\"API_KEY\");\nvar client = Sample.createClient(apiKey, null);";
        assert!(reject_mock_harness_scaffolding(body, &fixture, "java").is_ok());
    }

    fn snippet_report_for(fixture: Fixture, languages: &[&str], body: &'static str) -> Result<SnippetGenerationReport> {
        let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension { body })];
        let e2e = E2eConfig::default();
        let crate_config = ResolvedCrateConfig::default();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };
        let languages: Vec<String> = languages.iter().map(|language| (*language).to_string()).collect();
        generate_snippet_report_with_extensions(&[fixture], &languages, &snippet_config, &context, &extensions)
    }

    /// The decisive test for the guard's *failure mode*, not for the guard's predicate. ~keep
    ///
    /// Asserting that the rejected snippet is absent from the report would pass whether the
    /// run reported the rejection or dropped it on the floor — that vacuity is precisely how
    /// the silent deletion stayed hidden. So assert the report exists, and that it carries
    /// per-language and per-marker attribution.
    #[test]
    fn a_guard_rejected_snippet_is_a_reported_failure_not_a_silent_absence() {
        let leaking_body = "var url = System.getenv(\"MOCK_SERVER_URL\") + \"/fixtures/extension_owned\";";

        let error = snippet_report_for(documented_fixture(), &["java", "rust"], leaking_body)
            .expect_err("a guard-rejected snippet must abort generation");

        let message = format!("{error:#}");
        assert!(
            message.contains("rejected by the mock-harness guard"),
            "the failure must name the guard: {message}"
        );
        assert!(
            message.contains("2 documentation snippet(s)"),
            "the failure must count every rejection: {message}"
        );
        assert!(
            message.contains("\n  java (1):"),
            "the failure must attribute per language: {message}"
        );
        assert!(
            message.contains("\n  rust (1):"),
            "the failure must attribute per language: {message}"
        );
        assert!(
            message.contains("`MOCK_SERVER_URL` (1): extension_owned"),
            "the failure must attribute per reason and fixture: {message}"
        );
    }

    /// A `docs.coverage_exceptions` entry says "this language cannot express this recipe". ~keep
    /// It must not also retire "this generator emitted harness scaffolding": one is a
    /// documented limitation, the other a defect. Conflating them is what let an exception
    /// authored for an unrelated capability gap delete a snippet with no signal.
    #[test]
    fn a_documented_coverage_exception_cannot_retire_a_guard_rejection() {
        let mut fixture = documented_fixture();
        fixture
            .docs
            .as_mut()
            .expect("documented fixture has docs")
            .coverage_exceptions
            .insert(
                "rust".into(),
                crate::e2e::fixture::SnippetCoverageException {
                    reason: "the sample backend cannot express this recipe".into(),
                    documentation: "docs/limitations.md".into(),
                },
            );
        let leaking_body = "let url = std::env::var(\"MOCK_SERVER_URL\").unwrap();";

        let error = snippet_report_for(fixture, &["rust"], leaking_body)
            .expect_err("a coverage exception must not absorb a guard rejection");

        let message = format!("{error:#}");
        assert!(
            message.contains("rejected by the mock-harness guard"),
            "the exception silently absorbed the rejection: {message}"
        );
        assert!(
            message.contains("cannot retire a guard rejection"),
            "the failure must explain why the exception did not apply: {message}"
        );
    }

    /// Positive control: the guard is armed on the same path, so a clean body must still ~keep
    /// render. Without this, making the guard reject everything would pass the test above.
    #[test]
    fn a_clean_snippet_still_renders_while_the_guard_is_armed() {
        let clean_body =
            "let api_key = std::env::var(\"API_KEY\").unwrap();\nlet client = sample::create_client(api_key)?;";

        let report =
            snippet_report_for(documented_fixture(), &["rust"], clean_body).expect("a clean snippet must still render");

        assert!(report.guard_rejections.is_empty());
        assert!(report.coverage.missing.is_empty());
        assert_eq!(report.coverage.generated, report.coverage.expected);
        assert_eq!(report.snippets.len(), 1);
        assert!(
            report.snippets[0]
                .file
                .content
                .contains("sample::create_client(api_key)?")
        );
    }

    #[test]
    fn capability_decision_reports_missing_requirements_deterministically() {
        let fixture = Fixture {
            requirements: vec!["service:api".into(), "feature:json".into()],
            ..Fixture::default()
        };
        let capabilities = BTreeSet::from(["feature:json".to_string()]);
        assert_eq!(
            snippet_inclusion(&fixture, &capabilities),
            SnippetInclusion::Exclude {
                missing_requirements: vec!["service:api".into()]
            }
        );
    }

    #[test]
    fn snippet_paths_reject_traversal() {
        let mut docs = FixtureDocs {
            topic: "..".into(),
            stem: None,
            paths: BTreeMap::new(),
            title: None,
            description: None,
            input: None,
            shows: Vec::new(),
            error: None,
            presentation: None,
            client: None,
            side_effects: Default::default(),
            coverage_exceptions: BTreeMap::new(),
        };
        assert!(
            snippet_path(
                "docs/snippets",
                &docs,
                "basic",
                "python",
                DocumentationLanguage::Binding(Language::Python)
            )
            .is_err()
        );
        docs.topic = "fallback".into();
        docs.paths.insert("python".into(), "../escape.md".into());
        assert!(
            snippet_path(
                "docs/snippets",
                &docs,
                "basic",
                "python",
                DocumentationLanguage::Binding(Language::Python)
            )
            .is_err()
        );
    }

    #[test]
    fn target_path_override_precedes_topic_and_stem() {
        let docs = FixtureDocs {
            topic: "fallback".into(),
            stem: Some("fallback".into()),
            paths: BTreeMap::from([("node".into(), "config/basic_usage.md".into())]),
            title: None,
            description: None,
            input: None,
            shows: Vec::new(),
            error: None,
            presentation: None,
            client: None,
            side_effects: Default::default(),
            coverage_exceptions: BTreeMap::new(),
        };

        assert_eq!(
            snippet_path(
                "docs/snippets",
                &docs,
                "fixture",
                "node",
                DocumentationLanguage::Binding(Language::Node)
            )
            .expect("safe target path"),
            Path::new("docs/snippets/typescript/config/basic_usage.md")
        );
    }

    #[test]
    fn docs_path_target_must_be_configured() {
        let mut fixture = documented_fixture();
        fixture
            .docs
            .as_mut()
            .expect("fixture docs")
            .paths
            .insert("wasm".into(), "browser/basic.md".into());

        assert!(validate_docs_paths(&fixture, &["node".into()]).is_err());
        assert!(validate_docs_paths(&fixture, &["node".into(), "wasm".into()]).is_ok());
    }

    #[test]
    fn language_aliases_include_core_and_ffi_targets() {
        assert_eq!(
            parse_language("rust_core"),
            Some(DocumentationLanguage::Binding(Language::Rust))
        );
        assert_eq!(parse_language("ffi"), Some(DocumentationLanguage::Binding(Language::C)));
        assert_eq!(parse_language("brew"), Some(DocumentationLanguage::Shell));
        assert_eq!(parse_language("homebrew"), Some(DocumentationLanguage::Shell));
        assert_eq!(generator_name("rust_core"), "rust");
        assert_eq!(generator_name("ffi"), "c");
    }

    #[test]
    fn generated_docs_use_validator_canonical_language_identity() {
        let docs = FixtureDocs {
            topic: "api".into(),
            stem: None,
            paths: BTreeMap::new(),
            title: None,
            description: None,
            input: None,
            shows: Vec::new(),
            error: None,
            presentation: None,
            client: None,
            side_effects: SideEffectClass::Safe,
            coverage_exceptions: BTreeMap::new(),
        };
        let cases = [
            ("node", Language::Node, "typescript", "typescript"),
            ("wasm", Language::Wasm, "typescript", "wasm"),
            ("kotlin_android", Language::KotlinAndroid, "kotlin", "kotlin-android"),
        ];

        for (target_language, binding_language, canonical_name, output_slug) in cases {
            let language = DocumentationLanguage::Binding(binding_language);
            let fixture = documented_fixture();
            let rendered = render_snippet_markdown("example()", &fixture, &docs, target_language, language);
            let path = snippet_path("docs/snippets", &docs, "example", target_language, language)
                .expect("snippet path is valid");

            // ~keep Assert the WHOLE document, not a set of `contains` probes. Substring probes
            // pin only the fields they name, so `level`, `requires` and `side_effect` become
            // unguarded: a renderer emitting a bogus value for any of them still passes. See
            // `frontmatter_fields_are_pinned_by_exact_equality` for the controls.
            assert_eq!(
                rendered,
                format!(
                    "---\nid: fixture_{target_language}_extension_owned\nlanguage: {canonical_name}\ntarget: {target_language}\nlevel: null\nrequires: []\nside_effect: safe\n---\n\n{SNIPPET_HEADER}Extension-owned example\n\n```{canonical_name} title=\"{}\"\nexample()\n```\n",
                    language.display_name()
                )
            );
            assert_eq!(
                path,
                Path::new("docs/snippets").join(output_slug).join("api/example.md")
            );
            assert_ne!(
                crate::snippets::types::Language::from_fence_tag(canonical_name),
                crate::snippets::types::Language::Unknown
            );
        }
    }

    /// Controls for `generated_docs_use_validator_canonical_language_identity`.
    ///
    /// ~keep Each case varies exactly one frontmatter input and pins the whole rendered
    /// document. A renderer that emitted a bogus or constant `requires`, `side_effect` or
    /// `level` would satisfy a `contains`-style probe but fails here, which is the property
    /// the exact-equality assertion exists to hold.
    #[test]
    fn frontmatter_fields_are_pinned_by_exact_equality() {
        let render = |fixture: &Fixture, side_effects: SideEffectClass, target: &str| {
            let docs = FixtureDocs {
                topic: "api".into(),
                stem: None,
                paths: BTreeMap::new(),
                title: None,
                description: None,
                input: None,
                shows: Vec::new(),
                error: None,
                presentation: None,
                client: None,
                side_effects,
                coverage_exceptions: BTreeMap::new(),
            };
            render_snippet_markdown(
                "example()",
                fixture,
                &docs,
                target,
                DocumentationLanguage::Binding(Language::Node),
            )
        };

        let baseline = documented_fixture();

        // `side_effect` tracks the docs class rather than a hardcoded "safe".
        assert_eq!(
            render(&baseline, SideEffectClass::Network, "node"),
            format!(
                "---\nid: fixture_node_extension_owned\nlanguage: typescript\ntarget: node\nlevel: typecheck\nrequires: []\nside_effect: network\n---\n\n{SNIPPET_HEADER}Extension-owned example\n\n```typescript title=\"TypeScript\"\nexample()\n```\n"
            )
        );
        assert_eq!(
            render(&baseline, SideEffectClass::Install, "node"),
            format!(
                "---\nid: fixture_node_extension_owned\nlanguage: typescript\ntarget: node\nlevel: typecheck\nrequires: []\nside_effect: install\n---\n\n{SNIPPET_HEADER}Extension-owned example\n\n```typescript title=\"TypeScript\"\nexample()\n```\n"
            )
        );

        // `requires` tracks the fixture's declared requirements rather than a hardcoded `[]`.
        let required = Fixture {
            requirements: vec!["feature:json".into(), "service:api".into()],
            ..documented_fixture()
        };
        assert_eq!(
            render(&required, SideEffectClass::Safe, "node"),
            format!(
                "---\nid: fixture_node_extension_owned\nlanguage: typescript\ntarget: node\nlevel: null\nrequires: [\"feature:json\",\"service:api\"]\nside_effect: safe\n---\n\n{SNIPPET_HEADER}Extension-owned example\n\n```typescript title=\"TypeScript\"\nexample()\n```\n"
            )
        );
    }

    /// `render_snippet_markdown` stamps `level: null` for `Safe` side effects instead of the
    /// unconditional `typecheck` `94d09809d` introduced, so `SnippetMetadata::level` resolves to
    /// `None` and `effective_validation_level` (`src/snippets/runner.rs`) has nothing of the
    /// front matter's own to fold the requested level down against.
    #[test]
    fn safe_side_effects_snippet_is_not_level_capped() {
        let docs = FixtureDocs {
            topic: "api".into(),
            stem: None,
            paths: BTreeMap::new(),
            title: None,
            description: None,
            input: None,
            shows: Vec::new(),
            error: None,
            presentation: None,
            client: None,
            side_effects: SideEffectClass::Safe,
            coverage_exceptions: BTreeMap::new(),
        };
        let rendered = render_snippet_markdown(
            "example()",
            &documented_fixture(),
            &docs,
            "node",
            DocumentationLanguage::Binding(Language::Node),
        );

        assert!(
            rendered.contains("\nlevel: null\n"),
            "safe snippet must not declare a level cap, got: {rendered}"
        );

        let front_matter = rendered
            .split("---\n")
            .nth(1)
            .expect("rendered snippet has front matter");
        let metadata: crate::snippets::types::SnippetMetadata =
            serde_yaml::from_str(front_matter).expect("front matter is valid YAML");
        assert_eq!(metadata.level, None, "safe snippet must resolve to no declared level");
    }

    /// The `typecheck` cap `94d09809d` introduced for side-effecting fixtures survives: it is
    /// exactly the fixtures this test conditions on that the e2e harness cannot safely execute
    /// unattended, so they must still resolve to a declared `TypeCheck` ceiling.
    #[test]
    fn unsafe_side_effects_snippet_keeps_the_typecheck_cap() {
        for side_effects in [
            SideEffectClass::Network,
            SideEffectClass::Process,
            SideEffectClass::Install,
            SideEffectClass::Server,
        ] {
            let docs = FixtureDocs {
                topic: "api".into(),
                stem: None,
                paths: BTreeMap::new(),
                title: None,
                description: None,
                input: None,
                shows: Vec::new(),
                error: None,
                presentation: None,
                client: None,
                side_effects,
                coverage_exceptions: BTreeMap::new(),
            };
            let rendered = render_snippet_markdown(
                "example()",
                &documented_fixture(),
                &docs,
                "node",
                DocumentationLanguage::Binding(Language::Node),
            );

            assert!(
                rendered.contains("\nlevel: typecheck\n"),
                "unsafe snippet ({side_effects:?}) must keep the typecheck cap, got: {rendered}"
            );

            let front_matter = rendered
                .split("---\n")
                .nth(1)
                .expect("rendered snippet has front matter");
            let metadata: crate::snippets::types::SnippetMetadata =
                serde_yaml::from_str(front_matter).expect("front matter is valid YAML");
            assert_eq!(
                metadata.level,
                Some(crate::snippets::types::ValidationLevel::TypeCheck),
                "unsafe snippet ({side_effects:?}) must resolve to the typecheck cap"
            );
        }
    }

    /// A snippet exactly as `generate_snippet_report` emits it, for the ownership tests below.
    fn rendered_snippet() -> String {
        let docs = FixtureDocs {
            topic: "api".into(),
            stem: None,
            paths: BTreeMap::new(),
            title: None,
            description: None,
            input: None,
            shows: Vec::new(),
            error: None,
            presentation: None,
            client: None,
            side_effects: SideEffectClass::Safe,
            coverage_exceptions: BTreeMap::new(),
        };
        render_snippet_markdown(
            "example()",
            &documented_fixture(),
            &docs,
            "python",
            DocumentationLanguage::Binding(Language::Python),
        )
    }

    /// The same document with the provenance block removed, for use as a negative control.
    fn rendered_snippet_without_header() -> String {
        let rendered = rendered_snippet();
        let stripped = rendered.replace(SNIPPET_HEADER, "");
        assert_ne!(stripped, rendered, "control must actually remove the header");
        stripped
    }

    /// The marker alef emits into a snippet must be one the read side matches.
    ///
    /// `content_has_alef_marker` is the single definition of "alef owns this file's
    /// provenance" — the write guard, `alef verify`'s walk and the stamping pass all call it —
    /// so this is the assertion that the header is not merely present but recognised. ~keep
    #[test]
    fn rendered_snippet_carries_a_marker_the_read_side_recognises() {
        assert!(crate::core::hash::content_has_alef_marker(&rendered_snippet()));
    }

    /// Negative control for the test above: without the header the same document is NOT
    /// recognised, so recognition is attributable to the header and not to anything else the
    /// snippet happens to contain. Without this, the positive test proves nothing. ~keep
    #[test]
    fn read_side_does_not_recognise_a_snippet_without_the_header() {
        assert!(!crate::core::hash::content_has_alef_marker(
            &rendered_snippet_without_header()
        ));
    }

    /// `content_has_alef_marker` only scans a fixed window of leading lines, and the snippet
    /// front matter — which must stay first in the file for Astro/Starlight — consumes almost
    /// all of it. This pins the remaining budget: the marker lands on the last line still
    /// inside the window, and one extra front-matter line pushes it out. A regression here is
    /// silent (the marker stays in the file; nothing reads it), which is why the control that
    /// the window is genuinely the binding constraint is asserted alongside. ~keep
    #[test]
    fn snippet_marker_lands_inside_the_read_side_scan_window() {
        let rendered = rendered_snippet();
        let marker_index = rendered
            .lines()
            .position(|line| line.contains("auto-generated by alef"))
            .expect("rendered snippet carries the marker");
        assert_eq!(marker_index, 9, "marker must stay on the last line of the scan window");

        let widened = rendered.replacen("\nlevel: null\n", "\nlevel: null\nextra: value\n", 1);
        assert!(
            !crate::core::hash::content_has_alef_marker(&widened),
            "one extra front-matter line must push the marker out of the scan window -- \
             the budget this test guards is exactly zero lines"
        );
    }

    /// The header must not disturb the two structures the snippet pipeline parses out of these
    /// files: the YAML front matter (which must remain the first bytes) and the single fenced
    /// block. It must also not be mistaken for a `<!-- snippet:... -->` annotation, which
    /// `parser::extract_fenced_blocks` reads from the line immediately preceding a fence. ~keep
    #[test]
    fn snippet_header_preserves_frontmatter_and_fence_structure() {
        let rendered = rendered_snippet();
        assert!(rendered.starts_with("---\nid: fixture_python_extension_owned\n"));
        assert_eq!(
            crate::snippets::parser::frontmatter_status(&rendered),
            crate::snippets::parser::FrontmatterStatus::Present
        );

        let blocks = crate::snippets::parser::extract_fenced_blocks(&rendered);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].lang, "python");
        assert_eq!(blocks[0].code, "example()");
        assert_eq!(
            blocks[0].preceding_comment, None,
            "the provenance block must not be read as a snippet annotation"
        );
    }

    /// The marker also has to survive `alef verify`'s stamping round trip: `finalize_hashes`
    /// injects the `alef:hash:` line directly after the marker, and a marked file whose hash
    /// cannot be read back is reported stale with `<missing>` on every verify. ~keep
    #[test]
    fn snippet_marker_survives_hash_stamping() {
        let rendered = rendered_snippet();
        let stamped = crate::core::hash::inject_hash_line(&rendered, "abc123");

        assert!(crate::core::hash::content_has_alef_marker(&stamped));
        assert_eq!(crate::core::hash::extract_hash(&stamped).as_deref(), Some("abc123"));
        assert!(stamped.contains("<!-- alef:hash:abc123 -->"));
        assert_eq!(crate::core::hash::strip_hash_line(&stamped), rendered);
    }

    /// The stamping pass selects files by `carries_alef_marker`, not by `generated_header`.
    /// Snippets are emitted `generated_header: false`, so this is what routes them into
    /// `finalize_hashes` and keeps verify from reporting them as unstamped. ~keep
    #[test]
    fn generated_snippet_file_is_claimed_by_the_stamping_pass() {
        let file = crate::core::backend::GeneratedFile {
            path: PathBuf::from("docs/snippets/python/api/example.md"),
            content: rendered_snippet(),
            generated_header: false,
        };
        assert!(file.carries_alef_marker());
    }

    /// End-to-end proof against the real guard, which is the only thing that settles whether
    /// the marker actually unfreezes these files: the guard is asked to overwrite a
    /// pre-existing snippet whose content differs, once with the header on disk and once
    /// without. Asserting on `content_has_alef_marker` alone would not — the guard also
    /// consults the ownership record, and a test that never exercised it could pass for the
    /// wrong reason. Neither temp dir carries a `.alef-ownership.toml`, so the marker is the
    /// only proof available in either case. ~keep
    #[test]
    fn write_guard_accepts_a_marked_snippet_and_refuses_an_unmarked_one() {
        let relative = PathBuf::from("docs/snippets/python/api/example.md");
        let updated = rendered_snippet().replace("example()", "updated_example()");

        let write_over = |existing: &str| {
            let directory = tempfile::tempdir().expect("temporary output directory");
            let full_path = directory.path().join(&relative);
            std::fs::create_dir_all(full_path.parent().expect("snippet parent")).expect("snippet directory");
            std::fs::write(&full_path, existing).expect("pre-existing snippet");
            let report = crate::cli::pipeline::write_scaffold_files_report(
                &[crate::core::backend::GeneratedFile {
                    path: relative.clone(),
                    content: updated.clone(),
                    generated_header: false,
                }],
                directory.path(),
                true,
            )
            .expect("scaffold write report");
            (
                report.changed_paths.contains(&full_path),
                report.refused_paths.contains(&full_path),
                std::fs::read_to_string(&full_path).expect("snippet still readable"),
            )
        };

        let (marked_written, marked_refused, marked_content) = write_over(&rendered_snippet());
        assert!(marked_written, "a marked snippet must be regenerable");
        assert!(!marked_refused);
        assert!(marked_content.contains("updated_example()"));

        let unmarked_existing = rendered_snippet_without_header();
        let (unmarked_written, unmarked_refused, unmarked_content) = write_over(&unmarked_existing);
        assert!(!unmarked_written, "an unmarked snippet has no proof of authorship");
        assert!(unmarked_refused);
        assert_eq!(
            unmarked_content, unmarked_existing,
            "a refused file must be left byte-identical"
        );
    }

    /// The companion to the test above, and the only end-to-end proof that the ledger disjunct in
    /// `write_scaffold_files_report` actually unfreezes anything: the same unmarked, pre-existing
    /// snippet the guard refuses above is written when the previous run's coverage ledger records
    /// it. The negative half lives in the test above -- its temp dirs carry no ledger, so it
    /// already pins that an unrecorded unmarked snippet stays refused. Both halves matter: a
    /// disjunct that claimed every unmarked `.md` would pass this test alone. ~keep
    #[test]
    fn write_guard_accepts_an_unmarked_snippet_the_previous_run_recorded_in_the_ledger() {
        let relative = PathBuf::from("docs/snippets/python/api/example.md");
        let sibling = PathBuf::from("docs/snippets/python/api/hand-written.md");
        let updated = rendered_snippet().replace("example()", "updated_example()");
        let existing = rendered_snippet_without_header();

        let directory = tempfile::tempdir().expect("temporary output directory");
        let root = directory.path().join("docs/snippets");
        for path in [&relative, &sibling] {
            let full = directory.path().join(path);
            std::fs::create_dir_all(full.parent().expect("snippet parent")).expect("snippet directory");
            std::fs::write(&full, &existing).expect("pre-existing snippet");
        }

        let key = SnippetCoverageKey {
            fixture_id: "example".into(),
            language: "python".into(),
        };
        let ledger = SnippetCoverageLedger {
            format_version: COVERAGE_MANIFEST_VERSION,
            generated_paths: vec![PathBuf::from("python/api/example.md")],
            generated_metadata: vec![GeneratedSnippetMetadata {
                key: key.clone(),
                path: PathBuf::from("python/api/example.md"),
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
        };
        std::fs::write(
            root.join(COVERAGE_MANIFEST),
            serde_json::to_string(&ledger).expect("serialize ledger"),
        )
        .expect("write ledger");
        super::ownership::snapshot_pre_run_ledger(&root);

        let report = crate::cli::pipeline::write_scaffold_files_report(
            &[
                crate::core::backend::GeneratedFile {
                    path: relative.clone(),
                    content: updated.clone(),
                    generated_header: false,
                },
                crate::core::backend::GeneratedFile {
                    path: sibling.clone(),
                    content: updated.clone(),
                    generated_header: false,
                },
            ],
            directory.path(),
            true,
        )
        .expect("scaffold write report");

        let recorded = directory.path().join(&relative);
        let unrecorded = directory.path().join(&sibling);
        assert!(
            report.changed_paths.contains(&recorded),
            "a snippet the previous run recorded must be regenerable"
        );
        assert!(
            report.refused_paths.contains(&unrecorded),
            "a hand-written sibling under the same root has no record and must stay refused"
        );
        assert_eq!(
            std::fs::read_to_string(&unrecorded).expect("sibling still readable"),
            existing,
            "a refused file must be left byte-identical"
        );
    }

    #[test]
    fn snippet_generator_resolution_rejects_unknown_languages() {
        let error = snippet_generators(&["unknown".into()])
            .err()
            .expect("unknown language must fail");
        assert_eq!(
            error.to_string(),
            "no e2e code generator registered for snippet language `unknown`"
        );
    }

    #[test]
    fn snippet_generator_resolution_rejects_alias_duplicates() {
        let error = snippet_generators(&["rust".into(), "rust_core".into()])
            .err()
            .expect("duplicate generator selection must fail");
        assert_eq!(
            error.to_string(),
            "duplicate snippet language resolves to e2e code generator `rust`"
        );
    }

    #[test]
    fn markdown_wrapper_uses_backend_body_and_metadata() {
        let docs = FixtureDocs {
            topic: "api".into(),
            stem: None,
            paths: BTreeMap::new(),
            title: Some("Example".into()),
            description: None,
            input: None,
            shows: Vec::new(),
            error: None,
            presentation: None,
            client: None,
            side_effects: SideEffectClass::Network,
            coverage_exceptions: BTreeMap::new(),
        };

        let rendered = render_snippet_markdown(
            "backend_call()",
            &documented_fixture(),
            &docs,
            "python",
            DocumentationLanguage::Binding(Language::Python),
        );

        assert!(rendered.starts_with("---\nid: fixture_python_extension_owned\nlanguage: python\ntarget: python\n"));
        assert!(rendered.contains("requires: []\nside_effect: network\n---"));
        assert!(rendered.ends_with("```python title=\"Python\"\nbackend_call()\n```\n"));
        assert!(!rendered.contains("Backend-owned body"));
        assert!(!rendered.contains("Example"));
    }

    #[test]
    fn extension_owned_recipe_satisfies_expected_coverage() {
        let fixture = documented_fixture();
        let mut e2e = E2eConfig::default();
        e2e.call.function = "built_in_would_fail".into();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
            body: "extension_call()",
        })];
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(
            &[fixture],
            &["rust".into()],
            &snippet_config,
            &context,
            &extensions,
        )
        .expect("extension snippet report renders");

        assert_eq!(report.coverage.expected.len(), 1);
        assert_eq!(report.coverage.generated, report.coverage.expected);
        assert!(report.coverage.missing.is_empty());
        assert!(report.snippets[0].file.content.contains("extension_call()"));
    }

    #[test]
    fn c_trait_bridge_vtable_recipe_counts_as_generated() {
        let mut fixture = documented_fixture();
        fixture.call = Some("register_sample_backend".into());
        fixture.args = vec![crate::core::config::e2e::ArgMapping {
            name: "backend".into(),
            field: "backend".into(),
            arg_type: "test_backend".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some("SampleBackend".into()),
        }];
        let mut e2e = E2eConfig::default();
        let mut call = crate::core::config::e2e::CallConfig::default();
        call.overrides.insert(
            "python".into(),
            crate::core::config::e2e::CallOverride {
                function: Some("register_sample_backend".into()),
                ..Default::default()
            },
        );
        e2e.calls.insert("register_sample_backend".into(), call);
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let crate_config = ResolvedCrateConfig {
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "SampleBackend".into(),
                register_fn: Some("register_sample_backend".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let type_defs = [TypeDef {
            name: "SampleBackend".into(),
            is_trait: true,
            ..TypeDef::default()
        }];
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &type_defs,
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(
            &[fixture],
            &["c".into(), "python".into()],
            &snippet_config,
            &context,
            &[],
        )
        .expect("unsupported C recipe belongs in the coverage ledger");

        assert_eq!(report.coverage.expected.len(), 2);
        assert_eq!(report.coverage.generated.len(), 2);
        assert!(report.coverage.missing.is_empty());
        let c = report
            .snippets
            .iter()
            .find(|snippet| snippet.language == "c")
            .expect("C trait bridge snippet");
        assert!(c.file.content.contains("register_sample_backend"));
        assert!(c.file.content.contains(".free_user_data = sample_free_context"));
    }

    #[test]
    fn unclaimed_domain_fixture_is_recorded_as_missing() {
        let mut fixture = documented_fixture();
        fixture.asyncapi = Some(crate::e2e::fixture::AsyncApiFixture {
            spec: serde_json::json!({"asyncapi": "3.0.0"}),
            expected: serde_json::Value::Null,
            validation: None,
        });
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let e2e = E2eConfig::default();
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report =
            generate_snippet_report_with_extensions(&[fixture], &["go".into()], &snippet_config, &context, &[])
                .expect("unclaimed domain recipe belongs in coverage report");

        assert!(report.snippets.is_empty());
        assert_eq!(report.coverage.missing.len(), 1);
        assert_eq!(
            report.coverage.missing[0].reason,
            "AsyncAPI fixture requires an extension-owned documentation recipe"
        );
    }

    #[test]
    fn empty_call_identity_is_missing_instead_of_generated() {
        let fixture = documented_fixture();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let e2e = E2eConfig::default();
        assert!(e2e.call.function.is_empty());
        assert!(e2e.call.module.is_empty());
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(
            &[fixture],
            &["go".into(), "java".into()],
            &snippet_config,
            &context,
            &[],
        )
        .expect("missing call identities belong in the coverage ledger");

        assert!(report.snippets.is_empty());
        assert!(report.coverage.generated.is_empty());
        assert_eq!(report.coverage.expected.len(), 2);
        assert_eq!(report.coverage.missing.len(), 2);
        assert!(
            report
                .coverage
                .missing
                .iter()
                .all(|missing| missing.reason.contains("has no function identity"))
        );
    }

    #[test]
    fn language_function_override_supplies_missing_default_identity() {
        let fixture = documented_fixture();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.overrides.insert(
            "go".into(),
            crate::core::config::e2e::CallOverride {
                function: Some("process".into()),
                ..Default::default()
            },
        );
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report =
            generate_snippet_report_with_extensions(&[fixture], &["go".into()], &snippet_config, &context, &[])
                .expect("language override supplies a valid identity");

        assert_eq!(report.coverage.generated, report.coverage.expected);
        assert!(report.coverage.missing.is_empty());
        assert!(!report.snippets[0].file.content.contains("pkg.()"));
    }

    /// The peer's positive control: a fixture's function excluded via `[crates.wasm]
    /// exclude_functions` must drop out of `expected` for wasm specifically, while the very
    /// same fixture -- same call, same function identity -- stays expected (and generated)
    /// for a language that does not exclude it. A version of this check that ignored the
    /// exclusion entirely would still pass every other assertion in this file (the fixture
    /// renders fine on both targets absent the exclusion) but would fail the two assertions
    /// below, which is what makes this the load-bearing test rather than a truthiness check.
    #[test]
    fn excluded_function_drops_only_the_excluding_languages_cell_from_expected() {
        let fixture = documented_fixture();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "excluded_fn".into();
        let wasm_config = toml::from_str::<crate::core::config::WasmConfig>("exclude_functions = [\"excluded_fn\"]")
            .expect("wasm config with exclude_functions parses");
        let crate_config = ResolvedCrateConfig {
            wasm: Some(wasm_config),
            ..ResolvedCrateConfig::default()
        };
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(
            &[fixture],
            &["wasm".into(), "go".into()],
            &snippet_config,
            &context,
            &[],
        )
        .expect("an excluded function must not abort the run");

        let wasm_key = SnippetCoverageKey {
            fixture_id: "extension_owned".into(),
            language: "wasm".into(),
        };
        let go_key = SnippetCoverageKey {
            fixture_id: "extension_owned".into(),
            language: "go".into(),
        };
        assert_eq!(
            report.coverage.expected,
            vec![go_key.clone()],
            "wasm's exclude_functions entry must remove the wasm cell from `expected` while \
             leaving go's untouched: {:?}",
            report.coverage.expected
        );
        assert!(
            !report.coverage.expected.contains(&wasm_key),
            "excluded cell must not be expected for wasm: {:?}",
            report.coverage.expected
        );
        assert_eq!(report.coverage.generated, vec![go_key]);
        assert!(
            report.coverage.missing.is_empty(),
            "an excluded cell is not a coverage gap -- it must never have been expected in the \
             first place, so it must not appear in `missing` either: {:?}",
            report.coverage.missing
        );
        assert_eq!(report.coverage.generated_paths.len(), 1);
        assert!(!report.coverage.generated_paths[0].starts_with("wasm"));
    }

    #[test]
    fn documentation_rendering_is_independent_of_test_harness_skips() {
        let mut fixture = documented_fixture();
        fixture.skip = Some(crate::e2e::fixture::SkipDirective {
            languages: vec!["ruby".into()],
            reason: Some("The test harness cannot exercise this protocol operation".into()),
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "built_in_would_fail".into();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
            body: "extension_call()",
        })];
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(
            &[fixture],
            &["ruby".into()],
            &snippet_config,
            &context,
            &extensions,
        )
        .expect("test harness skip does not suppress the extension-owned recipe");

        assert_eq!(report.coverage.generated, report.coverage.expected);
        assert!(report.coverage.missing.is_empty());
    }

    /// Regression test for the `c` plugin-api doc snippets that called a symbol
    /// that does not exist (`{prefix}_clear_ocr_backends`, the pluralised `clear_fn`
    /// config text, instead of the real singular `{prefix}_clear_ocr_backend` the FFI
    /// backend derives from the trait name). Those fixtures are `skip.languages = ["c"]`
    /// because the C API cannot expose a host-language callback, have no
    /// extension-owned recipe, and no per-language call override — so the naive
    /// `trait_bridge_function_identity` fallback must not run for them; the
    /// pair should land in `coverage.missing`, not produce a broken snippet.
    #[test]
    fn skipped_fixture_without_extension_recipe_omits_c_snippet_and_records_missing() {
        let mut fixture = documented_fixture();
        fixture.call = Some("clear_ocr_backends".into());
        fixture.skip = Some(crate::e2e::fixture::SkipDirective {
            languages: vec!["c".into()],
            reason: Some("The C API does not expose the clear call that pairs with registration".into()),
        });
        let e2e = E2eConfig::default();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let crate_config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "OcrBackend".into(),
                clear_fn: Some("clear_ocr_backends".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(&[fixture], &["c".into()], &snippet_config, &context, &[])
            .expect("a skipped fixture with no recipe belongs in the coverage ledger, not an error");

        assert!(report.snippets.is_empty());
        assert!(report.coverage.generated.is_empty());
        assert_eq!(report.coverage.missing.len(), 1);
        assert_eq!(
            report.coverage.missing[0].reason,
            "built-in `c` snippet recipe has no function identity; configure a call function or provide an extension-owned documentation recipe"
        );
    }

    /// Companion to the regression test above: a fixture skipped for `c` but
    /// backed by an extension-owned recipe must still render — doc rendering
    /// stays independent of test-harness skips whenever a real recipe exists.
    /// The extension loop in `render_snippet_body` runs before the skip check
    /// this fix introduces, so this must keep passing unchanged.
    #[test]
    fn skipped_c_fixture_with_extension_owned_recipe_still_renders() {
        let mut fixture = documented_fixture();
        fixture.call = Some("clear_ocr_backends".into());
        fixture.skip = Some(crate::e2e::fixture::SkipDirective {
            languages: vec!["c".into()],
            reason: Some("The C API does not expose the clear call that pairs with registration".into()),
        });
        let e2e = E2eConfig::default();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let crate_config = ResolvedCrateConfig::default();
        let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
            body: "extension_call()",
        })];
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report =
            generate_snippet_report_with_extensions(&[fixture], &["c".into()], &snippet_config, &context, &extensions)
                .expect("an extension-owned recipe renders even when the harness skips this language");

        assert_eq!(report.coverage.generated.len(), 1);
        assert!(report.coverage.missing.is_empty());
        assert_eq!(report.snippets.len(), 1);
        assert!(report.snippets[0].file.content.contains("extension_call()"));
    }

    /// A fixture that is not skipped for `c` keeps using the naive
    /// `trait_bridge_function_identity` fallback exactly as before — this fix
    /// only gates the fallback on `SkipDirective::should_skip`, so an unskipped
    /// fixture must render identically to the pre-fix behaviour.
    #[test]
    fn not_skipped_c_fixture_renders_naive_trait_bridge_identity_as_before() {
        let mut fixture = documented_fixture();
        fixture.call = Some("clear_ocr_backends".into());
        let e2e = E2eConfig::default();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let crate_config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "OcrBackend".into(),
                clear_fn: Some("clear_ocr_backends".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(&[fixture], &["c".into()], &snippet_config, &context, &[])
            .expect("an unskipped fixture with a resolvable trait-bridge identity still generates a C snippet");

        assert_eq!(report.coverage.generated.len(), 1);
        assert!(report.coverage.missing.is_empty());
        assert_eq!(report.snippets.len(), 1);
        // A fixture that is NOT skipped still renders, and now renders the symbol the FFI
        // backend actually exports: `{prefix}_clear_{trait_snake}` derived from the trait name
        // (`registration.rs:141`), SINGULAR — not the pluralised `clear_fn` config text, which
        // only ever matched the fixture to a bridge. The trailing `NULL` is the C out-error
        // argument. Before the derivation fix this emitted `sample_clear_ocr_backends(NULL)`,
        // naming a symbol the header does not declare.
        assert!(
            report.snippets[0]
                .file
                .content
                .contains("sample_clear_ocr_backend(NULL);"),
            "expected the derived singular ABI symbol, got:\n{}",
            report.snippets[0].file.content
        );
    }

    /// The `NULL` a trait-bridge `clear`/`unregister` snippet emits must be the `out_error`
    /// out-param appended from `extra_args` (`c.rs`, `clear_fn.jinja`), NOT a by-product of
    /// rendering an absent fixture `input`. Those two sources were indistinguishable for as long
    /// as the only fixture covering this path used `Fixture::default()`, whose `input` is
    /// `Value::Null` -- and `json_to_c(Value::Null)` renders the literal `NULL`, landing in
    /// exactly the out_error slot by coincidence. This fixture carries a NON-null `input`, so the
    /// argument list can only read `(NULL)` if out_error is genuinely being appended: the
    /// coincidence would emit the serialized input instead. ~keep
    #[test]
    fn trait_bridge_out_error_arg_comes_from_extra_args_not_from_a_null_fixture_input() {
        let mut fixture = documented_fixture();
        fixture.call = Some("clear_ocr_backends".into());
        fixture.input = serde_json::json!({"unused": "payload"});
        let e2e = E2eConfig::default();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let crate_config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "OcrBackend".into(),
                clear_fn: Some("clear_ocr_backends".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(&[fixture], &["c".into()], &snippet_config, &context, &[])
            .expect("a trait-bridge fixture with a non-null input still generates a C snippet");

        let content = &report.snippets[0].file.content;
        assert!(
            content.contains("sample_clear_ocr_backend(NULL);"),
            "out_error must be appended from extra_args regardless of the fixture input, got:\n{content}"
        );
        assert!(
            !content.contains("unused"),
            "the fixture input must not be spliced into the argument list, got:\n{content}"
        );
    }

    #[test]
    fn shared_validation_identities_keep_distinct_target_output_paths() {
        let fixture = documented_fixture();
        let mut e2e = E2eConfig::default();
        e2e.call.function = "call".into();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
            body: "extension_call()",
        })];
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(
            &[fixture],
            &["node".into(), "wasm".into(), "kotlin".into(), "kotlin_android".into()],
            &snippet_config,
            &context,
            &extensions,
        )
        .expect("shared validation languages use distinct target output routes");

        let paths: BTreeSet<_> = report
            .snippets
            .iter()
            .map(|snippet| snippet.file.path.as_path())
            .collect();
        assert!(
            paths.contains(Path::new("docs/snippets/typescript/api/extension_owned.md")),
            "paths: {paths:?}"
        );
        assert!(
            paths.contains(Path::new("docs/snippets/wasm/api/extension_owned.md")),
            "paths: {paths:?}"
        );
        assert!(
            paths.contains(Path::new("docs/snippets/kotlin/api/extension_owned.md")),
            "paths: {paths:?}"
        );
        assert!(
            paths.contains(Path::new("docs/snippets/kotlin-android/api/extension_owned.md")),
            "paths: {paths:?}"
        );
        assert_eq!(report.coverage.generated, report.coverage.expected);
        for snippet in &report.snippets {
            let canonical = match snippet.language.as_str() {
                "node" | "wasm" => "typescript",
                "kotlin" | "kotlin_android" => "kotlin",
                other => panic!("unexpected target: {other}"),
            };
            assert!(snippet.file.content.contains(&format!("```{canonical} title=")));
            let metadata = report
                .coverage
                .generated_metadata
                .iter()
                .find(|metadata| metadata.key.language == snippet.language)
                .expect("target metadata");
            assert_eq!(metadata.language, canonical);
            assert_eq!(metadata.target, snippet.language);
            assert_eq!(metadata.session, snippet.language);
        }
    }

    #[test]
    fn empty_extension_recipe_is_recorded_as_missing() {
        let fixture = documented_fixture();
        let mut e2e = E2eConfig::default();
        e2e.call.function = "call".into();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension { body: "  " })];
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report = generate_snippet_report_with_extensions(
            &[fixture],
            &["rust".into()],
            &snippet_config,
            &context,
            &extensions,
        )
        .expect("empty recipe belongs in coverage report");

        assert!(report.snippets.is_empty());
        assert_eq!(report.coverage.missing.len(), 1);
        assert!(report.coverage.missing[0].reason.contains("empty snippet body"));
    }

    #[test]
    fn unsupported_brew_recipe_uses_exact_coverage_exception() {
        let mut fixture = documented_fixture();
        let docs = fixture.docs.as_mut().expect("fixture has documentation metadata");
        docs.coverage_exceptions.insert(
            "brew".into(),
            crate::e2e::fixture::SnippetCoverageException {
                reason: "The package installation flow is documented separately".into(),
                documentation: "docs/install/homebrew.md".into(),
            },
        );
        let e2e = E2eConfig::default();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report =
            generate_snippet_report_with_extensions(&[fixture], &["brew".into()], &snippet_config, &context, &[])
                .expect("unsupported brew recipe belongs in coverage report");

        assert!(report.snippets.is_empty());
        assert!(report.coverage.missing.is_empty());
        assert_eq!(report.coverage.expected.len(), 1);
        assert_eq!(report.coverage.documented_exceptions.len(), 1);
        assert_eq!(report.coverage.documented_exceptions[0].key.language, "brew");
    }

    #[test]
    fn unsupported_shell_targets_are_recorded_without_mapping_failures() {
        let e2e = E2eConfig::default();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        for language in ["brew", "homebrew"] {
            let report = generate_snippet_report_with_extensions(
                &[documented_fixture()],
                &[language.into()],
                &snippet_config,
                &context,
                &[],
            )
            .expect("unsupported shell target belongs in coverage report");

            assert_eq!(report.coverage.expected.len(), 1);
            assert_eq!(report.coverage.missing.len(), 1);
            assert_eq!(report.coverage.missing[0].key.language, language);
            assert!(!report.coverage.missing[0].reason.is_empty());
            assert!(!report.coverage.missing[0].reason.contains("language mapping"));
        }
    }

    #[test]
    fn fixture_without_docs_is_expected_and_recorded_as_missing() {
        let fixture = Fixture {
            id: "undocumented".into(),
            ..Fixture::default()
        };
        let e2e = E2eConfig::default();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };

        let report =
            generate_snippet_report_with_extensions(&[fixture], &["rust".into()], &snippet_config, &context, &[])
                .expect("undocumented fixture belongs in coverage report");

        assert_eq!(report.coverage.expected.len(), 1);
        assert_eq!(report.coverage.missing.len(), 1);
        assert_eq!(
            report.coverage.missing[0].reason,
            "fixture has no documentation metadata"
        );
    }

    #[test]
    fn rust_visitor_snippets_declare_the_required_feature() {
        let mut fixture = documented_fixture();
        fixture.visitor = Some(crate::e2e::fixture::VisitorSpec {
            callbacks: BTreeMap::new(),
        });

        assert_eq!(snippet_requirements(&fixture, "rust", ""), ["feature:visitor"]);
        assert!(snippet_requirements(&fixture, "java", "").is_empty());
    }

    fn json_argument_fixture() -> Fixture {
        let argument = |name: &str, arg_type: &str| crate::core::config::e2e::ArgMapping {
            name: name.into(),
            field: name.into(),
            arg_type: arg_type.into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        };
        Fixture {
            id: "json_options".into(),
            input: serde_json::json!({"text": "sample", "options": {"width": 80}}),
            args: vec![argument("text", "string"), argument("options", "json_object")],
            ..documented_fixture()
        }
    }

    fn rust_snippet_report(fixture: Fixture) -> SnippetGenerationReport {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        let snippet_config = SnippetConfig {
            output: "docs/snippets".into(),
            ..SnippetConfig::default()
        };
        let crate_config = ResolvedCrateConfig::default();
        let context = SnippetRenderContext {
            e2e: &e2e,
            crate_config: &crate_config,
            type_defs: &[],
            enums: &[],
            functions: &[],
        };
        generate_snippet_report_with_extensions(&[fixture], &["rust".into()], &snippet_config, &context, &[])
            .expect("rust snippet report renders")
    }

    #[test]
    fn rust_snippets_declare_the_serde_json_crate_their_body_names() {
        let report = rust_snippet_report(json_argument_fixture());

        let snippet = &report.snippets[0];
        assert!(
            snippet.file.content.contains("serde_json::from_str"),
            "body must exercise serde_json: {}",
            snippet.file.content
        );
        assert_eq!(snippet.requirements, ["crate:serde_json"]);
        assert!(
            snippet.file.content.contains("requires: [\"crate:serde_json\"]"),
            "frontmatter must declare the dependency: {}",
            snippet.file.content
        );
        assert_eq!(report.coverage.generated_metadata[0].requires, ["crate:serde_json"]);
    }

    #[test]
    fn rust_snippets_without_json_arguments_declare_no_crate_requirement() {
        let report = rust_snippet_report(documented_fixture());

        let snippet = &report.snippets[0];
        assert!(!snippet.file.content.contains("serde_json"), "{}", snippet.file.content);
        assert!(snippet.requirements.is_empty());
    }

    /// An async fixture renders through `rust/snippet_body.rs.jinja`, which emits `#[tokio::main]`.
    /// The snippet must carry the matching crate requirement, or the validator builds a check
    /// project with no `tokio` in `[dependencies]` and the snippet fails on E0433 rather than on
    /// anything it actually demonstrates.
    #[test]
    fn an_async_rust_snippet_requires_the_tokio_crate() {
        let body = "#[tokio::main]\nasync fn main() {\n    let value = 1u8;\n    println!(\"{value:?}\");\n}\n";

        let requirements = snippet_requirements(&documented_fixture(), "rust", body);

        assert_eq!(requirements, ["crate:tokio"], "async snippet must declare tokio");
    }

    #[test]
    fn a_synchronous_rust_snippet_requires_no_tokio_crate() {
        let body = "fn main() {\n    let value = 1u8;\n    println!(\"{value:?}\");\n}\n";

        let requirements = snippet_requirements(&documented_fixture(), "rust", body);

        assert!(
            requirements.is_empty(),
            "a snippet with no tokio attribute must not pull tokio in: {requirements:?}"
        );
    }
}
