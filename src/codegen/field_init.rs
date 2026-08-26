//! Rust struct-literal field initializers.
//!
//! Generated Rust is compiled by consumers under `-D warnings`, so a `Foo { bar: bar }` initializer
//! is a build failure (`clippy::redundant_field_names`), not a cosmetic wart. Every emitter that
//! builds a struct literal from a value expression routes the pair through [`struct_field_init`],
//! which collapses to field-init shorthand only when the expression is *exactly* the field
//! identifier — a value expression that merely starts with the field name (`bar.into()`,
//! `bar.unwrap_or_default()`) keeps the explicit `bar: …` form.

/// Render one struct-literal field initializer as `field: value`, or as field-init shorthand
/// (`field`) when `value_expr` is exactly `field_ident`.
///
/// The caller owns the invariant that `value_expr` names a binding in scope holding this field's
/// value; this helper only decides the spelling.
pub fn struct_field_init(field_ident: &str, value_expr: &str) -> String {
    if field_ident == value_expr {
        return field_ident.to_string();
    }
    format!("{field_ident}: {value_expr}")
}

#[cfg(test)]
mod tests {
    use super::struct_field_init;

    #[test]
    fn should_collapse_to_shorthand_when_value_is_exactly_the_field_identifier() {
        assert_eq!(struct_field_init("timeout", "timeout"), "timeout");
    }

    #[test]
    fn should_keep_explicit_form_when_value_expression_only_starts_with_the_field_name() {
        assert_eq!(
            struct_field_init("timeout", "timeout.unwrap_or_default()"),
            "timeout: timeout.unwrap_or_default()"
        );
        assert_eq!(struct_field_init("kind", "kind.into()"), "kind: kind.into()");
    }

    #[test]
    fn should_keep_explicit_form_when_field_and_value_names_differ() {
        assert_eq!(struct_field_init("type_", "r#type"), "type_: r#type");
        assert_eq!(struct_field_init("text", "field0"), "text: field0");
    }

    #[test]
    fn should_not_collapse_when_value_is_a_prefix_of_the_field_identifier() {
        assert_eq!(struct_field_init("timeout_ms", "timeout"), "timeout_ms: timeout");
    }
}
