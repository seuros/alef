//! Streaming virtual-field assertions -- fields that resolve against the drained `chunks`
//! collected-list variable rather than the raw result, for `is_streaming` fixtures.
//!
//! Split out of `assertions.rs` (file-modularization cap): see that file's module doc.

use crate::e2e::codegen::assertion_type_skip::{streaming_assertion_type_skip_line, streaming_assertion_value_skip_line};
use crate::e2e::fixture::Assertion;

pub(super) fn try_streaming_virtual_field_assertion(
    out: &mut String,
    assertion: &Assertion,
    is_streaming: bool,
    streaming_item_type: Option<&str>,
) -> bool {
    // Streaming virtual fields: intercept before is_valid_for_result so they are
    // never skipped.  These fields resolve against the `chunks` collected-list variable.
    // Gate on `is_streaming` so non-streaming fixtures (e.g. consumers whose real
    // result struct has a literal `chunks` field) don't divert into the virtual
    // accessor path — they should fall through to the normal field resolver.
    if let Some(f) = &assertion.field
        && is_streaming
        && !f.is_empty()
        && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
    {
        if let Some(expr) =
            crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor_with_streaming_context(
                f,
                "java",
                "chunks",
                None,
                streaming_item_type,
            )
        {
            let line = match assertion.assertion_type.as_str() {
                "count_min" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        assertTrue({expr}.size() >= {n}, \"expected >= {n} chunks\");\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "count_equals" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        assertEquals({n}, {expr}.size());\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "equals" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let literal = super::values::java_string_literal(s);
                        format!("        assertEquals({literal}, {expr});\n")
                    } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        assertEquals({n}, {expr});\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "not_empty" => format!("        assertFalse({expr}.isEmpty(), \"expected non-empty\");\n"),
                "is_empty" => format!("        assertTrue({expr}.isEmpty(), \"expected empty\");\n"),
                "is_true" => format!("        assertTrue({expr}, \"expected true\");\n"),
                "is_false" => format!("        assertFalse({expr}, \"expected false\");\n"),
                "greater_than" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        assertTrue({expr} > {n}, \"expected > {n}\");\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        assertTrue({expr} >= {n}, \"expected >= {n}\");\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "contains" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = crate::e2e::escape::escape_java(s);
                        format!(
                            "        assertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\");\n"
                        )
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                _ => format!(
                    "{}\n",
                    streaming_assertion_type_skip_line("        ", "//", f, &assertion.assertion_type)
                ),
            };
            out.push_str(&line);
        } else {
            // ~keep The accessor returns `None` for reachable inputs (a `stream.has_*_event`
            // predicate whose item type this call never resolved, for one), and this branch used
            // to be absent: the assertion vanished with no line for
            // `fail_on_unavailable_field_markers` to see, so a clean strict-gate run was
            // indistinguishable from one that dropped it. alef's streaming adapter owns the gap,
            // so it is counted, never fatal.
            out.push_str(&format!(
                "        // skipped: {}\n",
                crate::e2e::codegen::field_skip::FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
            ));
        }
        return true;
    }
    false
}
