//! R visitor callback rendering.

use crate::e2e::escape::{escape_r, r_template_to_paste0};
use crate::e2e::fixture::{CallbackAction, TemplateReturnForm};

/// Build an R visitor list and add setup line.
pub(super) fn build_r_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::e2e::fixture::VisitorSpec) {
    use std::fmt::Write as FmtWrite;
    // Collect each callback as a separate string, then join with ",\n" to avoid
    // trailing commas — R's list() does not accept a trailing comma.
    let methods: Vec<String> = visitor_spec
        .callbacks
        .iter()
        .map(|(method_name, action)| {
            let mut buf = String::new();
            emit_r_visitor_method(&mut buf, method_name, action);
            // strip the trailing ",\n" added by emit_r_visitor_method
            buf.trim_end_matches(['\n', ',']).to_string()
        })
        .collect();
    let mut visitor_obj = String::new();
    let _ = writeln!(visitor_obj, "list(");
    let _ = write!(visitor_obj, "{}", methods.join(",\n"));
    let _ = writeln!(visitor_obj);
    let _ = writeln!(visitor_obj, "  )");

    setup_lines.push(format!("visitor <- {visitor_obj}"));
}

/// Emit an R visitor method for a callback action.
fn emit_r_visitor_method(out: &mut String, method_name: &str, action: &CallbackAction) {
    use std::fmt::Write as FmtWrite;

    // R uses visit_ prefix (matches binding signature)
    let params = match method_name {
        "visit_link" => "ctx, href, text, title",
        "visit_image" => "ctx, src, alt, title",
        "visit_heading" => "ctx, level, text, id",
        "visit_code_block" => "ctx, lang, code",
        "visit_code_inline"
        | "visit_strong"
        | "visit_emphasis"
        | "visit_strikethrough"
        | "visit_underline"
        | "visit_subscript"
        | "visit_superscript"
        | "visit_mark"
        | "visit_button"
        | "visit_summary"
        | "visit_figcaption"
        | "visit_definition_term"
        | "visit_definition_description" => "ctx, text",
        "visit_text" => "ctx, text",
        "visit_list_item" => "ctx, ordered, marker, text",
        "visit_blockquote" => "ctx, content, depth",
        "visit_table_row" => "ctx, cells, is_header",
        "visit_custom_element" => "ctx, tag_name, html",
        "visit_form" => "ctx, action_url, method",
        "visit_input" => "ctx, input_type, name, value",
        "visit_audio" | "visit_video" | "visit_iframe" => "ctx, src",
        "visit_details" => "ctx, open",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => "ctx, output",
        "visit_list_start" => "ctx, ordered",
        "visit_list_end" => "ctx, ordered, output",
        _ => "ctx",
    };

    let _ = writeln!(out, "    {method_name} = function({params}) {{");
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "      \"{}\"", action.wire_name());
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "      \"{}\"", action.wire_name());
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "      \"{}\"", action.wire_name());
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_r(output);
            let _ = writeln!(out, "      list({} = \"{escaped}\")", action.wire_name());
        }
        CallbackAction::CustomTemplate { template, return_form } => {
            let r_expr = r_template_to_paste0(template);
            match return_form {
                TemplateReturnForm::BareString => {
                    let _ = writeln!(out, "      {r_expr}");
                }
                TemplateReturnForm::Dict => {
                    let _ = writeln!(out, "      list({} = {r_expr})", action.wire_name());
                }
            }
        }
    }
    let _ = writeln!(out, "    }},");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::visitor_result::required_visitor_result_metadata;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};

    #[test]
    fn visitor_actions_use_canonical_wire_shape() {
        let mut continued = String::new();
        emit_r_visitor_method(&mut continued, "visit_text", &CallbackAction::Continue);
        assert!(continued.contains(r#""continue""#));

        let mut custom = String::new();
        emit_r_visitor_method(
            &mut custom,
            "visit_text",
            &CallbackAction::Custom {
                output: "replacement".to_string(),
            },
        );
        assert!(custom.contains(r#"list(custom = "replacement")"#));
    }

    /// `VisitResult` IR shape mirroring the consumer convention for visitor trait bridges:
    /// unit `Skip`/`Continue`/`PreserveHtml` variants plus a single-`String`-field `Custom`
    /// payload variant, with `#[serde(rename_all = "snake_case")]` on the enum — the
    /// convention the extendr backend relies on to build lowercase wire names.
    fn visit_result_enum() -> EnumDef {
        EnumDef {
            name: "VisitResult".to_string(),
            rust_path: "sample_markdown_rs::visitor::VisitResult".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Continue".to_string(),
                    is_default: true,
                    ..Default::default()
                },
                EnumVariant {
                    name: "Skip".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "PreserveHtml".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Custom".to_string(),
                    fields: vec![FieldDef {
                        name: "0".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            has_serde: true,
            serde_rename_all: Some("snake_case".to_string()),
            ..Default::default()
        }
    }

    fn visitor_bridge_cfg() -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: "HtmlVisitor".to_string(),
            result_type: Some("VisitResult".to_string()),
            ..Default::default()
        }
    }

    /// Pins the e2e generator's emitted R literal to the flat key that
    /// `visitor_result::required_visitor_result_metadata` computes — the exact function the
    /// extendr backend's `gen_visitor_bridge` calls to build its `val.dollar(...)` lookup.
    /// Deriving the expected key from that shared function, rather than hardcoding it here,
    /// means the e2e generator and the real binding can't silently re-diverge: a nested
    /// `list(type = ..., output = ...)` envelope satisfies neither `val.dollar("custom")` nor
    /// any other flat lookup, so the previous shape looked self-consistent while never being
    /// deliverable to the extendr backend.
    #[test]
    fn r_visitor_custom_payload_key_matches_extendr_wire_name() {
        let api = ApiSurface {
            enums: vec![visit_result_enum()],
            ..Default::default()
        };
        let metadata = required_visitor_result_metadata(&api, &visitor_bridge_cfg())
            .expect("VisitResult metadata should resolve for a well-formed visitor result enum");
        let custom_wire = metadata
            .string_payload_variants
            .first()
            .expect("expected one string-payload variant (Custom)")
            .wire_name
            .clone();

        let mut out = String::new();
        emit_r_visitor_method(
            &mut out,
            "visit_text",
            &CallbackAction::Custom {
                output: "replacement".to_string(),
            },
        );

        let expected_key = format!("list({custom_wire} = ");
        assert!(
            out.contains(&expected_key),
            "expected extendr-matching key {expected_key:?} in rendered R:\n{out}"
        );
        assert!(
            !out.contains("type ="),
            "R visitor payload must not emit the dead nested `type`/`output` envelope:\n{out}"
        );
    }
}
