use crate::core::ir::{FunctionDef, TypeRef};
use crate::e2e::config::CallConfig;

pub(super) fn resolve_raw_c_result_type(call: &CallConfig, functions: &[FunctionDef]) -> Option<String> {
    let function = functions.iter().find(|function| function.name == call.function)?;
    c_string_return(&function.return_type).then(|| "char*".to_string())
}

fn c_string_return(return_type: &TypeRef) -> bool {
    match return_type {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => true,
        TypeRef::Optional(inner) => c_string_return(inner),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_returns_use_owned_c_strings() {
        for return_type in [
            TypeRef::Vec(Box::new(TypeRef::String)),
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
        ] {
            assert!(c_string_return(&return_type));
        }
    }

    #[test]
    fn named_returns_remain_opaque_handles() {
        assert!(!c_string_return(&TypeRef::Named("SampleResult".into())));
    }
}
