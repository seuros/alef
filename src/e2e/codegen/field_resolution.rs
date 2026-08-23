//! Resolving fixture-input field paths to JSON values, and picking the call config
//! whose required args those paths can satisfy.

use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

/// Resolve a JSON field from a fixture input by path.
///
/// Field paths in call config are "input.path", "input.config", etc.
/// Since we already receive `fixture.input`, strip the leading "input." prefix.
/// When `field_path` is exactly `"input"`, the whole input object is returned.
pub(crate) fn resolve_field<'a>(input: &'a serde_json::Value, field_path: &str) -> &'a serde_json::Value {
    // "input" with no subpath means "the entire input object".
    if field_path == "input" {
        // New fixture schema wraps the call input DTO under `extract_input`
        // alongside a sibling `mock_responses` array (so a single fixture can both
        // declare the input and configure the mock server). Unwrap it so the arg
        // resolves to the actual DTO. Flat fixtures — where `input` *is* the DTO —
        // have no `extract_input` key and are returned unchanged.
        if let Some(inner) = input.get("extract_input") {
            return inner;
        }
        return input;
    }
    let path = field_path.strip_prefix("input.").unwrap_or(field_path);
    let mut current = input;
    for part in path.split('.') {
        current = current.get(part).unwrap_or(&serde_json::Value::Null);
    }
    current
}

/// Select the best-matching call for a fixture based on input field availability.
///
/// When the initially resolved call config has required args whose fields are
/// missing from fixture input, search the named calls for one whose args better
/// match the available input fields. This allows generic call selection even when
/// select_when conditions are too specific (e.g., category-restricted).
///
/// Returns the passed-in `initial_call` if no better match is found.
pub(crate) fn select_best_matching_call<'a>(
    initial_call: &'a crate::e2e::config::CallConfig,
    e2e_config: &'a E2eConfig,
    fixture: &Fixture,
) -> &'a crate::e2e::config::CallConfig {
    // Check if initial call's required args can be satisfied from fixture input
    let initial_satisfied = initial_call.args.iter().all(|arg| {
        if arg.optional {
            return true;
        }
        // For mock_url_list args, use resolve_urls_field which handles aliasing
        // (e.g., batch_urls ↔ urls). For other arg types, use regular resolve_field.
        let field_value = if arg.arg_type == "mock_url_list" {
            resolve_urls_field(&fixture.input, &arg.field)
        } else {
            resolve_field(&fixture.input, &arg.field)
        };
        field_value.as_null().is_none()
    });

    if initial_satisfied {
        return initial_call;
    }

    // Initial call has unsatisfied required args. Search named calls for a better match.
    for alt_call in e2e_config.calls.values() {
        let all_satisfied = alt_call.args.iter().all(|arg| {
            if arg.optional {
                return true;
            }
            // For mock_url_list args, use resolve_urls_field which handles aliasing
            // (e.g., batch_urls ↔ urls). For other arg types, use regular resolve_field.
            let field_value = if arg.arg_type == "mock_url_list" {
                resolve_urls_field(&fixture.input, &arg.field)
            } else {
                resolve_field(&fixture.input, &arg.field)
            };
            field_value.as_null().is_none()
        });

        if all_satisfied {
            return alt_call;
        }
    }

    // No better call found; use initial
    initial_call
}

/// Resolve a list-type argument field, trying both the declared field name and
/// common aliases (batch_urls, urls; urls_list, url_list).
///
/// Used by codegen for `mock_url_list` arguments when the fixture may use
/// alternative field names (e.g. some fixtures use `urls` while call config
/// declares `batch_urls`).
pub(crate) fn resolve_urls_field<'a>(input: &'a serde_json::Value, field_path: &str) -> &'a serde_json::Value {
    // Try the declared field first
    let result = resolve_field(input, field_path);
    if !result.is_null() {
        return result;
    }

    // Try common aliases if the primary field is not found
    let aliases = [
        ("batch_urls", "urls"),
        ("urls", "batch_urls"),
        ("batch_urls", "url_list"),
        ("batch_urls", "urls_list"),
        ("urls", "url_list"),
        ("urls", "urls_list"),
    ];

    for (orig, alias) in &aliases {
        if field_path.ends_with(orig) {
            let aliased_path = field_path.replace(orig, alias);
            let result = resolve_field(input, &aliased_path);
            if !result.is_null() {
                return result;
            }
        }
    }

    // Nothing found; return null
    &serde_json::Value::Null
}
