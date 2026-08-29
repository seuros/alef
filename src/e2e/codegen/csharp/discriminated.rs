//! C# discriminated-union assertion helpers.

use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
use crate::e2e::escape::escape_csharp;
use crate::e2e::fixture::Assertion;
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;

use super::json_to_csharp;

/// Detect if a field path accesses a discriminated union variant in C#.
/// Pattern: `metadata.format.<variant_name>.<field_name>`
/// Returns: Some((accessor, variant_name, inner_field)) if matched, otherwise None
pub(super) fn parse_discriminated_union_access(field: &str) -> Option<(String, String, String)> {
    // Strip a leading list-index prefix (e.g. "results[0].") so both single-result
    // (`metadata.format.excel.sheet_count`) and list-result
    // (`results[0].metadata.format.excel.sheet_count`) field paths are recognized.
    let field = field.split_once("].").map(|(_, rest)| rest).unwrap_or(field);
    let parts: Vec<&str> = field.split('.').collect();
    if parts.len() >= 3 && parts.len() <= 4 {
        // Check if this is metadata.format.{variant}.{field} pattern
        if parts[0] == "metadata" && parts[1] == "format" {
            let variant_name = parts[2];
            // Known C# discriminated union variants (lowercase in fixture paths)
            let known_variants = [
                "pdf",
                "docx",
                "excel",
                "email",
                "pptx",
                "archive",
                "image",
                "xml",
                "text",
                "html",
                "ocr",
                "csv",
                "bibtex",
                "citation",
                "fiction_book",
                "dbf",
                "jats",
                "epub",
                "pst",
                "code",
            ];
            if known_variants.contains(&variant_name) {
                let variant_pascal = variant_name.to_upper_camel_case();
                if parts.len() == 4 {
                    let inner_field = parts[3];
                    return Some((
                        format!("result.Metadata.Format! as FormatMetadata.{}", variant_pascal),
                        variant_pascal,
                        inner_field.to_string(),
                    ));
                } else if parts.len() == 3 {
                    // Just accessing the variant itself (no inner field)
                    return Some((
                        format!("result.Metadata.Format! as FormatMetadata.{}", variant_pascal),
                        variant_pascal,
                        String::new(),
                    ));
                }
            }
        }
    }
    None
}

/// Render an assertion against a discriminated union variant's inner field.
/// `variant_var` is the unwrapped union variant (e.g., `variant` from pattern match).
/// `inner_field` is the field to access on the variant's Value (e.g., `sheet_count`).
pub(super) fn render_discriminated_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    variant_var: &str,
    inner_field: &str,
    field_is_collection: bool,
    _result_is_vec: bool,
    assert_enum_fields: &std::collections::HashMap<String, String>,
) {
    if inner_field.is_empty() {
        return; // No field to assert on
    }

    let field_pascal = inner_field.to_upper_camel_case();
    let mut field_expr = format!("{variant_var}.Value.{field_pascal}");

    // Wrap enum fields with display helper
    if assert_enum_fields.contains_key(&field_pascal) {
        let type_name = assert_enum_fields.get(&field_pascal).unwrap();
        field_expr = format!("{type_name}Display.ToDisplayString({field_expr})");
    }

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let cs_val = json_to_csharp(expected);
                if expected.is_string() {
                    let _ = writeln!(out, "            Assert.Equal({cs_val}, {field_expr}!.Trim());");
                } else if expected.as_bool() == Some(true) {
                    let _ = writeln!(out, "            Assert.True({field_expr});");
                } else if expected.as_bool() == Some(false) {
                    let _ = writeln!(out, "            Assert.False({field_expr});");
                } else if expected.is_number() && !expected.as_f64().is_some_and(|f| f.fract() != 0.0) {
                    let _ = writeln!(out, "            Assert.True({field_expr} == {cs_val});");
                } else {
                    let _ = writeln!(out, "            Assert.Equal({cs_val}, {field_expr});");
                }
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let cs_val = json_to_csharp(val);
                let _ = writeln!(
                    out,
                    "            Assert.True({field_expr} >= {cs_val}, \"expected >= {cs_val}\");"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let field_as_str = format!("JsonSerializer.Serialize({field_expr})");
                for val in values {
                    let lower_val = val.as_str().map(|s| s.to_lowercase());
                    let cs_val = lower_val
                        .as_deref()
                        .map(|s| format!("\"{}\"", escape_csharp(s)))
                        .unwrap_or_else(|| json_to_csharp(val));
                    let _ = writeln!(out, "            Assert.Contains({cs_val}, {field_as_str}.ToLower());");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let field_as_str = format!("JsonSerializer.Serialize({field_expr})");
                let lower_expected = expected.as_str().map(|s| s.to_lowercase());
                let cs_val = lower_expected
                    .as_deref()
                    .map(|s| format!("\"{}\"", escape_csharp(s)))
                    .unwrap_or_else(|| json_to_csharp(expected));
                let _ = writeln!(out, "            Assert.Contains({cs_val}, {field_as_str}.ToLower());");
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "            Assert.NotEmpty({field_expr});");
        }
        "is_empty" => {
            let _ = writeln!(out, "            Assert.Empty({field_expr});");
        }
        "count_min" if field_is_collection => {
            if let Some(count) = assertion.value.as_ref().and_then(serde_json::Value::as_u64) {
                let _ = writeln!(
                    out,
                    "            Assert.True(({field_expr}?.Count ?? 0) >= {count}, \"expected count >= {count}\");"
                );
            } else {
                render_unsupported_assertion(out, assertion);
            }
        }
        _ => {
            render_unsupported_assertion(out, assertion);
        }
    }
}

