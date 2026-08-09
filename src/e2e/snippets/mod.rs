use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::{E2eConfig, SnippetConfig};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::codegen::{E2eCodegen, all_generators, fixture_inclusion};
use crate::e2e::fixture::{Fixture, FixtureDocs, SideEffectClass};
use anyhow::{Result, bail};
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
        generate_snippet_artifacts(fixtures, languages, e2e, snippets, crate_config, type_defs, enums)?
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
    validate_relative_path(Path::new(&snippets.output), "snippet output")?;
    let generators = snippet_generators(languages)?;
    let mut generated = BTreeMap::<PathBuf, GeneratedSnippet>::new();
    for fixture in fixtures.iter().filter(|fixture| fixture.docs.is_some()) {
        validate_requirements(fixture)?;
        for (language, generator) in &generators {
            if !fixture_inclusion(fixture, language, e2e).is_included() {
                continue;
            }
            let capabilities = capabilities(language, snippets, crate_config);
            if !matches!(snippet_inclusion(fixture, &capabilities), SnippetInclusion::Include) {
                continue;
            }
            let lang = parse_language(generator.language_name()).ok_or_else(|| {
                anyhow::anyhow!(
                    "e2e code generator `{}` has no documentation language mapping",
                    generator.language_name()
                )
            })?;
            let docs = fixture.docs.as_ref().expect("filtered docs fixtures have metadata");
            let path = snippet_path(&snippets.output, docs, &fixture.id, lang)?;
            let body = generator.render_snippet_body(fixture, e2e, crate_config, type_defs, enums)?;
            let content = render_snippet_markdown(&body, fixture, docs, lang, generator.language_name());
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
        }
    }
    Ok(generated.into_values().collect())
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
    language: Language,
    language_name: &str,
) -> String {
    crate::e2e::template_env::render(
        "snippets/file.md.jinja",
        minijinja::context! {
            description => docs.description.as_deref().unwrap_or(&fixture.description),
            fence => crate::docs::naming::lang_code_fence(language),
            title => docs.title.as_deref().unwrap_or(crate::docs::naming::lang_display_name(language)), body => body,
            fixture_id => fixture.id, language => language_name, requirements => fixture.requirements,
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

fn snippet_path(output: &str, docs: &FixtureDocs, fixture_id: &str, language: Language) -> Result<PathBuf> {
    validate_component(&docs.topic, "snippet topic")?;
    let stem = docs.stem.as_deref().unwrap_or(fixture_id);
    validate_component(stem, "snippet stem")?;
    Ok(Path::new(output)
        .join(crate::docs::naming::lang_slug(language))
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
        SideEffectClass::None | SideEffectClass::Local => "safe",
        SideEffectClass::Network | SideEffectClass::ExternalMutation => "network",
    }
}

fn parse_language(value: &str) -> Option<Language> {
    Some(match value {
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
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        assert!(snippet_path("docs/snippets", &docs, "basic", Language::Python).is_err());
    }

    #[test]
    fn language_aliases_include_core_and_ffi_targets() {
        assert_eq!(parse_language("rust_core"), Some(Language::Rust));
        assert_eq!(parse_language("ffi"), Some(Language::C));
        assert_eq!(generator_name("rust_core"), "rust");
        assert_eq!(generator_name("ffi"), "c");
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
        };

        let rendered = render_snippet_markdown("backend_call()", &fixture, &docs, Language::Python, "python");

        assert!(rendered.contains("language: python"));
        assert!(rendered.contains("side_effect: network"));
        assert!(rendered.contains("```python title=\"Example\"\nbackend_call()\n```"));
    }
}
