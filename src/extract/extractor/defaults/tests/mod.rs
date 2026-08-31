use super::*;

fn default_value_of(expr_src: &str) -> DefaultValue {
    default_value_of_with_consts(expr_src, &[])
}

fn default_value_of_with_consts(expr_src: &str, consts: &[(&str, &str)]) -> DefaultValue {
    let expr: syn::Expr = syn::parse_str(expr_src).expect("valid expr");
    let literal_consts: AHashMap<String, DefaultValue> = consts
        .iter()
        .map(|(k, v)| (k.to_string(), DefaultValue::StringLiteral(v.to_string())))
        .collect();
    let field_types = AHashMap::new();
    expr_to_default_value(&expr, &EvalScope::new("Subject", &literal_consts, &field_types), None)
}

/// Drive the whole extractor over a module source, returning the resolved defaults for
/// the named type's `impl Default`. Reproduces exactly what `extractor::mod` does: build
/// the const and constructor indexes from the module's items, then read the `impl Default`
/// against them.
fn defaults_for(source: &str, type_name: &str, field_names: &[&str]) -> Vec<(String, DefaultValue)> {
    let fields: Vec<(&str, TypeRef)> = field_names.iter().map(|name| (*name, TypeRef::Unit)).collect();
    defaults_for_typed(source, type_name, &fields)
}

/// As [`defaults_for`], but with each field's declared type spelled out. Path and collection
/// mutation lowering consult it; every other case can keep using the untyped helper. ~keep
fn defaults_for_typed(source: &str, type_name: &str, fields: &[(&str, TypeRef)]) -> Vec<(String, DefaultValue)> {
    let file: syn::File = syn::parse_str(source).expect("valid module source");
    let literal_consts = collect_literal_consts(&file.items);
    let constructors = collect_constructors(&file.items);

    let default_impl = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Impl(item_impl)
                if item_impl
                    .trait_
                    .as_ref()
                    .is_some_and(|(path, _)| path.segments.last().is_some_and(|s| s.ident == "Default"))
                    && path_type_name(&item_impl.self_ty).as_deref() == Some(type_name) =>
            {
                Some(item_impl)
            }
            _ => None,
        })
        .expect("module declares `impl Default` for the type");

    let mut fields: Vec<FieldDef> = fields
        .iter()
        .map(|(name, ty)| FieldDef {
            name: (*name).to_string(),
            ty: ty.clone(),
            ..Default::default()
        })
        .collect();

    extract_default_values(
        default_impl,
        type_name,
        &mut fields,
        &literal_consts,
        &constructors,
        false,
    );

    fields
        .into_iter()
        .map(|field| {
            let value = field.typed_default.expect("every field is assigned a default");
            (field.name, value)
        })
        .collect()
}

/// What `codegen::config_gen::shared` writes into a generated binding for this field, which
/// is where the fabrication was observable: an `EnumVariant` on a `String`-typed field is
/// rendered as the *snake-cased variant name*, so `EnumVariant("DEFAULT_MODEL")` shipped the
/// string `"default_model"` — a value that appears nowhere in the source crate. ~keep
fn rendered_python_default(name: &str, ty: TypeRef, value: &DefaultValue) -> String {
    let field = FieldDef {
        name: name.to_string(),
        ty,
        typed_default: Some(value.clone()),
        ..Default::default()
    };
    crate::codegen::config_gen::default_value_for_field(&field, "python")
}

mod cfg_gated_field_defaults;
mod delegation;
mod enum_and_associated_consts;
mod function_call_folding;
mod literals_and_consts;
mod mutated_literal;
mod mutated_literal_unresolved;
mod variant_folding;