fn render_unsupported_assertion(out: &mut String, assertion: &Assertion) {
    let reason = AssertionTypeSkip::DiscriminatedUnionAssertionTypeNotSupported.message(&assertion.assertion_type);
    let _ = writeln!(out, "            // skipped: {reason}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
    use crate::e2e::field_access::FieldResolver;

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn resolver() -> FieldResolver {
        let types = vec![
            TypeDef {
                name: "Envelope".to_string(),
                fields: vec![field("details", TypeRef::Named("DetailUnion".to_string()))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "WebPayload".to_string(),
                fields: vec![
                    field("entries", TypeRef::Vec(Box::new(TypeRef::String))),
                    field("label", TypeRef::String),
                ],
                ..TypeDef::default()
            },
        ];
        let enums = vec![EnumDef {
            name: "DetailUnion".to_string(),
            variants: vec![EnumVariant {
                name: "Web".to_string(),
                fields: vec![field("payload", TypeRef::Named("WebPayload".to_string()))],
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        }];
        FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        )
        .with_ir_enum_map(
            FieldResolver::ir_enum_fields(&types, &enums),
            Some("Envelope".to_string()),
        )
        .with_ir_collection_map(
            FieldResolver::ir_collection_fields(&types),
            Some("Envelope".to_string()),
        )
    }

    #[test]
    fn count_min_on_a_union_payload_collection_renders_a_count_assertion() {
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("details.web.entries".to_string()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        render_discriminated_union_assertion(
            &mut out,
            &assertion,
            "webVariant",
            "entries",
            resolver().union_variant_field_is_collection("details", "web", "entries"),
            false,
            &std::collections::HashMap::new(),
        );

        assert_eq!(
            out,
            "            Assert.True((webVariant.Value.Entries?.Count ?? 0) >= 2, \"expected count >= 2\");\n"
        );
    }

    #[test]
    fn count_min_on_a_union_payload_scalar_stays_an_explicit_skip() {
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        render_discriminated_union_assertion(
            &mut out,
            &assertion,
            "webVariant",
            "label",
            resolver().union_variant_field_is_collection("details", "web", "label"),
            false,
            &std::collections::HashMap::new(),
        );

        assert_eq!(
            out,
            "            // skipped: assertion type 'count_min' not yet supported for discriminated union fields\n"
        );
    }
}
