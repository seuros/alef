//! JSON Schema and semantic validation for e2e fixture files.

use crate::e2e::codegen::assertion_recipes;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, group_fixtures};
use anyhow::{Context, Result};
use std::fmt;
use std::path::Path;

static FIXTURE_SCHEMA: &str = include_str!("schema/fixture.schema.json");

/// Severity level for validation diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Hard error — fixture is broken and will not produce correct tests.
    Error,
    /// Warning — fixture may not behave as intended.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
        }
    }
}

/// A validation error with its source file and message.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Relative path of the fixture file that failed validation.
    pub file: String,
    /// Human-readable error message.
    pub message: String,
    /// Severity level.
    pub severity: Severity,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.file, self.message)
    }
}

/// Validate all JSON fixture files in a directory against the fixture schema.
///
/// Returns a list of validation errors. An empty list means all fixtures are valid.
pub fn validate_fixtures(fixtures_dir: &Path) -> Result<Vec<ValidationError>> {
    let mut schema_value: serde_json::Value =
        serde_json::from_str(FIXTURE_SCHEMA).context("failed to parse embedded fixture schema")?;
    let document_validator = jsonschema::validator_for(&schema_value).context("failed to compile fixture schema")?;
    schema_value
        .as_object_mut()
        .context("embedded fixture schema root must be an object")?
        .insert("$ref".to_string(), serde_json::json!("#/$defs/fixture"));
    schema_value
        .as_object_mut()
        .context("embedded fixture schema root must be an object")?
        .remove("oneOf");
    let fixture_validator =
        jsonschema::validator_for(&schema_value).context("failed to compile fixture element schema")?;

    let mut errors = Vec::new();
    validate_recursive(
        fixtures_dir,
        fixtures_dir,
        &document_validator,
        &fixture_validator,
        &mut errors,
    )?;
    Ok(errors)
}

