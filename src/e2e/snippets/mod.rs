use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::{E2eConfig, SnippetConfig};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::codegen::{E2eCodegen, all_generators, fixture_inclusion};
use crate::e2e::fixture::{Fixture, FixtureDocs, SideEffectClass};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub mod migration;

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SnippetCoverageKey {
    pub fixture_id: String,
    pub language: String,
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
    pub expected: Vec<SnippetCoverageKey>,
    pub generated: Vec<SnippetCoverageKey>,
    pub missing: Vec<MissingSnippet>,
    pub documented_exceptions: Vec<DocumentedSnippetException>,
}

#[derive(Debug, Clone, Default)]
pub struct SnippetGenerationReport {
    pub snippets: Vec<GeneratedSnippet>,
    pub coverage: SnippetCoverageLedger,
}

struct SnippetRenderContext<'a> {
    e2e: &'a E2eConfig,
    crate_config: &'a ResolvedCrateConfig,
    type_defs: &'a [TypeDef],
    enums: &'a [EnumDef],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentationLanguage {
    Binding(Language),
    Shell,
}

impl DocumentationLanguage {
    fn slug(self) -> &'static str {
        self.canonical_name()
    }

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

pub fn generate_snippets(
    fixtures: &[Fixture],
    languages: &[String],
    e2e: &E2eConfig,
    snippets: &SnippetConfig,
    crate_config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<Vec<GeneratedFile>> {
    Ok(
        generate_snippet_report(fixtures, languages, e2e, snippets, crate_config, type_defs, enums)?
            .snippets
            .into_iter()
            .map(|snippet| snippet.file)
            .collect(),
    )
}

pub fn generate_snippet_artifacts(
    fixtures: &[Fixture],
    languages: &[String],
    e2e: &E2eConfig,
    snippets: &SnippetConfig,
    crate_config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<Vec<GeneratedSnippet>> {
    Ok(generate_snippet_report(fixtures, languages, e2e, snippets, crate_config, type_defs, enums)?.snippets)
}

pub fn generate_snippet_report(
    fixtures: &[Fixture],
    languages: &[String],
    e2e: &E2eConfig,
    snippets: &SnippetConfig,
    crate_config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<SnippetGenerationReport> {
    crate::with_extensions(|extensions| {
        let context = SnippetRenderContext {
            e2e,
            crate_config,
            type_defs,
            enums,
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
    let generators = snippet_generators(languages)?;
    let mut generated = BTreeMap::<PathBuf, GeneratedSnippet>::new();
    let mut coverage = SnippetCoverageLedger::default();
    for fixture in fixtures {
        validate_requirements(fixture)?;
        validate_coverage_exceptions(fixture)?;
        for (language, generator) in &generators {
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
            let fixture_decision = fixture_inclusion(fixture, language, context.e2e);
            let capabilities = capabilities(language, snippets, context.crate_config);
            let capability_decision = snippet_inclusion(fixture, &capabilities);
            let exclusion_reason = match (&fixture_decision, &capability_decision) {
                (crate::e2e::codegen::InclusionDecision::Exclude(reason), _) => Some((*reason).to_string()),
                (_, SnippetInclusion::Exclude { missing_requirements }) => {
                    Some(format!("missing requirements: {}", missing_requirements.join(", ")))
                }
                _ => None,
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
            let path = snippet_path(&snippets.output, docs, &fixture.id, lang)?;
            let body = match render_snippet_body(extensions, generator.as_ref(), fixture, language, context) {
                Ok(body) => body,
                Err(error) => {
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
            let content = render_snippet_markdown(&body, fixture, docs, lang);
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
                requirements: fixture.requirements.clone(),
                side_effects: docs.side_effects,
            };
            if generated.insert(path.clone(), snippet).is_some() {
                bail!("snippet output collision at {}", path.display());
            }
            coverage.generated.push(key);
        }
    }
    coverage.expected.sort();
    coverage.generated.sort();
    coverage.missing.sort_by(|left, right| left.key.cmp(&right.key));
    coverage
        .documented_exceptions
        .sort_by(|left, right| left.key.cmp(&right.key));
    Ok(SnippetGenerationReport {
        snippets: generated.into_values().collect(),
        coverage,
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
            return Ok(body);
        }
    }
    let body = generator
        .render_snippet_body(
            fixture,
            context.e2e,
            context.crate_config,
            context.type_defs,
            context.enums,
        )
        .map_err(|error| anyhow::anyhow!("built-in `{language}` snippet recipe is incompatible: {error:#}"))?;
    if body.trim().is_empty() {
        bail!("built-in `{language}` snippet recipe returned an empty body");
    }
    Ok(body)
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

fn render_snippet_markdown(
    body: &str,
    fixture: &Fixture,
    docs: &FixtureDocs,
    language: DocumentationLanguage,
) -> String {
    crate::e2e::template_env::render(
        "snippets/file.md.jinja",
        minijinja::context! {
            description => docs.description.as_deref().unwrap_or(&fixture.description),
            fence => language.code_fence(),
            title => docs.title.as_deref().unwrap_or(language.display_name()), body => body,
            fixture_id => fixture.id, language => language.canonical_name(), requirements => fixture.requirements,
            side_effect => side_effect_name(docs.side_effects),
        },
    )
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

fn snippet_path(
    output: &str,
    docs: &FixtureDocs,
    fixture_id: &str,
    language: DocumentationLanguage,
) -> Result<PathBuf> {
    validate_component(&docs.topic, "snippet topic")?;
    let stem = docs.stem.as_deref().unwrap_or(fixture_id);
    validate_component(stem, "snippet stem")?;
    Ok(Path::new(output)
        .join(language.slug())
        .join(&docs.topic)
        .join(format!("{stem}.md")))
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
        || path.components().any(|part| matches!(part, Component::ParentDir))
    {
        bail!("{label} must be a safe relative path: {}", path.display());
    }
    Ok(())
}

fn side_effect_name(value: SideEffectClass) -> &'static str {
    match value {
        SideEffectClass::Safe => "safe",
        SideEffectClass::Network => "network",
        SideEffectClass::Process => "process",
        SideEffectClass::Install => "install",
        SideEffectClass::Server => "server",
    }
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
                title: None,
                description: None,
                side_effects: SideEffectClass::Safe,
                coverage_exceptions: BTreeMap::new(),
            }),
            ..Fixture::default()
        }
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
        let docs = FixtureDocs {
            topic: "..".into(),
            stem: None,
            title: None,
            description: None,
            side_effects: Default::default(),
            coverage_exceptions: BTreeMap::new(),
        };
        assert!(
            snippet_path(
                "docs/snippets",
                &docs,
                "basic",
                DocumentationLanguage::Binding(Language::Python)
            )
            .is_err()
        );
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
            title: None,
            description: None,
            side_effects: SideEffectClass::Safe,
            coverage_exceptions: BTreeMap::new(),
        };
        let fixture = documented_fixture();
        let cases = [
            (Language::Node, "typescript"),
            (Language::Wasm, "typescript"),
            (Language::KotlinAndroid, "kotlin"),
        ];

        for (binding_language, canonical_name) in cases {
            let language = DocumentationLanguage::Binding(binding_language);
            let rendered = render_snippet_markdown("example()", &fixture, &docs, language);
            let path = snippet_path("docs/snippets", &docs, "example", language).expect("snippet path is valid");

            assert!(rendered.contains(&format!("language: {canonical_name}\n")));
            assert!(rendered.contains(&format!("```{canonical_name} ")));
            assert_eq!(
                path,
                Path::new("docs/snippets").join(canonical_name).join("api/example.md")
            );
            assert_ne!(
                crate::snippets::types::Language::from_fence_tag(canonical_name),
                crate::snippets::types::Language::Unknown
            );
        }
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
        let fixture = Fixture {
            id: "backend_owned".into(),
            description: "Backend-owned body".into(),
            requirements: vec!["feature:docs".into()],
            ..Fixture::default()
        };
        let docs = FixtureDocs {
            topic: "api".into(),
            stem: None,
            title: Some("Example".into()),
            description: None,
            side_effects: SideEffectClass::Network,
            coverage_exceptions: BTreeMap::new(),
        };

        let rendered = render_snippet_markdown(
            "backend_call()",
            &fixture,
            &docs,
            DocumentationLanguage::Binding(Language::Python),
        );

        assert!(rendered.contains("language: python"));
        assert!(rendered.contains("side_effect: network"));
        assert!(rendered.contains("```python title=\"Example\"\nbackend_call()\n```"));
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
    fn side_effect_names_preserve_every_class() {
        let cases = [
            (SideEffectClass::Safe, "safe"),
            (SideEffectClass::Network, "network"),
            (SideEffectClass::Process, "process"),
            (SideEffectClass::Install, "install"),
            (SideEffectClass::Server, "server"),
        ];
        for (class, expected) in cases {
            assert_eq!(side_effect_name(class), expected);
        }
    }
}
