//! Bracket-wildcard (`field[].sub`) assertion rendering for the Java e2e backend.

use crate::e2e::codegen::field_skip::nested_wildcard_skip_line;
use crate::e2e::escape::escape_java;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// Lambda-parameter / message suffix keyed to the assertion.
///
/// A Java lambda parameter may not shadow an enclosing local, and generated test methods
/// bind locals named after fixture fields. Hashing the assertion's discriminating fields
/// keeps the parameter name unique and stable across regenerations. ~keep
fn wildcard_lambda_param(assertion: &Assertion) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    assertion.assertion_type.hash(&mut hasher);
    assertion.field.hash(&mut hasher);
    assertion
        .value
        .as_ref()
        .map(std::string::ToString::to_string)
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("e{:x}", hasher.finish() & 0xffff_ffff)
}

/// Emit `assertTrue(<array>.stream().anyMatch(e -> …))` for a bracket-wildcard path.
pub(super) fn render_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    field: &str,
    array_part: &str,
    elem_part: &str,
) {
    // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a second
    // wildcard in `elem_part` that the element accessor below would lower to index 0. ~keep
    if let Some(line) = nested_wildcard_skip_line("        ", "//", field, elem_part) {
        out.push_str(&line);
        out.push('\n');
        return;
    }
    let array_accessor = if array_part.is_empty() {
        result_var.to_string()
    } else {
        let accessor = field_resolver.accessor(array_part, "java", result_var);
        // Nullable list getters come back as `@Nullable List<T>`; `.stream()` on null would
        // NPE, so fall back to an empty list exactly as the count assertions do. ~keep
        if field_resolver.is_optional(field_resolver.resolve(array_part)) {
            format!("java.util.Optional.ofNullable({accessor}).orElse(java.util.List.of())")
        } else {
            accessor
        }
    };
    let param = wildcard_lambda_param(assertion);
    // Passing the lambda parameter as the result var is what resolves a nested element
    // sub-path against the loop element instead of the whole result. ~keep
    let elem_accessor = field_resolver.accessor(elem_part, "java", &param);

    let any_match = |value: &serde_json::Value| -> Option<(String, String)> {
        let serde_json::Value::String(s) = value else {
            return None;
        };
        let escaped = escape_java(s);
        Some((
            format!(
                "{array_accessor}.stream().anyMatch({param} -> String.valueOf({elem_accessor}).contains(\"{escaped}\"))"
            ),
            escaped,
        ))
    };

    match assertion.assertion_type.as_str() {
        "contains" | "not_contains" if assertion.value.is_some() => {
            let value = assertion.value.as_ref().expect("guarded by the match arm");
            let Some((expr, escaped)) = any_match(value) else {
                out.push_str(&format!(
                    "        // skipped: non-string value for '{field}' traversal assertion\n"
                ));
                return;
            };
            if assertion.assertion_type == "contains" {
                out.push_str(&format!(
                    "        assertTrue({expr}, \"expected some element of '{field}' to contain: {escaped}\");\n"
                ));
            } else {
                out.push_str(&format!(
                    "        assertFalse({expr}, \"expected no element of '{field}' to contain: {escaped}\");\n"
                ));
            }
        }
        "contains" | "contains_all" | "not_contains" => {
            let Some(values) = &assertion.values else {
                out.push_str(&format!(
                    "        // skipped: '{field}' traversal assertion has no values\n"
                ));
                return;
            };
            let negated = assertion.assertion_type == "not_contains";
            for value in values {
                let Some((expr, escaped)) = any_match(value) else {
                    continue;
                };
                if negated {
                    out.push_str(&format!(
                        "        assertFalse({expr}, \"expected no element of '{field}' to contain: {escaped}\");\n"
                    ));
                } else {
                    out.push_str(&format!(
                        "        assertTrue({expr}, \"expected some element of '{field}' to contain: {escaped}\");\n"
                    ));
                }
            }
        }
        "not_empty" => {
            out.push_str(&format!(
                "        assertTrue({array_accessor}.stream().anyMatch({param} -> \
                 !String.valueOf({elem_accessor}).isEmpty()), \"expected some element of '{field}' to be \
                 non-empty\");\n"
            ));
        }
        other => {
            out.push_str(&format!(
                "        // skipped: unsupported traversal assertion '{other}' on '{field}'\n"
            ));
        }
    }
}