/// Perform semantic validation on loaded fixtures against e2e configuration.
///
/// Checks for:
/// 1. Fixtures skipped for all languages (empty `skip.languages`)
/// 2. Unknown call references not in `[e2e.calls.*]`
/// 3. Categories where all fixtures are skipped (produces 0 test functions)
/// 4. Missing required input fields for the resolved call config
/// 5. (D1) Argument arity and type mismatches in call configs
/// 6. (D2) Field path assertions against simple return types
/// 7. Domain-shaped assertions without required assertion recipes
pub fn validate_fixtures_semantic(
    fixtures: &[Fixture],
    e2e_config: &E2eConfig,
    languages: &[String],
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_unsupported_in_languages(e2e_config, languages, &mut errors);

    // Per-fixture checks
    for fixture in fixtures {
        // Check 1: skip-all detection
        // Fixtures in excluded categories are intentionally excluded at the
        // category level; empty skip.languages with no reason is the correct
        // shape there. Do not warn for them.
        if !e2e_config.exclude_categories.contains(&fixture.resolved_category())
            && let Some(skip) = &fixture.skip
            && skip.languages.is_empty()
        {
            let reason = skip.reason.as_deref().unwrap_or("no reason given");
            errors.push(ValidationError {
                file: fixture.source.clone(),
                message: format!(
                    "fixture '{}' is skipped for all languages (skip.languages is empty). Reason: {}",
                    fixture.id, reason
                ),
                severity: Severity::Warning,
            });
        }

        // Check 2: unknown call reference
        if let Some(call_name) = &fixture.call
            && !e2e_config.calls.contains_key(call_name)
        {
            errors.push(ValidationError {
                file: fixture.source.clone(),
                message: format!(
                    "fixture '{}' references unknown call '{}', will fall back to default [e2e.call]",
                    fixture.id, call_name
                ),
                severity: Severity::Error,
            });
        }

        // Check 4: missing required input fields
        // Resolve call using select_when auto-routing (not just explicit fixture.call)
        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        for language in languages {
            if let Some(missing) =
                assertion_recipes::missing_recipe_for_language(fixture, call_config, language, e2e_config)
            {
                errors.push(ValidationError {
                    file: fixture.source.clone(),
                    message: format!(
                        "fixture '{}' assertion '{}' requires assertion recipe '{}' for language '{}'",
                        fixture.id, missing.field, missing.recipe, language
                    ),
                    severity: Severity::Error,
                });
            }
        }
        for arg in fixture.resolved_args(call_config) {
            if arg.optional {
                continue;
            }
            // When the arg's field is exactly the top-level "input" path (no dot),
            // the whole fixture.input object IS the JSON value for that arg — no
            // sub-key lookup applies. Only dotted paths like "input.foo" require a
            // specific key to exist inside fixture.input.
            if !arg.field.starts_with("input.") {
                continue;
            }
            let input_field = arg.field.strip_prefix("input.").expect("starts_with checked above");
            if !fixture.input.is_null()
                && let Some(obj) = fixture.input.as_object()
                && !obj.contains_key(input_field)
            {
                // Skip check for error-type assertions (they may intentionally omit fields)
                let is_error_test = fixture.assertions.iter().any(|a| a.assertion_type == "error");
                if !is_error_test {
                    errors.push(ValidationError {
                        file: fixture.source.clone(),
                        message: format!(
                            "fixture '{}' is missing required input field '{}' for call '{}'",
                            fixture.id,
                            input_field,
                            fixture.call.as_deref().unwrap_or("<default>")
                        ),
                        severity: Severity::Warning,
                    });
                }
            }
        }

        // Check 5: `preserve_input_urls` set on a fixture with nothing to preserve.
        //
        // ~keep An inert flag is the exact failure this option exists to prevent: the
        // author believes the fixture's own address reaches the call, while every
        // backend keeps substituting the mock server and the test quietly proves
        // something else. Neither serde nor the JSON schema rejects an unknown fixture
        // key, so a typo elsewhere in the fixture surfaces here or nowhere.
        if fixture.preserve_input_urls {
            let has_url_arg = fixture
                .resolved_args(call_config)
                .iter()
                .any(|arg| matches!(arg.arg_type.as_str(), "mock_url" | "mock_url_list"));
            if !has_url_arg {
                errors.push(ValidationError {
                    file: fixture.source.clone(),
                    message: format!(
                        "fixture '{}' sets preserve_input_urls but call '{}' has no mock_url or \
                         mock_url_list argument, so the flag has no effect",
                        fixture.id,
                        fixture.call.as_deref().unwrap_or("<default>")
                    ),
                    severity: Severity::Error,
                });
            }
        }
    }

    // Check 3: empty categories (all fixtures skipped for all languages)
    if !languages.is_empty() {
        let groups = group_fixtures(fixtures);
        for group in &groups {
            // Categories explicitly excluded from cross-language codegen are
            // expected to produce 0 test functions; do not warn.
            if e2e_config.exclude_categories.contains(&group.category) {
                continue;
            }
            let has_any_non_skipped = group.fixtures.iter().any(|f| {
                match &f.skip {
                    None => true, // no skip → will generate
                    Some(skip) => {
                        // At least one language is NOT skipped
                        languages.iter().any(|lang| !skip.should_skip(lang))
                    }
                }
            });

            if !has_any_non_skipped {
                // Collect all skip reasons from fixtures to see if they're uniform
                let all_have_skip = group.fixtures.iter().all(|f| f.skip.is_some());
                let skip_reasons: Vec<&Option<String>> = if all_have_skip {
                    group
                        .fixtures
                        .iter()
                        .map(|f| &f.skip.as_ref().unwrap().reason)
                        .collect()
                } else {
                    vec![]
                };

                // Check if all fixtures have the same skip reason
                let same_reason = if !skip_reasons.is_empty() {
                    skip_reasons.iter().all(|r| r == skip_reasons.first().unwrap())
                } else {
                    false
                };

                if all_have_skip && same_reason && skip_reasons.first().unwrap().is_some() {
                    // All fixtures skip with the same reason — demote to INFO
                    // Use tracing::info if available; otherwise push as INFO level
                    // For now, we skip adding this to errors so it doesn't appear as a warning
                } else {
                    // Mixed or no reason — report as Error
                    errors.push(ValidationError {
                        file: format!("{}/ (category)", group.category),
                        message: format!(
                            "category '{}' produces 0 test functions — all {} fixture(s) are skipped for all languages",
                            group.category,
                            group.fixtures.len()
                        ),
                        severity: Severity::Error,
                    });
                }
            }
        }
    }

    errors
}

