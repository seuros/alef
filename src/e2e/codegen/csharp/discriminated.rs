//! C# discriminated-union assertion helpers.

use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
use crate::e2e::escape::escape_csharp;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};

use super::json_to_csharp;

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

pub(super) fn try_render_generic_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    result_var: &str,
    field: &str,
    assert_enum_fields: &std::collections::HashMap<String, String>,
) -> bool {
    let Some((prefix, union_type, variant, suffix)) = field_resolver.ir_tagged_union_split(field) else {
        return false;
    };
    if field_resolver.union_variant_payload(&union_type, &variant).is_none() {
        render_unsupported_assertion(out, assertion);
        return true;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    field.hash(&mut hasher);
    let variant_var = format!("variant_{:08x}", hasher.finish() as u32);
    let container = field_resolver.accessor(&prefix, "csharp", result_var);
    let field_is_collection = field_resolver.union_variant_field_is_collection(&prefix, &variant, &suffix);
    let _ = writeln!(out, "        if ({container} is {union_type}.{variant} {variant_var})");
    let _ = writeln!(out, "        {{");
    render_discriminated_union_assertion(
        out,
        assertion,
        &variant_var,
        &suffix,
        field_is_collection,
        false,
        assert_enum_fields,
    );
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        else");
    let _ = writeln!(out, "        {{");
    let _ = writeln!(out, "            Assert.Fail(\"Expected {variant} variant\");");
    let _ = writeln!(out, "        }}");
    true
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
            variants: vec![
                EnumVariant {
                    name: "Web".to_string(),
                    fields: vec![field("payload", TypeRef::Named("WebPayload".to_string()))],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Empty".to_string(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Pair".to_string(),
                    fields: vec![field("left", TypeRef::String), field("right", TypeRef::String)],
                    ..EnumVariant::default()
                },
            ],
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

    fn xberg_html_resolver() -> FieldResolver {
        let types = vec![
            TypeDef {
                name: "ExtractionResult".to_string(),
                fields: vec![field(
                    "results",
                    TypeRef::Vec(Box::new(TypeRef::Named("ExtractedDocument".to_string()))),
                )],
                ..TypeDef::default()
            },
            TypeDef {
                name: "ExtractedDocument".to_string(),
                fields: vec![field("metadata", TypeRef::Named("Metadata".to_string()))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Metadata".to_string(),
                fields: vec![field("format", TypeRef::Named("FormatMetadata".to_string()))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "HtmlMetadata".to_string(),
                fields: vec![field(
                    "headers",
                    TypeRef::Vec(Box::new(TypeRef::Named("HeaderMetadata".to_string()))),
                )],
                ..TypeDef::default()
            },
            TypeDef {
                name: "HeaderMetadata".to_string(),
                ..TypeDef::default()
            },
        ];
        let enums = vec![EnumDef {
            name: "FormatMetadata".to_string(),
            variants: vec![EnumVariant {
                name: "Html".to_string(),
                fields: vec![field("value", TypeRef::Named("HtmlMetadata".to_string()))],
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
            Some("ExtractionResult".to_string()),
        )
        .with_ir_collection_map(
            FieldResolver::ir_collection_fields(&types),
            Some("ExtractionResult".to_string()),
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

    #[test]
    fn count_min_uses_ir_tagged_union_path_without_named_parser() {
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("details.web.entries".to_string()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        assert!(try_render_generic_union_assertion(
            &mut out,
            &assertion,
            &resolver(),
            "result",
            "details.web.entries",
            &std::collections::HashMap::new(),
        ));
        assert!(out.contains("DetailUnion.Web"));
        assert!(out.contains("Value.Entries?.Count ?? 0"));
        assert!(!out.contains("skipped:"));
    }

    #[test]
    fn xberg_html_headers_count_min_uses_the_ir_union_owner() {
        let field = "results[0].metadata.format.html.headers";
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        assert!(try_render_generic_union_assertion(
            &mut out,
            &assertion,
            &xberg_html_resolver(),
            "result",
            field,
            &std::collections::HashMap::new(),
        ));
        assert!(out.contains("result.Results[0].Metadata.Format is FormatMetadata.Html"));
        assert!(out.contains("Value.Headers?.Count ?? 0) >= 2"));
        assert!(!out.contains("skipped:"));
    }

    #[test]
    fn unsupported_union_shapes_stay_registered_skips() {
        for field in [
            "details.unknown.entries",
            "details.empty.entries",
            "details.pair.entries",
            "details.web.entries.value",
            "details.web.entries[0]",
        ] {
            let assertion = Assertion {
                assertion_type: "count_min".to_string(),
                field: Some(field.to_string()),
                value: Some(serde_json::json!(2)),
                ..Assertion::default()
            };
            let mut out = String::new();

            assert!(try_render_generic_union_assertion(
                &mut out,
                &assertion,
                &resolver(),
                "result",
                field,
                &std::collections::HashMap::new(),
            ));
            assert!(out.contains("skipped: assertion type 'count_min' not yet supported"));
        }
    }
}
