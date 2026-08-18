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

/// The `alef.toml` file every field-classification diagnostic points at. These entries live in
/// config, not in a fixture, so the `file` slot names the config rather than a fixture path. ~keep
const CONFIG_FILE_LABEL: &str = "alef.toml";

/// Which field-classification table an entry came from, and what it claims about the field.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldClassification {
    /// `fields_optional` — the accessor renderers unwrap this path's leaf as an `Option`.
    Optional,
    /// `fields_array` — the accessor renderers index this path's leaf at `[0]`.
    Array,
}

impl FieldClassification {
    fn table(self) -> &'static str {
        match self {
            Self::Optional => "fields_optional",
            Self::Array => "fields_array",
        }
    }

    /// What the emitted accessor does when the entry is honoured, quoted back to the operator so
    /// the diagnostic explains the compile error the entry would otherwise have caused. ~keep
    fn emitted_effect(self) -> &'static str {
        match self {
            Self::Optional => "an Option unwrap (`.as_ref().unwrap()` / `?.` / `!!` / `.?`)",
            Self::Array => "an element index (`[0]`)",
        }
    }

    /// True when the IR's own shape for a field agrees with what this table claims about it.
    fn agrees_with(self, field: &crate::core::ir::FieldDef) -> bool {
        use crate::core::ir::TypeRef;
        match self {
            // `FieldDef::ty` has already had one `Option<..>` layer peeled off into
            // `FieldDef::optional` (see `extract::extractor::helpers::fields::extract_field`), so
            // both spellings have to be accepted -- hand-built `TypeDef`s keep the wrapper. ~keep
            Self::Optional => field.optional || matches!(field.ty, TypeRef::Optional(_)),
            Self::Array => is_indexable(&field.ty),
        }
    }

    /// Whether this classification is consistent with the ELEMENT a subscripted entry
    /// (`open_graph[title]`, `choices[0]`) reaches on `field`.
    ///
    /// `Optional` clears any subscriptable container: a map key or a list index is a lookup that
    /// can miss, which is precisely what every host binding models as an optional. `Array` has to
    /// look one level deeper — `foo[0]` being itself indexable means `foo` is a nested
    /// collection. ~keep
    fn agrees_with_element_of(self, field: &crate::core::ir::FieldDef) -> bool {
        match element_shape(&field.ty) {
            ElementShape::NotSubscriptable => false,
            ElementShape::Opaque => true,
            ElementShape::Known(inner) => match self {
                Self::Optional => true,
                Self::Array => is_indexable(inner),
            },
        }
    }
}

/// True for the IR shapes an `[0]` index is legal against.
///
/// `Json` and `Map` are in deliberately: a `serde_json::Value` field can carry an array, and
/// `FieldResolver::inject_array_indexing` upgrades a numeric map key on a registered array field
/// to an index. Neither is a mis-declaration the operator could act on. ~keep
fn is_indexable(ty: &crate::core::ir::TypeRef) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Vec(_) | TypeRef::Bytes | TypeRef::Json | TypeRef::Map(_, _) => true,
        TypeRef::Optional(inner) => is_indexable(inner),
        _ => false,
    }
}

/// What one `[...]` subscript against `ty` lands on.
enum ElementShape<'a> {
    /// The subscript is legal and reaches this type.
    Known(&'a crate::core::ir::TypeRef),
    /// The subscript is legal but the element type is not expressible as a `TypeRef` here
    /// (`Vec<u8>` indexes to a byte). Subscripting is not a mis-declaration, so this clears the
    /// entry the same way `IrAbsent` does rather than manufacturing a diagnostic. ~keep
    Opaque,
    /// `ty` cannot be subscripted at all — the entry's path is wrong, and that IS actionable.
    NotSubscriptable,
}

/// Resolve one `[...]` subscript against an IR type.
fn element_shape(ty: &crate::core::ir::TypeRef) -> ElementShape<'_> {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Vec(inner) | TypeRef::Map(_, inner) => ElementShape::Known(inner),
        // A `serde_json::Value` subscripts to another `Value`, whatever it carries. ~keep
        TypeRef::Json => ElementShape::Known(ty),
        TypeRef::Bytes => ElementShape::Opaque,
        TypeRef::Optional(inner) => element_shape(inner),
        _ => ElementShape::NotSubscriptable,
    }
}

