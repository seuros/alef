use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::{E2eConfig, SnippetConfig};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::e2e::codegen::fixture_inclusion;
use crate::e2e::codegen::recipe::E2eCallRecipe;
use crate::e2e::fixture::{Fixture, FixtureDocs, SideEffectClass};
use anyhow::{Result, bail};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

pub mod migration;

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureCallArgument {
    pub name: String,
    pub arg_type: String,
    pub optional: bool,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureCallModel {
    pub fixture_id: String,
    pub language: String,
    pub function: String,
    pub is_async: bool,
    pub arguments: Vec<FixtureCallArgument>,
    pub module: String,
    pub options_type: Option<String>,
    pub options_via: String,
    pub from_json_module: Option<String>,
    pub enum_module: Option<String>,
    pub enum_fields: BTreeMap<String, String>,
    pub client_factory: Option<String>,
    pub setup: Vec<FixtureSetupCall>,
    pub uses_mock_url: bool,
    pub handle_nested_types: HashMap<String, String>,
    pub handle_dict_types: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureSetupCall {
    pub function: String,
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
        let recipe = E2eCallRecipe::resolve(language, fixture, call, &[]);
        let function = override_config
            .and_then(|value| value.function.clone())
            .unwrap_or_else(|| call.function.clone());
        if function.is_empty() {
            bail!("fixture `{}` has no callable function for `{language}`", fixture.id);
        }
        let arguments = recipe
            .args
            .iter()
            .map(|argument| FixtureCallArgument {
                name: argument.name.clone(),
                arg_type: argument.arg_type.clone(),
                optional: argument.optional,
                value: input_value(&fixture.input, &argument.field)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            })
            .collect();
        let setup = fixture
            .setup
            .iter()
            .map(|setup| {
                let setup_call = e2e.resolve_call_for_fixture(
                    Some(&setup.call),
                    &fixture.id,
                    &fixture.resolved_category(),
                    &fixture.tags,
                    &setup.input,
                );
                let setup_override = setup_call.overrides.get(language);
                FixtureSetupCall {
                    function: setup_override
                        .and_then(|value| value.function.clone())
                        .unwrap_or_else(|| setup_call.function.clone()),
                    arguments: setup_call
                        .args
                        .iter()
                        .map(|argument| FixtureCallArgument {
                            name: argument.name.clone(),
                            arg_type: argument.arg_type.clone(),
                            optional: argument.optional,
                            value: input_value(&setup.input, &argument.field)
                                .cloned()
                                .unwrap_or(serde_json::Value::Null),
                        })
                        .collect(),
                }
            })
            .collect();
        let uses_mock_url = recipe
            .args
            .iter()
            .any(|argument| matches!(argument.arg_type.as_str(), "mock_url" | "mock_url_list"));
        Ok(Self {
            fixture_id: fixture.id.clone(),
            language: language.to_string(),
            function,
            is_async: override_config.and_then(|value| value.r#async).unwrap_or(call.r#async),
            arguments,
            module: override_config
                .and_then(|value| value.module.clone())
                .unwrap_or_else(|| call.module.clone()),
            options_type: recipe.options_type.map(str::to_string),
            options_via: recipe.options_via.to_string(),
            from_json_module: override_config.and_then(|value| value.from_json_module.clone()),
            enum_module: override_config.and_then(|value| value.enum_module.clone()),
            enum_fields: override_config
                .map(|value| value.enum_fields.clone().into_iter().collect())
                .unwrap_or_default(),
            client_factory: override_config.and_then(|value| value.client_factory.clone()),
            setup,
            uses_mock_url,
            handle_nested_types: override_config
                .map(|value| value.handle_nested_types.clone())
                .unwrap_or_default(),
            handle_dict_types: override_config
                .map(|value| value.handle_dict_types.clone())
                .unwrap_or_default(),
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
    if field == "input" {
        return Some(input);
    }
    field.split('.').try_fold(input, |value, segment| value.get(segment))
}

fn render_snippet(model: &FixtureCallModel, fixture: &Fixture, docs: &FixtureDocs, language: Language) -> String {
    let handle_setup = model
        .arguments
        .iter()
        .flat_map(|argument| render_handle_setup(argument, model, language))
        .collect::<Vec<_>>();
    let handle_functions = model
        .arguments
        .iter()
        .filter(|argument| argument.arg_type == "handle")
        .map(|argument| format!("create_{}", crate::codegen::naming::to_python_name(&argument.name)))
        .collect::<Vec<_>>();
    let handle_types = model
        .handle_nested_types
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let call = model
        .arguments
        .iter()
        .filter(|argument| should_render_argument(argument))
        .map(|argument| render_argument(argument, model, fixture, language))
        .collect::<Vec<_>>()
        .join(", ");
    let setup = model
        .setup
        .iter()
        .map(|setup| {
            let arguments = setup
                .arguments
                .iter()
                .map(|argument| render_literal(&argument.value, language))
                .collect::<Vec<_>>()
                .join(", ");
            crate::e2e::template_env::render(
                "snippets/setup_call.jinja",
                minijinja::context! {
                    language => model.language, function => setup.function, arguments => arguments,
                },
            )
            .trim_end()
            .to_string()
        })
        .collect::<Vec<_>>();
    let setup_functions = model
        .setup
        .iter()
        .map(|setup| setup.function.as_str())
        .collect::<Vec<_>>();
    let uses_options_type = model.arguments.iter().any(|argument| {
        matches!(argument.arg_type.as_str(), "json_object" | "handle")
            && !argument.value.is_null()
            && model.options_type.is_some()
    });
    let enum_types = model.enum_fields.values().collect::<BTreeSet<_>>();
    let body = crate::e2e::template_env::render(
        "snippets/call.jinja",
        minijinja::context! {
            language => model.language, module => model.module, function => model.function, arguments => call,
        async_call => model.is_async, setup => setup, client_factory => model.client_factory,
        setup_functions => setup_functions,
            handle_setup => handle_setup, handle_functions => handle_functions, handle_types => handle_types,
            options_type => model.options_type, uses_mock_url => model.uses_mock_url,
            uses_options_type => uses_options_type, from_json_module => model.from_json_module,
            enum_module => model.enum_module, enum_types => enum_types,
            fixture_id => model.fixture_id,
        },
    )
    .trim_end()
    .to_string();
    crate::e2e::template_env::render(
        "snippets/file.md.jinja",
        minijinja::context! {
            description => docs.description.as_deref().unwrap_or(&fixture.description),
            fence => crate::docs::naming::lang_code_fence(language),
            title => docs.title.as_deref().unwrap_or(crate::docs::naming::lang_display_name(language)), body => body,
            fixture_id => fixture.id, language => model.language, requirements => fixture.requirements,
            side_effect => side_effect_name(docs.side_effects),
        },
    )
}

fn should_render_argument(argument: &FixtureCallArgument) -> bool {
    !argument.optional || !argument.value.is_null()
}

fn render_argument(
    argument: &FixtureCallArgument,
    model: &FixtureCallModel,
    fixture: &Fixture,
    language: Language,
) -> String {
    if argument.arg_type == "handle" && language == Language::Python {
        return crate::codegen::naming::to_python_name(&argument.name);
    }
    if argument.arg_type == "mock_url" {
        return mock_url_expression(&fixture.id, language);
    }
    let literal = if language == Language::Rust && argument.arg_type == "json_object" {
        crate::e2e::template_env::render(
            "snippets/rust_json_argument.jinja",
            minijinja::context! { json => render_literal(&argument.value, language) },
        )
        .trim_end()
        .to_string()
    } else {
        render_literal(&argument.value, language)
    };
    let Some(options_type) = model
        .options_type
        .as_deref()
        .filter(|_| argument.arg_type == "json_object" && argument.value.is_object())
    else {
        return literal;
    };
    render_options(
        options_type,
        &argument.value,
        &model.options_via,
        &model.enum_fields,
        language,
    )
    .unwrap_or(literal)
}

fn render_handle_setup(argument: &FixtureCallArgument, model: &FixtureCallModel, language: Language) -> Vec<String> {
    if argument.arg_type != "handle" || language != Language::Python {
        return Vec::new();
    }
    let variable = crate::codegen::naming::to_python_name(&argument.name);
    let constructor = format!("create_{variable}");
    let object = argument.value.as_object();
    let fields = object
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .iter()
                .map(|(key, value)| {
                    let name = crate::codegen::naming::to_python_name(key);
                    let value = crate::e2e::codegen::python::build_handle_kwarg_value(
                        key,
                        value,
                        &model.handle_nested_types,
                        &model.handle_dict_types,
                    );
                    (name, value)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let config_variable = format!("{variable}_config");
    let argument_value = if argument.value.is_null() || object.is_some_and(serde_json::Map::is_empty) {
        "None".to_string()
    } else if object.is_some_and(|value| !value.is_empty()) {
        let fields = fields
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(", ");
        let config_setup = crate::e2e::template_env::render(
            "snippets/handle_config_setup.jinja",
            minijinja::context! { variable => config_variable, options_type => model.options_type, fields => fields },
        )
        .trim_end()
        .to_string();
        return vec![
            config_setup,
            crate::e2e::template_env::render(
                "snippets/handle_create_setup.jinja",
                minijinja::context! { variable => variable, constructor => constructor, argument => config_variable },
            )
            .trim_end()
            .to_string(),
        ];
    } else {
        render_literal(&argument.value, language)
    };
    vec![
        crate::e2e::template_env::render(
            "snippets/handle_create_setup.jinja",
            minijinja::context! { variable => variable, constructor => constructor, argument => argument_value },
        )
        .trim_end()
        .to_string(),
    ]
}

fn render_options(
    options_type: &str,
    value: &serde_json::Value,
    via: &str,
    enum_fields: &BTreeMap<String, String>,
    language: Language,
) -> Option<String> {
    let object = value.as_object()?;
    let fields = object
        .iter()
        .map(|(key, value)| {
            if language != Language::Python {
                return format!("{key}={}", render_literal(value, language));
            }
            let enum_type = enum_fields.get(key);
            crate::e2e::template_env::render(
                "snippets/python_option_field.jinja",
                minijinja::context! {
                    name => crate::codegen::naming::to_python_name(key),
                    value => render_literal(value, language), enum_type => enum_type,
                },
            )
            .trim_end()
            .to_string()
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(match (language, via) {
        (Language::Python, "from_json") => {
            let json = serde_json::to_string(value).expect("JSON option values serialize");
            format!(
                "{options_type}.from_json({})",
                render_literal(&serde_json::Value::String(json), language)
            )
        }
        (Language::Python, "dict") => render_literal(value, language),
        (Language::Python, _) => format!("{options_type}({fields})"),
        (Language::Ruby, _) => format!("{options_type}.new({})", fields.replace('=', ": ")),
        (Language::Node | Language::Wasm, _) => format!("new {options_type}({})", render_literal(value, language)),
        _ => return None,
    })
}

fn mock_url_expression(fixture_id: &str, language: Language) -> String {
    match language {
        Language::Python => format!("f\"{{os.environ['MOCK_SERVER_URL']}}/fixtures/{fixture_id}\""),
        Language::Ruby => format!("\"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\""),
        Language::Node | Language::Wasm => format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`"),
        _ => format!("\"${{MOCK_SERVER_URL}}/fixtures/{fixture_id}\""),
    }
}

fn side_effect_name(value: SideEffectClass) -> &'static str {
    match value {
        SideEffectClass::None | SideEffectClass::Local => "safe",
        SideEffectClass::Network | SideEffectClass::ExternalMutation => "network",
    }
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
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(|value| render_literal(value, language))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        serde_json::Value::Object(values) => {
            let separator = if language == Language::Ruby { " => " } else { ": " };
            let fields = values
                .iter()
                .map(|(key, value)| {
                    format!(
                        "{}{separator}{}",
                        serde_json::to_string(key).expect("JSON object keys serialize"),
                        render_literal(value, language)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{fields}}}")
        }
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
    fn whole_input_argument_does_not_look_for_a_nested_input_field() {
        let input = serde_json::json!({"text": "hello"});
        assert_eq!(input_value(&input, "input"), Some(&input));
    }

    #[test]
    fn dynamic_language_object_literals_are_native() {
        let value = serde_json::json!({"enabled": true, "nested": {"count": 2}});
        assert_eq!(
            render_literal(&value, Language::Python),
            r#"{"enabled": true, "nested": {"count": 2}}"#
        );
        assert_eq!(
            render_literal(&value, Language::Ruby),
            r#"{"enabled" => true, "nested" => {"count" => 2}}"#
        );
    }

    #[test]
    fn rust_json_arguments_use_json_macro() {
        let model = FixtureCallModel {
            fixture_id: "json_call".into(),
            language: "rust".into(),
            function: "run".into(),
            is_async: true,
            arguments: vec![FixtureCallArgument {
                name: "payload".into(),
                arg_type: "json_object".into(),
                optional: false,
                value: serde_json::json!({"enabled": true}),
            }],
            module: "sample".into(),
            options_type: None,
            options_via: "kwargs".into(),
            from_json_module: None,
            enum_module: None,
            enum_fields: BTreeMap::new(),
            client_factory: None,
            setup: Vec::new(),
            uses_mock_url: false,
            handle_nested_types: HashMap::new(),
            handle_dict_types: HashSet::new(),
        };
        let fixture = Fixture {
            id: "json_call".into(),
            description: "Call with JSON".into(),
            ..Fixture::default()
        };
        let docs = FixtureDocs {
            topic: "api".into(),
            stem: None,
            title: None,
            description: None,
            side_effects: SideEffectClass::None,
        };

        let rendered = render_snippet(&model, &fixture, &docs, Language::Rust);

        assert!(rendered.contains(r#"let result = run(serde_json::json!({"enabled": true})).await;"#));
    }

    #[test]
    fn python_options_wrap_enum_fields_and_omit_absent_optional_args() {
        let fields = BTreeMap::from([
            ("heading_style".into(), "HeadingStyle".into()),
            ("output_format".into(), "OutputFormat".into()),
        ]);
        let rendered = render_options(
            "Options",
            &serde_json::json!({"heading_style": "Atx", "output_format": "Markdown"}),
            "kwargs",
            &fields,
            Language::Python,
        )
        .expect("object options render");

        assert_eq!(
            rendered,
            r#"Options(heading_style=HeadingStyle("Atx"), output_format=OutputFormat("Markdown"))"#
        );

        let arguments = [FixtureCallArgument {
            name: "encoding".into(),
            arg_type: "string".into(),
            optional: true,
            value: serde_json::Value::Null,
        }];
        assert!(!should_render_argument(&arguments[0]));
    }

    #[test]
    fn language_aliases_include_core_and_ffi_targets() {
        assert_eq!(parse_language("rust_core"), Some(Language::Rust));
        assert_eq!(parse_language("ffi"), Some(Language::C));
    }

    #[test]
    fn rendered_markdown_preserves_side_effect_metadata() {
        let fixture = Fixture {
            id: "network_call".into(),
            description: "Call the service".into(),
            ..Fixture::default()
        };
        let docs = FixtureDocs {
            topic: "api".into(),
            stem: None,
            title: None,
            description: None,
            side_effects: SideEffectClass::Network,
        };
        let model = FixtureCallModel {
            fixture_id: fixture.id.clone(),
            language: "python".into(),
            function: "run".into(),
            is_async: false,
            arguments: Vec::new(),
            module: "sample".into(),
            options_type: None,
            options_via: "kwargs".into(),
            from_json_module: None,
            enum_module: None,
            enum_fields: BTreeMap::new(),
            client_factory: None,
            setup: Vec::new(),
            uses_mock_url: false,
            handle_nested_types: HashMap::new(),
            handle_dict_types: HashSet::new(),
        };
        let rendered = render_snippet(&model, &fixture, &docs, Language::Python);
        assert!(
            rendered.starts_with("---\nid: network_call\nlanguage: python\nrequires: []\nside_effect: network\n---")
        );
        assert!(rendered.contains("from sample import run"));
    }

    #[test]
    fn renders_options_client_factory_and_mock_url_recipe() {
        let fixture = Fixture {
            id: "configured_call".into(),
            description: "Configured call".into(),
            input: serde_json::json!({"options": {"enabled": true}, "url": "/request"}),
            ..Fixture::default()
        };
        let model = FixtureCallModel {
            fixture_id: fixture.id.clone(),
            language: "python".into(),
            function: "execute".into(),
            is_async: true,
            arguments: vec![
                FixtureCallArgument {
                    name: "options".into(),
                    arg_type: "json_object".into(),
                    optional: false,
                    value: fixture.input["options"].clone(),
                },
                FixtureCallArgument {
                    name: "url".into(),
                    arg_type: "mock_url".into(),
                    optional: false,
                    value: fixture.input["url"].clone(),
                },
            ],
            module: "sample".into(),
            options_type: Some("Options".into()),
            options_via: "kwargs".into(),
            from_json_module: None,
            enum_module: None,
            enum_fields: BTreeMap::new(),
            client_factory: Some("create_client".into()),
            setup: Vec::new(),
            uses_mock_url: true,
            handle_nested_types: HashMap::new(),
            handle_dict_types: HashSet::new(),
        };
        let docs = FixtureDocs {
            topic: "api".into(),
            stem: None,
            title: None,
            description: None,
            side_effects: SideEffectClass::Network,
        };
        let rendered = render_snippet(&model, &fixture, &docs, Language::Python);
        assert!(rendered.contains("client = create_client(\"test-key\", f\"{os.environ['MOCK_SERVER_URL']}"));
        assert!(rendered.contains("result = await client.execute(Options(enabled=true), f\"{os.environ"));
    }

    #[test]
    fn renders_python_handle_setup_before_async_call() {
        let fixture = Fixture {
            id: "handle_call".into(),
            description: "Call with a configured handle".into(),
            input: serde_json::json!({
                "config": {
                    "browser": {"mode": "auto"},
                    "headers": {"x-test": "enabled"},
                    "request_timeout": 2000
                },
                "url": "/request"
            }),
            ..Fixture::default()
        };
        let model = FixtureCallModel {
            fixture_id: fixture.id.clone(),
            language: "python".into(),
            function: "fetch".into(),
            is_async: true,
            arguments: vec![
                FixtureCallArgument {
                    name: "engine".into(),
                    arg_type: "handle".into(),
                    optional: false,
                    value: fixture.input["config"].clone(),
                },
                FixtureCallArgument {
                    name: "url".into(),
                    arg_type: "mock_url".into(),
                    optional: false,
                    value: fixture.input["url"].clone(),
                },
            ],
            module: "sample".into(),
            options_type: Some("EngineConfig".into()),
            options_via: "kwargs".into(),
            from_json_module: None,
            enum_module: None,
            enum_fields: BTreeMap::new(),
            client_factory: None,
            setup: Vec::new(),
            uses_mock_url: true,
            handle_nested_types: HashMap::from([("browser".into(), "BrowserConfig".into())]),
            handle_dict_types: HashSet::from(["headers".into()]),
        };
        let docs = FixtureDocs {
            topic: "api".into(),
            stem: None,
            title: None,
            description: None,
            side_effects: SideEffectClass::Network,
        };

        let rendered = render_snippet(&model, &fixture, &docs, Language::Python);

        assert!(rendered.contains("from sample import fetch, EngineConfig, create_engine, BrowserConfig"));
        assert!(rendered.contains("engine_config = EngineConfig("));
        assert!(rendered.contains("browser=BrowserConfig(mode=\"auto\"),"));
        assert!(rendered.contains("headers={\"x-test\": \"enabled\"},"));
        assert!(rendered.contains("request_timeout=2)"));
        assert!(rendered.contains("engine = create_engine(engine_config)"));
        assert!(rendered.contains("result = await fetch(engine, f\"{os.environ['MOCK_SERVER_URL']}"));
    }
}
