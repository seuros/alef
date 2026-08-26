use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{FieldDef, TypeRef};

use super::field_fragments::sanitized_vec_field_to_core_expr;
use super::{field_references_excluded_type, is_tuple_variant};

/// Wrap a sanitized field's JSON deserialize so a value that fails to parse emits a
/// `tracing::warn!` before falling back to `Default::default()`, instead of swallowing the
/// parse error with no diagnostic. Only reachable for `binding_enums_have_data: true` backends
/// (rustler, magnus), where the field's JSON text comes from a host-language caller. Stays
/// infallible on purpose: this expression sits inside `impl From<Binding> for Core`, and every
/// backend call site uses `.into()`, so returning `Result` here would require a coordinated
/// `TryFrom` migration across backends outside this module's territory. ~keep
fn sanitized_field_parse_or_warn(access: &str, variant_name: &str, field_name: &str) -> String {
    let context = format!("variant = \"{variant_name}\", field = \"{field_name}\"");
    crate::codegen::template_env::render(
        "conversions/sanitized_json_parse_or_warn",
        minijinja::context! {
            access => access,
            context => context,
            message => "binding provided unparseable JSON for enum variant field; substituting default",
        },
    )
    .trim_end()
    .to_string()
}

/// Generate a match arm for binding -> core direction.
/// Binding enums are always unit-variant-only; core enums may have data variants.
/// `binding_has_data` controls whether the binding enum has the variant's fields (true) or is
/// unit-only (false, e.g. Rustler/Elixir).
/// `binding_uses_tuple_form` records the binding-side variant body shape for tuple variants,
/// so the destructure pattern matches the declaration emitted by the backend template.
/// Generate match arm for binding->core conversion with config (handles type conversions).
pub fn binding_to_core_match_arm_ext_cfg(
    binding_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
    config: &ConversionConfig,
    binding_uses_tuple_form: bool,
) -> String {
    use crate::codegen::conversions::field_conversion_to_core_cfg;

    if fields.is_empty() {
        format!("{binding_prefix}::{variant_name} => Self::{variant_name},")
    } else if !binding_has_data {
        if is_tuple_variant(fields) {
            let defaults: Vec<&str> = fields.iter().map(|_| "Default::default()").collect();
            format!(
                "{binding_prefix}::{variant_name} => Self::{variant_name}({}),",
                defaults.join(", ")
            )
        } else {
            let defaults: Vec<String> = fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            format!(
                "{binding_prefix}::{variant_name} => Self::{variant_name} {{ {} }},",
                defaults.join(", ")
            )
        }
    } else if is_tuple_variant(fields) {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let binding_pattern = field_names.join(", ");
        let core_args: Vec<String> = fields
            .iter()
            .map(|f| {
                let name = &f.name;
                if f.sanitized {
                    let expr = if let TypeRef::Vec(_) = &f.ty {
                        sanitized_vec_field_to_core_expr(name, &f.ty)
                    } else {
                        sanitized_field_parse_or_warn(name, variant_name, name)
                    };
                    return if f.is_boxed { format!("Box::new({expr})") } else { expr };
                }
                if !config.exclude_types.is_empty() && field_references_excluded_type(&f.ty, config.exclude_types) {
                    let expr = sanitized_field_parse_or_warn(name, variant_name, name);
                    return if f.is_boxed { format!("Box::new({expr})") } else { expr };
                }
                let conv = field_conversion_to_core_cfg(name, &f.ty, f.optional, config);
                let expr = if let Some(expr) = conv.strip_prefix(&format!("{name}: ")) {
                    let expr = expr.replace(&format!("val.{name}"), name);
                    expr.to_string()
                } else {
                    conv
                };
                if f.is_boxed { format!("Box::new({expr})") } else { expr }
            })
            .collect();
        let pattern_syntax = if binding_uses_tuple_form {
            format!("{binding_prefix}::{variant_name}({binding_pattern})")
        } else {
            format!("{binding_prefix}::{variant_name} {{ {binding_pattern} }}")
        };
        format!("{pattern_syntax} => Self::{variant_name}({}),", core_args.join(", "))
    } else {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let core_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                if f.sanitized {
                    if let TypeRef::Vec(_) = &f.ty {
                        let expr = sanitized_vec_field_to_core_expr(&f.name, &f.ty);
                        return format!("{}: {expr}", f.name);
                    }
                    let expr = sanitized_field_parse_or_warn(&f.name, variant_name, &f.name);
                    return format!("{}: {expr}", f.name);
                }
                let conv = field_conversion_to_core_cfg(&f.name, &f.ty, f.optional, config);
                let expr = if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    expr.replace(&format!("val.{}", f.name), &f.name)
                } else {
                    conv.strip_prefix(&format!("{}: ", f.name)).unwrap_or(&conv).to_string()
                };
                let expr = if f.is_boxed {
                    if f.optional {
                        format!("{expr}.map(Box::new)")
                    } else {
                        format!("Box::new({expr})")
                    }
                } else {
                    expr
                };
                format!("{}: {expr}", f.name)
            })
            .collect();
        format!(
            "{binding_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            core_fields.join(", ")
        )
    }
}

