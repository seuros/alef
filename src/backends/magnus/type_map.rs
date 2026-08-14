use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, TypeRef};
use std::borrow::Cow;

/// TypeMapper for Magnus (Ruby) bindings — default Rust types with String for Json.
pub struct MagnusMapper;

impl TypeMapper for MagnusMapper {
    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn error_wrapper(&self) -> &str {
        "Result"
    }

    /// Magnus wraps errors as `Result<T, Error>`.
    fn wrap_return(&self, base: &str, has_error: bool) -> String {
        if has_error {
            format!("Result<{base}, Error>")
        } else {
            base.to_string()
        }
    }
}

/// Maps a TypeRef to the Ruby type a generated accessor actually hands back.
///
/// ~keep Differs from [`rbs_type`] on `Json` alone, and only because the two describe different
/// things: `rbs_type`'s `json_value` is the shape of the *parsed* document, while the binding
/// returns whatever [`MagnusMapper`] rendered, and `MagnusMapper::json` is `String` — the value
/// has already been serialized by the time Ruby sees it. Declaring `json_value` on an accessor
/// therefore tells steep that `page.extracted_data["key"]` type-checks, when at runtime it is a
/// `String` and raises. The substitution has to recurse, or `Array[json_value]` keeps the same
/// lie one level down. Every other case delegates, so the two stay in step by construction.
pub fn rbs_marshalled_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Json => "String".to_string(),
        TypeRef::Optional(inner) => format!("{}?", rbs_marshalled_type(inner)),
        TypeRef::Vec(inner) => format!("Array[{}]", rbs_marshalled_type(inner)),
        TypeRef::Map(key, value) => {
            format!("Hash[{}, {}]", rbs_marshalled_type(key), rbs_marshalled_type(value))
        }
        other => rbs_type(other),
    }
}

/// Maps a TypeRef to its Ruby representation for .rbs stubs.
pub fn rbs_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::I64
            | PrimitiveType::Usize
            | PrimitiveType::Isize => "Integer".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "Float".to_string(),
        },
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "String".to_string(),
        TypeRef::Optional(inner) => format!("{}?", rbs_type(inner)),
        TypeRef::Vec(inner) => format!("Array[{}]", rbs_type(inner)),
        TypeRef::Map(k, v) => {
            format!("Hash[{}, {}]", rbs_type(k), rbs_type(v))
        }
        TypeRef::Named(name) => name.clone(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Json => "json_value".to_string(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Duration => "Integer".to_string(),
    }
}