/// What the core IR knows about the leaf field name a classification entry names.
///
/// Mirrors `e2e::codegen::c::assertions::TargetParams` and `e2e::codegen::c::ResultTypeName`:
/// "the IR was never consulted" and "the IR was consulted and had nothing" are different facts,
/// and collapsing them into a bare `Option` is what would let an unverifiable entry be reported
/// as a wrong one. The halves of one rule must agree on what an absent IR licenses. ~keep
enum IrFieldShape<'a> {
    /// The IR declares at least one field with this name — these are every occurrence of it,
    /// across every type. Enough to rule on the entry.
    Known(Vec<&'a crate::core::ir::FieldDef>),
    /// No IR was supplied at all, so nothing was consulted and nothing can be concluded. A
    /// legitimate, common state (unit tests and several snippet entry points generate from empty
    /// IR slices), so it produces no diagnostic rather than refusing. Refusing here would fail
    /// every IR-less caller — a far larger blast radius than the defect being fixed. ~keep
    IrAbsent,
    /// The IR was there to consult and no type declares a field of this name. That is not proof
    /// the entry is wrong: virtual namespace prefixes, streaming pseudo-fields and synthetic
    /// assertion paths all legitimately name things the IR has never heard of, exactly as
    /// `FieldResolver::is_valid_for_result` documents. Unverifiable, so it warns rather than
    /// failing. ~keep
    Unresolvable,
}

/// Every occurrence of `leaf` across the IR's type definitions, or why there is none.
fn ir_field_shape<'a>(leaf: &str, type_defs: &'a [crate::core::ir::TypeDef]) -> IrFieldShape<'a> {
    if type_defs.is_empty() {
        return IrFieldShape::IrAbsent;
    }
    let occurrences: Vec<_> = type_defs
        .iter()
        .flat_map(|type_def| type_def.fields.iter())
        .filter(|field| field.name == leaf)
        .collect();
    if occurrences.is_empty() {
        IrFieldShape::Unresolvable
    } else {
        IrFieldShape::Known(occurrences)
    }
}

/// The field name a classification entry's accessor lands on, plus whether the entry reaches it
/// through a `[0]` / `[]` / `["key"]` subscript.
///
/// The renderers check the FULL prefix path at every segment against `fields_optional` /
/// `fields_array` (see `field_access::optional_renderers::push_key_field_name`), so an entry
/// `metadata.article.tags` is a claim about `tags` — not about `metadata` or `article`. ~keep
///
/// The subscript flag is not cosmetic: `open_graph[title]` says nothing about `open_graph`, it
/// classifies the ELEMENT that subscript reaches. Ruling on the container with the bare-field
/// predicate is what rejected every legal `HashMap<String, String>` key lookup as "contradicts
/// the core IR" — the map is exactly the right home for an optional `[title]`, and exactly the
/// wrong home for an optional bare field. ~keep
fn classification_target(entry: &str) -> (&str, bool) {
    let last = entry.rsplit('.').next().unwrap_or(entry);
    match last.split_once('[') {
        Some((name, _)) => (name.trim(), true),
        None => (last.trim(), false),
    }
}

/// The Rust spelling of an IR field's type, for quoting back in a diagnostic.
fn describe_field_type(field: &crate::core::ir::FieldDef) -> String {
    let inner = describe_type_ref(&field.ty);
    if field.optional {
        format!("Option<{inner}>")
    } else {
        inner
    }
}

fn describe_type_ref(ty: &crate::core::ir::TypeRef) -> String {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Primitive(primitive) => format!("{primitive:?}").to_lowercase(),
        TypeRef::String => "String".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", describe_type_ref(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", describe_type_ref(inner)),
        TypeRef::Map(key, value) => format!("HashMap<{}, {}>", describe_type_ref(key), describe_type_ref(value)),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Path => "PathBuf".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "Duration".to_string(),
    }
}