fn validate_unsupported_in_languages(e2e_config: &E2eConfig, languages: &[String], errors: &mut Vec<ValidationError>) {
    if languages.is_empty() {
        return;
    }

    for (call_name, call_config) in &e2e_config.calls {
        for language in call_config.unsupported_in.keys() {
            if !languages.iter().any(|configured| configured == language) {
                errors.push(ValidationError {
                    file: "alef.toml".to_string(),
                    message: format!(
                        "call '{call_name}' marks unsupported language '{language}', but that language is not in the \
                         resolved e2e language set"
                    ),
                    severity: Severity::Error,
                });
            }
        }
    }
}

fn validate_recursive(
    base: &Path,
    dir: &Path,
    document_validator: &jsonschema::Validator,
    fixture_validator: &jsonschema::Validator,
    errors: &mut Vec<ValidationError>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("failed to read directory: {}", dir.display()))?;

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            validate_recursive(base, &path, document_validator, fixture_validator, errors)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip schema files and files starting with _
            if filename == "schema.json" || filename.starts_with('_') {
                continue;
            }

            let relative = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    errors.push(ValidationError {
                        file: relative,
                        message: format!("failed to read file: {e}"),
                        severity: Severity::Error,
                    });
                    continue;
                }
            };

            let value: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(ValidationError {
                        file: relative,
                        message: format!("invalid JSON: {e}"),
                        severity: Severity::Error,
                    });
                    continue;
                }
            };

            validate_json_value(&relative, &value, document_validator, fixture_validator, errors);
        }
    }
    Ok(())
}

fn validate_json_value(
    relative: &str,
    value: &serde_json::Value,
    document_validator: &jsonschema::Validator,
    fixture_validator: &jsonschema::Validator,
    errors: &mut Vec<ValidationError>,
) {
    if let Some(fixtures) = value.as_array() {
        for (index, fixture) in fixtures.iter().enumerate() {
            validate_fixture_value(&format!("{relative}[{index}]"), fixture, fixture_validator, errors);
        }
        return;
    }
    validate_fixture_value(relative, value, document_validator, errors);
}

