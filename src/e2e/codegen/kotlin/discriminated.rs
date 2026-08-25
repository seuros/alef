//! Kotlin discriminated-union (sealed class) assertion helpers.
//!
//! Mirrors `csharp/discriminated.rs` but emits Kotlin `is` pattern matching
//! against `FormatMetadata` subclasses.  Sealed class subclasses expose the
//! payload as a single `metadata` property (see `FormatMetadata.Excel(val metadata: ExcelMetadata)`),
//! so the inner field is accessed as `variant.metadata.<innerCamelCase>`.
//!
//! [`parse_discriminated_union_access`] only recognizes that one hand-maintained fixture
//! shape. `kotlin/assertions.rs::render_assertion` also drives
//! [`render_discriminated_union_assertion`] from a second, IR-general detector
//! (`FieldResolver::tagged_union_split` + `FieldResolver::union_variant_payload`) for any
//! other tagged union, computing the payload property name from the IR itself via
//! `kotlin_field_name_with_type` instead of assuming `"metadata"`.

use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::escape::escape_kotlin;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::values::json_to_kotlin;

/// Detect if a field path navigates a discriminated union variant.
/// Pattern: `metadata.format.<variant_name>(.<inner_field>)?`
/// Returns: Some((variant_pascal, inner_field_snake)) if matched.
pub(super) fn parse_discriminated_union_access(field: &str) -> Option<(String, String)> {
    // Strip a leading list-index prefix (e.g. "results[0].") so both single-result
    // (`metadata.format.excel.sheet_count`) and list-result
    // (`results[0].metadata.format.excel.sheet_count`) field paths are recognized.
    let field = field.split_once("].").map(|(_, rest)| rest).unwrap_or(field);
    let parts: Vec<&str> = field.split('.').collect();
    if !(parts.len() == 3 || parts.len() == 4) {
        return None;
    }
    if parts[0] != "metadata" || parts[1] != "format" {
        return None;
    }
    let variant_name = parts[2];
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
    if !known_variants.contains(&variant_name) {
        return None;
    }
    let variant_pascal = variant_name.to_upper_camel_case();
    let inner_field = if parts.len() == 4 {
        parts[3].to_string()
    } else {
        String::new()
    };
    Some((variant_pascal, inner_field))
}

/// Render an assertion against a sealed-class variant's inner field.
///
/// `variant_var` is the bound name from `is FormatMetadata.<Variant> -> { … }`
/// (e.g. `format_excel`). `payload_field` is the sealed-class subclass's own
/// property that carries the variant's payload (e.g. `"metadata"`) — every hand-maintained
/// caller of this function passes the literal `"metadata"` because every variant in that
/// consumer's fixtures happens to wrap a type named `<Variant>Metadata`, but the property
/// name is a per-variant fact (see `kotlin_field_name_with_type` and its callers), not a
/// universal constant, so it is a parameter rather than baked in here.
pub(super) fn render_discriminated_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    variant_var: &str,
    payload_field: &str,
    inner_field: &str,
) {
    if inner_field.is_empty() {
        return;
    }

    let field_camel = inner_field.to_lower_camel_case();
    // The variant payload field (`variant.<payload_field>.<inner>`) is frequently Optional in
    // the alef-generated Kotlin types (e.g. `ExcelMetadata.sheetCount: Int?`).  The fixture
    // assertion only fires when the variant matched, so a null inner field would itself be
    // a test failure — assert non-null with `!!.` before the comparison so kotlinc accepts
    // arithmetic and ordering operators (`>=`, `>`, etc.) on the receiver.
    let field_expr = format!("{variant_var}.{payload_field}.{field_camel}!!");

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let kt_val = json_to_kotlin(expected);
                if expected.is_string() {
                    let _ = writeln!(
                        out,
                        "                assertEquals({kt_val}, {field_expr}.trim(), \"expected: {}\")",
                        escape_kotlin(expected.as_str().unwrap_or(""))
                    );
                } else if expected.as_bool() == Some(true) {
                    let _ = writeln!(
                        out,
                        "                assertTrue({field_expr} == true, \"expected true\")"
                    );
                } else if expected.as_bool() == Some(false) {
                    let _ = writeln!(
                        out,
                        "                assertTrue({field_expr} == false, \"expected false\")"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "                assertEquals({kt_val}, {field_expr}, \"expected: {kt_val}\")"
                    );
                }
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kt_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr} >= {kt_val}, \"expected >= {kt_val}\")"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kt_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr} <= {kt_val}, \"expected <= {kt_val}\")"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let kt_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr} > {kt_val}, \"expected > {kt_val}\")"
                );
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let kt_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr} < {kt_val}, \"expected < {kt_val}\")"
                );
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value
                && let Some(s) = expected.as_str()
            {
                let lower = s.to_lowercase();
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr}.orEmpty().toString().lowercase().contains(\"{}\".lowercase()), \"expected to contain: {}\")",
                    escape_kotlin(&lower),
                    escape_kotlin(s)
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    if let Some(s) = val.as_str() {
                        let lower = s.to_lowercase();
                        let _ = writeln!(
                            out,
                            "                assertTrue({field_expr}.orEmpty().toString().lowercase().contains(\"{}\".lowercase()), \"expected to contain: {}\")",
                            escape_kotlin(&lower),
                            escape_kotlin(s)
                        );
                    }
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "                assertTrue({field_expr}.toString().isNotEmpty(), \"expected non-empty value\")"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "                assertTrue({field_expr}.toString().isEmpty(), \"expected empty value\")"
            );
        }
        _ => {
            let _ = writeln!(
                out,
                "                // skipped: assertion type '{}' not yet supported for discriminated union fields",
                assertion.assertion_type
            );
        }
    }
}

