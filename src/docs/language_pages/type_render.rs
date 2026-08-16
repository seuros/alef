use crate::codegen::shared::binding_fields;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeDef};
use crate::docs::descriptions::generate_field_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings_to_start_at};
use crate::docs::formatting::{doc_type_with_optional, escape_table_cell, format_field_default};
use crate::docs::naming::{field_name, type_name};
use crate::docs::type_mapping::FFI_HANDLE_TYPE_NAME;
use crate::docs::{clean_doc, template_env};

use super::function_render::push_version_annotation;
use super::streaming::{method_visible_in_lang, render_method};

const TYPE_DOC_FIRST_HEADING_LEVEL: usize = 5;

/// ~keep Every `TypeRef::Named` crosses the C ABI as a scalar `AlefHandle` token, not a
/// pointer to a struct named after the Rust type (see type_mapping.rs's
/// `FFI_HANDLE_TYPE_NAME`). The page heading above still names the *logical* Rust type --
/// renaming every DTO's heading to the shared handle token would make every C type page
/// title collide, which is worse than the status quo. What a C reader cannot infer on
/// their own is that this heading is documentation-only: `tname` does not appear anywhere
/// in the generated header. This note says so explicitly, once, on every C type page.
fn push_ffi_handle_note(out: &mut String, tname: &str, lang: Language, ffi_prefix: &str) {
    if !matches!(lang, Language::Ffi | Language::C) {
        return;
    }
    let handle_type = type_name(FFI_HANDLE_TYPE_NAME, lang, ffi_prefix);
    out.push_str(&format!(
        "**C representation:** `{tname}` is a documentation-only name for this type. \
         The C ABI hands you a scalar `{handle_type}` handle -- the literal string \
         `{tname}` does not appear anywhere in the generated header.\n"
    ));
    out.push('\n');
}

pub(super) fn render_type(
    ty: &TypeDef,
    lang: Language,
    config: &ResolvedCrateConfig,
    api: &ApiSurface,
    ffi_prefix: &str,
) -> String {
    let mut out = String::new();
    let tname = type_name(&ty.name, lang, ffi_prefix);
    // ~keep Every documented type name must be a legal identifier in `lang` -- see
    // formatting.rs's `assert_valid_identifier`. Rarely fires (PascalCase type names
    // don't usually collide with lowercase keywords), but a handful of languages have
    // capitalized reserved words too (Rust's `Self`, Swift's `Any`/`Self`).
    crate::docs::formatting::assert_valid_identifier(&tname, lang, "a type heading");

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => tname },
    ));

    push_version_annotation(&mut out, &ty.version);
    push_ffi_handle_note(&mut out, &tname, lang, ffi_prefix);

    let doc = clean_doc(&ty.doc, lang);
    let doc = demote_headings_to_start_at(&doc, TYPE_DOC_FIRST_HEADING_LEVEL);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    let fields: Vec<_> = if lang == Language::Rust {
        ty.fields.iter().collect()
    } else {
        binding_fields(&ty.fields).collect()
    };
    if !ty.is_opaque && !fields.is_empty() {
        out.push('\n');
        out.push_str("| Field | Type | Default | Description |\n");
        out.push_str("|-------|------|---------|-------------|\n");
        for field in fields {
            let fname = field_name(&field.name, lang);
            let fty = doc_type_with_optional(&field.ty, lang, field.optional, ffi_prefix);
            let fdefault = format_field_default(field, lang, api, ffi_prefix);
            let fdoc = {
                let raw = clean_doc_inline(&field.doc, lang);
                if raw.is_empty() {
                    generate_field_description(&field.name, &field.ty)
                } else {
                    raw
                }
            };
            out.push_str(&template_env::render(
                "field_row.jinja",
                minijinja::context! {
                    name => escape_table_cell(&fname),
                    ty => escape_table_cell(&fty),
                    default => escape_table_cell(&fdefault),
                    doc => escape_table_cell(&fdoc),
                },
            ));
        }
        out.push('\n');
    }

    let methods: Vec<_> = ty
        .methods
        .iter()
        .filter(|method| method_visible_in_lang(config, method, &ty.name, lang))
        .collect();
    if !methods.is_empty() {
        let methods_heading = if lang == Language::Elixir {
            "Functions"
        } else {
            "Methods"
        };
        out.push_str(&template_env::render(
            "heading.jinja",
            minijinja::context! { marker => "#####", title => methods_heading },
        ));
        for method in methods {
            out.push_str(&render_method(method, &ty.name, lang, config, ffi_prefix));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Language;
    use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeRef};

    #[test]
    fn type_doc_headings_stay_under_type_heading() {
        let ty = TypeDef {
            name: "ReportConfig".to_string(),
            doc: "## Default Behavior\n\nConfiguration notes.".to_string(),
            methods: vec![MethodDef {
                name: "validate".to_string(),
                receiver: Some(ReceiverKind::Ref),
                return_type: TypeRef::Unit,
                ..Default::default()
            }],
            ..Default::default()
        };

        let rendered = render_type(
            &ty,
            Language::Rust,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "sample",
        );

        assert!(
            rendered.contains("#### ReportConfig"),
            "type heading should render at h4; got:\n{rendered}"
        );
        assert!(
            rendered.contains("##### Default Behavior"),
            "type rustdoc heading should be demoted below h4; got:\n{rendered}"
        );
        assert!(
            rendered.contains("##### Methods"),
            "methods heading should remain at h5; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("\n###### Default Behavior"),
            "type rustdoc H2 heading must start at h5, not skip to h6; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("\n### Default Behavior"),
            "type rustdoc heading must not be promoted above the type heading; got:\n{rendered}"
        );
    }

    #[test]
    fn test_c_type_page_states_handle_representation() {
        let ty = TypeDef {
            name: "ChatCompletionRequest".to_string(),
            ..Default::default()
        };
        let rendered = render_type(
            &ty,
            Language::C,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );

        assert!(
            rendered.contains("#### HTMChatCompletionRequest"),
            "the heading keeps the logical name -- one page per DTO would collide on a shared \
             handle token; got:\n{rendered}"
        );
        assert!(
            rendered.contains("**C representation:**") && rendered.contains("HTMAlefHandle"),
            "a C DTO page must state the actual handle type explicitly; got:\n{rendered}"
        );
    }

    #[test]
    fn test_non_c_type_page_has_no_handle_note() {
        let ty = TypeDef {
            name: "ChatCompletionRequest".to_string(),
            ..Default::default()
        };
        let rendered = render_type(
            &ty,
            Language::Python,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );
        assert!(
            !rendered.contains("C representation"),
            "the handle note is a C-ABI-only concept; got:\n{rendered}"
        );
    }

    /// ~keep The handle token named in the page-level note and the handle token
    /// `doc_type` independently computes for any `TypeRef::Named` of this type must be
    /// the same string -- this is the type-page analogue of the earlier
    /// signature-vs-example and signature-vs-returns-prose cross-checks. If the note
    /// hardcoded a different spelling than the rest of the C pipeline actually emits,
    /// this test -- not a human re-reading the page -- catches the drift.
    #[test]
    fn test_c_type_page_handle_note_agrees_with_doc_type_handle_token() {
        let ty = TypeDef {
            name: "ChatCompletionRequest".to_string(),
            ..Default::default()
        };
        let rendered = render_type(
            &ty,
            Language::C,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "Htm",
        );

        let handle_token = crate::docs::doc_type(&TypeRef::Named(ty.name.clone()), Language::C, "Htm");
        assert!(
            rendered.contains(&handle_token),
            "note must name the same handle token doc_type computes ({handle_token}); got:\n{rendered}"
        );
    }
}