/// Check every `fields_optional` / `fields_array` entry in `e2e_config` against the core IR.
///
/// A wrong entry used to reach the operator as a compiler diagnostic pointed at GENERATED code:
/// `metadata.article.tags` declared optional against a plain `Vec<String>` emits
/// `.as_ref().unwrap().len()` and fails with "type annotations needed", naming a file the
/// operator never wrote and no config line at all. This turns that into one line naming the
/// table, the entry, and the type the IR actually declares.
///
/// The check is on the entry's LEAF field name, and an occurrence of that name that agrees with
/// the entry anywhere in the IR clears it — the same predicate
/// [`FieldResolver::ir_field_sets`] already uses for reachability, for the same reason: a bare
/// field name cannot be pinned to one result type from here, so agrees-on-any-type has to win or
/// a name shared by two structs would produce a false failure. That trade makes this check
/// under-report (an entry wrong on the type it is actually used against still passes when a
/// same-named field elsewhere agrees) and never over-report. ~keep
///
/// [`FieldResolver::ir_field_sets`]: crate::e2e::field_access::FieldResolver::ir_field_sets
pub fn validate_field_classifications(
    e2e_config: &E2eConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let mut tables: Vec<(&std::collections::HashSet<String>, FieldClassification, String)> = vec![
        (
            &e2e_config.fields_optional,
            FieldClassification::Optional,
            "[e2e]".into(),
        ),
        (&e2e_config.fields_array, FieldClassification::Array, "[e2e]".into()),
        (
            &e2e_config.call.fields_optional,
            FieldClassification::Optional,
            "[e2e.call]".into(),
        ),
        (
            &e2e_config.call.fields_array,
            FieldClassification::Array,
            "[e2e.call]".into(),
        ),
    ];
    for (name, call) in &e2e_config.calls {
        let key = format!("[e2e.calls.{name}]");
        tables.push((&call.fields_optional, FieldClassification::Optional, key.clone()));
        tables.push((&call.fields_array, FieldClassification::Array, key));
    }
    for (entries, classification, config_key) in tables {
        check_classification_table(entries, classification, &config_key, type_defs, &mut errors);
    }
    errors
}