/// The IR-general sealed-class narrowing path: recognizes ANY tagged union
/// [`parse_discriminated_union_access`]'s hand-maintained `metadata.format.<variant>` parser
/// does not, using [`FieldResolver::tagged_union_split`] (the same detector Gleam, Dart and
/// Swift already consult, driven by the consumer's `fields_method_calls` config) to find the
/// boundary and [`FieldResolver::union_variant_payload`] to ask the IR which single field the
/// matched variant wraps. The payload property's real Kotlin name comes from
/// `kotlin_field_name_with_type` — the exact helper the Kotlin binding backend itself uses to
/// name it — rather than assuming `"metadata"`, so this also covers unions whose variants wrap
/// a differently-named or differently-typed payload.
///
/// Returns `true` when it rendered something (either a real narrowing block, or — for a
/// boundary it detected but could not lower, e.g. a multi-field variant, or a union type the
/// IR never anchored — the loud, named [`FieldSkip::UnionTraversalNotImplementedForKotlin`]
/// skip) and `false` when `f` does not cross a tagged-union boundary at all, in which case the
/// caller must keep trying its other field-shape branches. Never falls through to
/// `field_resolver.accessor` for the field paths it recognizes: that renderer walks flat
/// property/method chains and does not know pattern matching exists, so on a sealed class it
/// would emit a chain like `.excel().sheetCount()` that alef's own generated Kotlin does not
/// declare a method for. ~keep
pub(super) fn try_render_generic_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    result_var: &str,
    kotlin_android_style: bool,
    f: &str,
) -> bool {
    let Some((prefix, variant, suffix)) = field_resolver.tagged_union_split(f) else {
        return false;
    };
    let variant_pascal = variant.to_upper_camel_case();
    let lowered = field_resolver.ir_enum_type_name(&prefix).and_then(|union_type| {
        field_resolver
            .union_variant_payload(&union_type, &variant_pascal)
            .map(|(payload_field_name, payload_type)| {
                (union_type, payload_field_name.to_string(), payload_type.to_string())
            })
    });
    let Some((union_type, payload_field_name, payload_type)) = lowered else {
        let _ = writeln!(
            out,
            "        // skipped: {}",
            FieldSkip::UnionTraversalNotImplementedForKotlin.message(f)
        );
        return true;
    };

    let style = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let variant_var = format!("union{variant_pascal}");
    let container = field_resolver.accessor(&prefix, style, result_var);
    let payload_field = crate::backends::kotlin::kotlin_field_name_with_type(
        &payload_field_name,
        0,
        Some(&payload_type),
        &variant_pascal,
        1,
    );
    let _ = writeln!(out, "        when (val {variant_var} = {container}) {{");
    let _ = writeln!(out, "            is {union_type}.{variant_pascal} -> {{");
    render_discriminated_union_assertion(out, assertion, &variant_var, &payload_field, &suffix);
    let _ = writeln!(out, "            }}");
    let _ = writeln!(out, "            else -> {{}}");
    let _ = writeln!(out, "        }}");
    true
}