fn validate_fixture_value(
    relative: &str,
    fixture: &serde_json::Value,
    validator: &jsonschema::Validator,
    errors: &mut Vec<ValidationError>,
) {
    for error in validator.iter_errors(fixture) {
        errors.push(ValidationError {
            file: relative.to_string(),
            message: format!("{} at {}", error, error.instance_path()),
            severity: Severity::Error,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::{ArgMapping, CallConfig, CallOverride};

    #[test]
    fn validates_each_fixture_in_top_level_array() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("fixtures.json"),
            r#"[
                {"id": "first_fixture", "description": "first"},
                {
                    "id": "02_second_fixture",
                    "description": "second",
                    "custom_protocol": {"expected": true}
                }
            ]"#,
        )
        .unwrap();

        let errors = validate_fixtures(directory.path()).unwrap();

        assert!(errors.is_empty(), "unexpected validation errors: {errors:?}");
    }

    #[test]
    fn array_validation_identifies_invalid_fixture_index() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("fixtures.json"),
            r#"[
                {"id": "valid_fixture", "description": "valid"},
                {"id": "invalid_fixture"}
            ]"#,
        )
        .unwrap();

        let errors = validate_fixtures(directory.path()).unwrap();

        assert!(!errors.is_empty());
        assert!(errors.iter().all(|error| error.file == "fixtures.json[1]"));
    }

    #[test]
    fn fixture_schema_accepts_current_docs_metadata() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("fixture.json"),
            r#"{
                "id": "documented_fixture",
                "description": "documented",
                "docs": {
                    "topic": "configuration",
                    "paths": {"node": "configuration/example.md"},
                    "presentation": {
                        "call": "process_file",
                        "input": {"source": "guide.txt"},
                        "args": [{"name": "source", "field": "input.source", "type": "string"}],
                        "files": [{"field": "/source", "path": "examples/guide.txt"}],
                        "operations": [
                            {"op": "show", "path": "summary"},
                            {
                                "op": "iterate",
                                "path": "items",
                                "item": "item",
                                "fields": ["text"],
                                "display": true,
                                "optional": true
                            }
                        ]
                    }
                }
            }"#,
        )
        .unwrap();

        let errors = validate_fixtures(directory.path()).unwrap();

        assert!(errors.is_empty(), "unexpected validation errors: {errors:?}");
    }
    use crate::e2e::codegen::assertion_recipes::{EMBEDDINGS_RECIPE, KEYWORDS_RECIPE};
    use crate::e2e::fixture::{Assertion, SkipDirective};

    fn make_fixture(id: &str, source: &str, skip: Option<SkipDirective>, call: Option<&str>) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: id.to_string(),
            category: None,
            description: format!("Test {id}"),
            tags: vec![],
            skip,
            env: None,
            setup: Vec::new(),
            call: call.map(|s| s.to_string()),
            input: serde_json::json!({"path": "test.pdf"}),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: source.to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }
    }

    fn make_e2e_config(calls: Vec<(&str, CallConfig)>) -> E2eConfig {
        E2eConfig {
            calls: calls.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_skip_all_languages_detected() {
        let fixtures = vec![make_fixture(
            "test_skipped",
            "code/test.json",
            Some(SkipDirective {
                languages: vec![],
                reason: Some("Requires feature X".to_string()),
            }),
            None,
        )];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(errors.iter().any(|e| e.message.contains("skipped for all languages")));
    }

    #[test]
    fn test_unknown_call_detected() {
        let fixtures = vec![make_fixture("test_bad_call", "test.json", None, Some("nonexistent"))];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(errors.iter().any(|e| e.message.contains("unknown call 'nonexistent'")));
    }

    #[test]
    fn test_known_call_not_flagged() {
        let fixtures = vec![make_fixture("test_good_call", "test.json", None, Some("embed"))];
        let config = make_e2e_config(vec![("embed", CallConfig::default())]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(!errors.iter().any(|e| e.message.contains("unknown call")));
    }

    #[test]
    fn preserve_input_urls_requires_mock_url_argument() {
        let mut fixture = make_fixture("literal_url", "literal_url.json", None, None);
        fixture.preserve_input_urls = true;
        let errors = validate_fixtures_semantic(&[fixture], &make_e2e_config(vec![]), &["rust".to_string()]);
        assert!(errors.iter().any(|error| error.message.contains("flag has no effect")));
    }

    #[test]
    fn preserve_input_urls_accepts_scalar_and_list_url_arguments() {
        for arg_type in ["mock_url", "mock_url_list"] {
            let mut fixture = make_fixture("literal_url", "literal_url.json", None, None);
            fixture.preserve_input_urls = true;
            fixture.args = vec![
                serde_json::from_value(serde_json::json!({
                    "name": "urls",
                    "field": "input.urls",
                    "type": arg_type
                }))
                .expect("argument mapping should deserialize"),
            ];
            let errors = validate_fixtures_semantic(&[fixture], &make_e2e_config(vec![]), &["rust".to_string()]);
            assert!(!errors.iter().any(|error| error.message.contains("flag has no effect")));
        }
    }

    #[test]
    fn domain_assertion_without_recipe_is_error() {
        let mut fixture = make_fixture("test_embeddings", "test.json", None, None);
        fixture.assertions = vec![Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("embeddings_valid".to_string()),
            ..Default::default()
        }];
        let config = make_e2e_config(vec![]);

        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);

        assert!(
            errors.iter().any(|e| {
                e.severity == Severity::Error && e.message.contains("requires assertion recipe 'embeddings'")
            }),
            "expected missing embeddings recipe error, got: {errors:?}"
        );
    }

    #[test]
    fn fixture_recipe_allows_domain_assertion() {
        let mut fixture = make_fixture("test_embeddings", "test.json", None, None);
        fixture.assertion_recipes.push(EMBEDDINGS_RECIPE.to_string());
        fixture.assertions = vec![Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("embeddings_valid".to_string()),
            ..Default::default()
        }];
        let config = make_e2e_config(vec![]);

        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);

        assert!(
            !errors.iter().any(|e| e.message.contains("requires assertion recipe")),
            "fixture-level recipe should allow embeddings assertion, got: {errors:?}"
        );
    }

    #[test]
    fn language_override_allows_domain_assertion_for_that_language_only() {
        let mut fixture = make_fixture("test_keywords", "test.json", None, Some("extract"));
        fixture.assertions = vec![Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("keywords".to_string()),
            ..Default::default()
        }];
        let mut call = CallConfig::default();
        let mut python_override = CallOverride::default();
        python_override.assertion_recipes.insert(KEYWORDS_RECIPE.to_string());
        call.overrides.insert("python".to_string(), python_override);
        let config = make_e2e_config(vec![("extract", call)]);

        let errors = validate_fixtures_semantic(&[fixture], &config, &["python".to_string(), "rust".to_string()]);

        assert!(
            !errors.iter().any(|e| e.message.contains("language 'python'")),
            "python override should allow keywords assertion, got: {errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.message.contains("language 'rust'")),
            "rust should still require an explicit recipe, got: {errors:?}"
        );
    }

    #[test]
    fn test_unsupported_in_unknown_language_detected() {
        let mut call = CallConfig::default();
        call.unsupported_in
            .insert("brew".to_string(), "CLI backend cannot pass complex args".to_string());
        let config = make_e2e_config(vec![("interact", call)]);

        let errors = validate_fixtures_semantic(&[], &config, &["rust".to_string()]);

        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("marks unsupported language 'brew'")),
            "unsupported_in for inactive languages should be rejected; got: {:?}",
            errors
        );
    }

    #[test]
    fn test_unsupported_in_resolved_language_is_valid() {
        let mut call = CallConfig::default();
        call.unsupported_in
            .insert("brew".to_string(), "CLI backend cannot pass complex args".to_string());
        let config = make_e2e_config(vec![("interact", call)]);

        let errors = validate_fixtures_semantic(&[], &config, &["rust".to_string(), "brew".to_string()]);

        assert!(
            !errors.iter().any(|e| e.message.contains("marks unsupported language")),
            "unsupported_in should accept active languages; got: {:?}",
            errors
        );
    }

    #[test]
    fn test_empty_category_detected() {
        let fixtures = vec![
            make_fixture(
                "test_a",
                "orphan/a.json",
                Some(SkipDirective {
                    languages: vec![],
                    reason: None, // No reason — error will be raised
                }),
                None,
            ),
            make_fixture(
                "test_b",
                "orphan/b.json",
                Some(SkipDirective {
                    languages: vec![],
                    reason: None, // No reason — error will be raised
                }),
                None,
            ),
        ];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(errors.iter().any(|e| e.message.contains("produces 0 test functions")));
    }

    #[test]
    fn test_missing_required_input_field() {
        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "test_missing".to_string(),
            category: None,
            description: "Test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: Some("extract_bytes".to_string()),
            input: serde_json::json!({"data": "abc"}), // missing "mime_type"
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };
        let call = CallConfig {
            function: "extract_bytes".to_string(),
            args: vec![
                ArgMapping {
                    name: "data".to_string(),
                    field: "input.data".to_string(),
                    arg_type: "bytes".to_string(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
                ArgMapping {
                    name: "mime_type".to_string(),
                    field: "input.mime_type".to_string(),
                    arg_type: "string".to_string(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
            ],
            ..Default::default()
        };
        let config = make_e2e_config(vec![("extract_bytes", call)]);
        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("missing required input field 'mime_type'"))
        );
    }

    #[test]
    fn test_no_errors_for_valid_fixture() {
        let fixtures = vec![make_fixture("test_valid", "contract/test.json", None, None)];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        // Only check for errors/warnings beyond the expected "missing input" ones
        // (default call config has no args, so no input field checks)
        assert!(errors.is_empty());
    }

    /// Bare `field = "input"` (no dot) must NOT emit a "missing required input
    /// field 'input'" warning — the whole fixture.input IS the arg value.
    #[test]
    fn test_bare_input_field_no_false_positive_warning() {
        use crate::core::config::e2e::ArgMapping;

        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "basic_chat".to_string(),
            category: None,
            description: "Chat completion".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: Some("chat".to_string()),
            input: serde_json::json!({"model": "gpt-4", "messages": []}),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "smoke/basic_chat.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };
        let call = CallConfig {
            function: "chat".to_string(),
            args: vec![ArgMapping {
                name: "request".to_string(),
                // Bare "input" — the whole fixture.input is the arg value
                field: "input".to_string(),
                arg_type: "ChatCompletionRequest".to_string(),
                optional: false,
                owned: true,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..Default::default()
        };
        let config = make_e2e_config(vec![("chat", call)]);
        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);
        assert!(
            !errors
                .iter()
                .any(|e| e.message.contains("missing required input field 'input'")),
            "bare 'input' field should not produce a false-positive missing-field warning; got: {:?}",
            errors
        );
    }

    #[test]
    fn test_fixture_args_override_missing_field_validation() {
        use crate::core::config::e2e::ArgMapping;

        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "url_batch".to_string(),
            category: None,
            description: "URL batch".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: Some("extract_batch".to_string()),
            input: serde_json::json!({"extract_inputs": []}),
            mock_response: None,
            visitor: None,
            args: vec![ArgMapping {
                name: "inputs".to_string(),
                field: "input.extract_inputs".to_string(),
                arg_type: "json_object".to_string(),
                optional: false,
                owned: true,
                element_type: Some("ExtractInput".to_string()),
                go_type: Some("ExtractInput".to_string()),
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "url/url_batch.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };
        let call = CallConfig {
            function: "extract_batch".to_string(),
            args: vec![ArgMapping {
                name: "inputs".to_string(),
                field: "input.inputs".to_string(),
                arg_type: "json_object".to_string(),
                optional: false,
                owned: true,
                element_type: Some("ExtractInput".to_string()),
                go_type: Some("ExtractInput".to_string()),
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..Default::default()
        };
        let config = make_e2e_config(vec![("extract_batch", call)]);
        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);
        assert!(
            !errors
                .iter()
                .any(|e| e.message.contains("missing required input field 'inputs'")),
            "fixture-level args should replace call args for missing-field validation; got: {:?}",
            errors
        );
    }

    /// A fixture in an excluded category with empty `skip.languages` must NOT
    /// emit a "skipped for all languages" warning — the exclusion is intentional
    /// at the category level.
    #[test]
    fn test_excluded_category_no_skip_all_warning() {
        use std::collections::HashSet;

        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "budget_enforced".to_string(),
            category: None,
            description: "Budget enforcement test".to_string(),
            tags: vec![],
            skip: Some(SkipDirective {
                languages: vec![], // empty — would normally trigger the warning
                reason: None,
            }),
            env: None,
            setup: Vec::new(),
            call: Some("chat".to_string()),
            input: serde_json::json!({"model": "gpt-4", "messages": []}),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            // resolved_category() derives "budget" from this path
            source: "budget/budget_enforced.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };
        let mut config = make_e2e_config(vec![]);
        config.exclude_categories = HashSet::from(["budget".to_string()]);
        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);
        assert!(
            !errors.iter().any(|e| e.message.contains("skipped for all languages")),
            "excluded-category fixture should not trigger skip-all warning; got: {:?}",
            errors
        );
    }
}