fn check_classification_table(
    entries: &std::collections::HashSet<String>,
    classification: FieldClassification,
    config_key: &str,
    type_defs: &[crate::core::ir::TypeDef],
    errors: &mut Vec<ValidationError>,
) {
    let table = classification.table();
    // Sorted so the diagnostics one run emits are stable across `HashSet` iteration order. ~keep
    let mut sorted: Vec<&String> = entries.iter().collect();
    sorted.sort_unstable();
    for entry in sorted {
        let (leaf, subscripted) = classification_target(entry);
        if leaf.is_empty() {
            continue;
        }
        match ir_field_shape(leaf, type_defs) {
            IrFieldShape::IrAbsent => {}
            IrFieldShape::Unresolvable => errors.push(ValidationError {
                file: CONFIG_FILE_LABEL.to_string(),
                message: format!(
                    "{config_key}.{table} entry `{entry}` is unverified: no type in the core IR declares a \
                     field named `{leaf}`, so alef cannot confirm the classification (expected for virtual \
                     namespace prefixes and synthetic/streaming paths, a typo otherwise)"
                ),
                severity: Severity::Warning,
            }),
            IrFieldShape::Known(occurrences) => {
                let agrees = occurrences.iter().any(|field| {
                    if subscripted {
                        classification.agrees_with_element_of(field)
                    } else {
                        classification.agrees_with(field)
                    }
                });
                if agrees {
                    continue;
                }
                let mut declared: Vec<String> = occurrences.iter().map(|field| describe_field_type(field)).collect();
                declared.sort_unstable();
                declared.dedup();
                let effect = if subscripted {
                    format!(
                        "subscripts it and emits {} against the element",
                        classification.emitted_effect()
                    )
                } else {
                    format!("emits {} against it", classification.emitted_effect())
                };
                errors.push(ValidationError {
                    file: CONFIG_FILE_LABEL.to_string(),
                    message: format!(
                        "{config_key}.{table} entry `{entry}` contradicts the core IR: `{leaf}` is declared as \
                         {} there, and honouring this entry {effect} — remove the entry or fix the path",
                        declared.join(" / "),
                    ),
                    severity: Severity::Error,
                });
            }
        }
    }
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

#[cfg(test)]
mod field_classification_tests {
    use super::{Severity, validate_field_classifications};
    use crate::core::config::e2e::{CallConfig, E2eConfig};
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};
    use std::collections::HashSet;

    fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..FieldDef::default()
        }
    }

    /// One IR type carrying the shapes the assertions below rule on: a plain `Vec<String>`, a
    /// genuine `Option<String>`, and a plain `String`.
    fn article_ir() -> Vec<TypeDef> {
        vec![TypeDef {
            name: "ArticleMetadata".to_string(),
            fields: vec![
                field("tags", TypeRef::Vec(Box::new(TypeRef::String)), false),
                field("subtitle", TypeRef::String, true),
                field("title", TypeRef::String, false),
                field(
                    "open_graph",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                ),
            ],
            ..TypeDef::default()
        }]
    }

    fn config_with(optional: &[&str], array: &[&str]) -> E2eConfig {
        E2eConfig {
            fields_optional: optional.iter().map(|f| (*f).to_string()).collect(),
            fields_array: array.iter().map(|f| (*f).to_string()).collect(),
            ..E2eConfig::default()
        }
    }

    fn errors_only(diagnostics: &[super::ValidationError]) -> Vec<&super::ValidationError> {
        diagnostics
            .iter()
            .filter(|diag| diag.severity == Severity::Error)
            .collect()
    }

    /// The defect this check exists for: `metadata.article.tags` declared optional against a
    /// plain `Vec<String>` used to surface only as "type annotations needed" pointed at
    /// generated code, with nothing naming the config line.
    #[test]
    fn optional_entry_naming_a_non_optional_ir_field_is_an_error_that_names_key_and_type() {
        let diagnostics = validate_field_classifications(&config_with(&["metadata.article.tags"], &[]), &article_ir());

        let errors = errors_only(&diagnostics);
        assert_eq!(errors.len(), 1, "expected exactly one hard error, got: {diagnostics:?}");
        let message = &errors[0].message;
        assert!(message.contains("[e2e].fields_optional"), "names the table: {message}");
        assert!(
            message.contains("`metadata.article.tags`"),
            "names the entry: {message}"
        );
        assert!(message.contains("Vec<String>"), "names the real IR type: {message}");
        assert_eq!(errors[0].file, "alef.toml");
    }

    /// The check must not fire on a correct declaration — the field really is `Option<String>`.
    #[test]
    fn optional_entry_naming_a_genuinely_optional_ir_field_produces_no_error() {
        let diagnostics = validate_field_classifications(&config_with(&["metadata.subtitle"], &[]), &article_ir());

        assert!(
            errors_only(&diagnostics).is_empty(),
            "a correct fields_optional entry must not be rejected: {diagnostics:?}"
        );
    }

    /// A subscripted entry classifies the ELEMENT the subscript reaches, not the container. Every
    /// key lookup on a `HashMap<String, String>` used to be rejected as contradicting the IR
    /// because the subscript was stripped and the map itself was ruled on as a bare field — which
    /// is how a correct `metadata.document.open_graph[title]` failed the whole run.
    #[test]
    fn optional_entry_subscripting_a_map_field_produces_no_error() {
        let diagnostics = validate_field_classifications(
            &config_with(&["metadata.document.open_graph[title]"], &[]),
            &article_ir(),
        );

        assert!(
            errors_only(&diagnostics).is_empty(),
            "a map key lookup is legitimately optional: {diagnostics:?}"
        );
    }

    /// Indexing a list element as optional is the same shape and must also pass.
    #[test]
    fn optional_entry_subscripting_a_vec_field_produces_no_error() {
        let diagnostics = validate_field_classifications(&config_with(&["tags[0]"], &[]), &article_ir());

        assert!(
            errors_only(&diagnostics).is_empty(),
            "a list index is legitimately optional: {diagnostics:?}"
        );
    }

    /// Subscript-awareness must not become a blanket amnesty: a subscript against a scalar is
    /// still a wrong path, and the diagnostic has to say the subscript is the problem.
    #[test]
    fn optional_entry_subscripting_a_scalar_field_is_still_an_error() {
        let diagnostics = validate_field_classifications(&config_with(&["title[0]"], &[]), &article_ir());

        let errors = errors_only(&diagnostics);
        assert_eq!(errors.len(), 1, "expected exactly one hard error, got: {diagnostics:?}");
        assert!(
            errors[0].message.contains("subscripts it"),
            "got: {}",
            errors[0].message
        );
        assert!(errors[0].message.contains("String"), "got: {}", errors[0].message);
    }

    /// `fields_array` on a subscripted entry claims the element is itself indexable, so a
    /// `HashMap<String, String>` value — a plain `String` — must still be rejected.
    #[test]
    fn array_entry_subscripting_a_map_of_scalars_is_an_error() {
        let diagnostics = validate_field_classifications(&config_with(&[], &["open_graph[title]"]), &article_ir());

        let errors = errors_only(&diagnostics);
        assert_eq!(errors.len(), 1, "expected exactly one hard error, got: {diagnostics:?}");
        assert!(errors[0].message.contains("fields_array"), "got: {}", errors[0].message);
    }

    /// `fields_array` gets the same treatment: declaring a plain `String` indexable emits `[0]`
    /// against a scalar.
    #[test]
    fn array_entry_naming_a_non_collection_ir_field_is_an_error() {
        let diagnostics = validate_field_classifications(&config_with(&[], &["title"]), &article_ir());

        let errors = errors_only(&diagnostics);
        assert_eq!(errors.len(), 1, "expected exactly one hard error, got: {diagnostics:?}");
        assert!(errors[0].message.contains("fields_array"), "got: {}", errors[0].message);
        assert!(errors[0].message.contains("String"), "got: {}", errors[0].message);
    }

    #[test]
    fn array_entry_naming_a_vec_ir_field_produces_no_error() {
        let diagnostics = validate_field_classifications(&config_with(&[], &["tags"]), &article_ir());

        assert!(
            errors_only(&diagnostics).is_empty(),
            "a correct fields_array entry must not be rejected: {diagnostics:?}"
        );
    }

    /// The `IrAbsent` arm. Every IR-less caller (unit tests, snippet entry points that generate
    /// from empty IR slices) must keep working — nothing was consulted, so nothing is claimed.
    #[test]
    fn an_absent_ir_produces_no_diagnostic_at_all() {
        let diagnostics = validate_field_classifications(&config_with(&["tags"], &["title"]), &[]);

        assert!(
            diagnostics.is_empty(),
            "an absent IR licenses no claim in either direction: {diagnostics:?}"
        );
    }

    /// The `Unresolvable` arm. A name the IR has never heard of is unverified, not wrong —
    /// virtual namespace prefixes and synthetic/streaming paths legitimately look like this — so
    /// it warns and generation still proceeds.
    #[test]
    fn an_ir_unknown_leaf_warns_rather_than_failing_generation() {
        let diagnostics =
            validate_field_classifications(&config_with(&["interaction.chunk_count"], &[]), &article_ir());

        assert!(
            errors_only(&diagnostics).is_empty(),
            "an unverifiable entry must not fail generation: {diagnostics:?}"
        );
        assert_eq!(diagnostics.len(), 1, "expected one warning, got: {diagnostics:?}");
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(
            diagnostics[0].message.contains("unverified") && diagnostics[0].message.contains("`chunk_count`"),
            "the warning must name the leaf it could not resolve: {}",
            diagnostics[0].message
        );
    }

    /// Index and wildcard suffixes are part of the entry spelling, not part of the field name.
    #[test]
    fn indexed_and_wildcard_entry_spellings_resolve_to_the_same_leaf_field() {
        let indexed = validate_field_classifications(&config_with(&["article[0].tags"], &[]), &article_ir());
        let wildcard = validate_field_classifications(&config_with(&["article[].tags"], &[]), &article_ir());

        assert_eq!(errors_only(&indexed).len(), 1, "got: {indexed:?}");
        assert_eq!(errors_only(&wildcard).len(), 1, "got: {wildcard:?}");
    }

    /// Per-call override tables are checked too, and the diagnostic names the call's own table
    /// rather than the global one — the operator has to be pointed at the line they must edit.
    #[test]
    fn per_call_override_tables_are_checked_and_named_in_the_diagnostic() {
        let mut config = E2eConfig::default();
        config.calls.insert(
            "summarize".to_string(),
            CallConfig {
                fields_optional: HashSet::from(["tags".to_string()]),
                ..CallConfig::default()
            },
        );

        let diagnostics = validate_field_classifications(&config, &article_ir());

        let errors = errors_only(&diagnostics);
        assert_eq!(errors.len(), 1, "got: {diagnostics:?}");
        assert!(
            errors[0].message.contains("[e2e.calls.summarize].fields_optional"),
            "got: {}",
            errors[0].message
        );
    }

    /// The documented under-report trade: a field name that agrees with the entry on ANY IR type
    /// clears it, because a bare leaf name cannot be pinned to one result type from here. This
    /// pins the trade so a later "tighten it up" change has to confront it deliberately.
    #[test]
    fn a_same_named_field_that_agrees_on_another_type_clears_the_entry() {
        let mut ir = article_ir();
        ir.push(TypeDef {
            name: "DraftMetadata".to_string(),
            fields: vec![field("tags", TypeRef::Vec(Box::new(TypeRef::String)), true)],
            ..TypeDef::default()
        });

        let diagnostics = validate_field_classifications(&config_with(&["tags"], &[]), &ir);

        assert!(
            errors_only(&diagnostics).is_empty(),
            "agrees-on-any-type must win, or a shared field name produces a false failure: {diagnostics:?}"
        );
    }
}
