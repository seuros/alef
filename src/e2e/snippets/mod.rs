use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::{E2eConfig, SnippetConfig};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::e2e::codegen::fixture_inclusion;
use crate::e2e::fixture::{Fixture, FixtureDocs, SideEffectClass};
use anyhow::{Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub mod migration;

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureCallArgument {
    pub name: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureCallModel {
    pub fixture_id: String,
    pub language: String,
    pub function: String,
    pub is_async: bool,
    pub arguments: Vec<FixtureCallArgument>,
}

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
) -> Result<Vec<GeneratedFile>> {
    Ok(
        generate_snippet_artifacts(fixtures, languages, e2e, snippets, crate_config)?
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
) -> Result<Vec<GeneratedSnippet>> {
    validate_relative_path(Path::new(&snippets.output), "snippet output")?;
    let mut generated = BTreeMap::<PathBuf, GeneratedSnippet>::new();
    for fixture in fixtures.iter().filter(|fixture| fixture.docs.is_some()) {
        validate_requirements(fixture)?;
        for language in languages {
            if !fixture_inclusion(fixture, language, e2e).is_included() {
                continue;
            }
            let capabilities = capabilities(language, snippets, crate_config);
            if !matches!(snippet_inclusion(fixture, &capabilities), SnippetInclusion::Include) {
                continue;
            }
            let Some(lang) = parse_language(language) else { continue };
            let docs = fixture.docs.as_ref().expect("filtered docs fixtures have metadata");
            let path = snippet_path(&snippets.output, docs, &fixture.id, lang)?;
            let model = FixtureCallModel::from_fixture(fixture, language, e2e)?;
            let content = render_snippet(&model, fixture, docs, lang);
            let file = GeneratedFile {
                path: path.clone(),
                content,
                generated_header: false,
            };
            let snippet = GeneratedSnippet {
                file,
                fixture_id: fixture.id.clone(),
                fixture_source: fixture.source.clone(),
                language: language.clone(),
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

impl FixtureCallModel {
    pub fn from_fixture(fixture: &Fixture, language: &str, e2e: &E2eConfig) -> Result<Self> {
        let call = e2e.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let override_config = call.overrides.get(language);
        let function = override_config
            .and_then(|value| value.function.clone())
            .unwrap_or_else(|| call.function.clone());
        if function.is_empty() {
            bail!("fixture `{}` has no callable function for `{language}`", fixture.id);
        }
        let arguments = fixture
            .resolved_args(call)
            .iter()
            .map(|argument| FixtureCallArgument {
                name: argument.name.clone(),
                value: input_value(&fixture.input, &argument.field)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            })
            .collect();
        Ok(Self {
            fixture_id: fixture.id.clone(),
            language: language.to_string(),
            function,
            is_async: call.r#async,
            arguments,
        })
    }
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

fn input_value<'a>(input: &'a serde_json::Value, field: &str) -> Option<&'a serde_json::Value> {
    field.split('.').try_fold(input, |value, segment| value.get(segment))
}

fn render_snippet(model: &FixtureCallModel, fixture: &Fixture, docs: &FixtureDocs, language: Language) -> String {
    let arguments = model
        .arguments
        .iter()
        .map(|argument| render_literal(&argument.value, language))
        .collect::<Vec<_>>()
        .join(", ");
    let body = crate::e2e::template_env::render("snippets/call.jinja", minijinja::context! {
        language => model.language, function => model.function, arguments => arguments, async_call => model.is_async,
    }).trim_end().to_string();
    crate::e2e::template_env::render(
        "snippets/file.md.jinja",
        minijinja::context! {
            description => docs.description.as_deref().unwrap_or(&fixture.description),
            fence => crate::docs::naming::lang_code_fence(language),
            title => docs.title.as_deref().unwrap_or(crate::docs::naming::lang_display_name(language)), body => body,
        },
    )
}

fn render_literal(value: &serde_json::Value, language: Language) -> String {
    match value {
        serde_json::Value::Null => match language {
            Language::Python => "None".into(),
            Language::Ruby => "nil".into(),
            Language::Elixir => "nil".into(),
            _ => "null".into(),
        },
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => serde_json::to_string(value).expect("JSON strings serialize"),
        other => serde_json::to_string_pretty(other).expect("JSON values serialize"),
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
        "rust" => Language::Rust,
        "kotlin" => Language::Kotlin,
        "kotlin_android" => Language::KotlinAndroid,
        "swift" => Language::Swift,
        "dart" => Language::Dart,
        "gleam" => Language::Gleam,
        "zig" => Language::Zig,
        "c" => Language::C,
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
}