/// Generate a match arm for core -> binding direction.
/// When the binding also has data variants, destructure and forward fields.
/// When the binding is unit-variant-only, discard core data with `..`.
/// `binding_has_data` controls whether the binding enum has the variant's fields (true) or is
/// unit-only (false).
/// `binding_uses_tuple_form` records the binding-side variant body shape for tuple variants,
/// so the constructor matches the declaration emitted by the backend template.
/// Generate match arm for core->binding conversion with config (handles type conversions).
pub fn core_to_binding_match_arm_ext_cfg(
    core_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
    config: &ConversionConfig,
    binding_uses_tuple_form: bool,
) -> String {
    use crate::codegen::conversions::field_conversion_from_core_cfg;
    use ahash::AHashSet;

    if fields.is_empty() {
        format!("{core_prefix}::{variant_name} => Self::{variant_name},")
    } else if !binding_has_data {
        if is_tuple_variant(fields) {
            format!("{core_prefix}::{variant_name}(..) => Self::{variant_name},")
        } else {
            format!("{core_prefix}::{variant_name} {{ .. }} => Self::{variant_name},")
        }
    } else if is_tuple_variant(fields) {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let core_pattern = field_names.join(", ");
        let binding_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                let conv =
                    field_conversion_from_core_cfg(&f.name, &f.ty, f.optional, f.sanitized, &AHashSet::new(), config);
                if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    let mut expr = expr.replace(&format!("val.{}", f.name), &f.name);
                    if f.is_boxed {
                        expr = expr.replace(&format!("{}.into()", f.name), &format!("(*{}).into()", f.name));
                    }
                    if binding_uses_tuple_form {
                        let string_move = format!("{}.to_string()", f.name);
                        if expr == string_move {
                            expr = f.name.clone();
                        }
                        expr
                    } else {
                        format!("{}: {}", f.name, expr)
                    }
                } else {
                    conv
                }
            })
            .collect();
        if binding_uses_tuple_form {
            format!(
                "{core_prefix}::{variant_name}({core_pattern}) => Self::{variant_name}({}),",
                binding_fields.join(", ")
            )
        } else {
            format!(
                "{core_prefix}::{variant_name}({core_pattern}) => Self::{variant_name} {{ {} }},",
                binding_fields.join(", ")
            )
        }
    } else {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let binding_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                let conv =
                    field_conversion_from_core_cfg(&f.name, &f.ty, f.optional, f.sanitized, &AHashSet::new(), config);
                if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    let mut expr = expr.replace(&format!("val.{}", f.name), &f.name);
                    if f.is_boxed {
                        expr = expr.replace(&format!("{}.into()", f.name), &format!("(*{}).into()", f.name));
                    }
                    format!("{}: {}", f.name, expr)
                } else {
                    conv
                }
            })
            .collect();
        format!(
            "{core_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            binding_fields.join(", ")
        )
    }
}
