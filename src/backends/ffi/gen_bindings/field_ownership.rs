use crate::codegen::naming::pascal_to_snake;
use crate::core::ir::TypeRef;
use ahash::AHashSet;

pub(super) fn field_accessor_ownership_lines(
    ty: &TypeRef,
    prefix: &str,
    enum_names: &AHashSet<String>,
    clone_names: &AHashSet<String>,
    override_type_name: Option<&str>,
) -> Vec<String> {
    match ty {
        TypeRef::Optional(inner) => {
            field_accessor_ownership_lines(inner, prefix, enum_names, clone_names, override_type_name)
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            vec![
                "A non-null returned pointer is owned by the caller.".to_string(),
                format!("It must be freed with `{prefix}_free_string`."),
            ]
        }
        TypeRef::Bytes => vec![
            "The returned byte pointer is borrowed from `ptr` and must not be freed.".to_string(),
            "It remains valid until `ptr` is destroyed or the field is mutated.".to_string(),
        ],
        TypeRef::Named(name) if enum_names.contains(name) || clone_names.contains(name) => {
            let free_type = override_type_name.unwrap_or(name);
            vec![
                "A non-null returned handle is owned by the caller.".to_string(),
                format!("It must be freed with `{prefix}_{}_free`.", pascal_to_snake(free_type)),
            ]
        }
        _ => Vec::new(),
    }
}
