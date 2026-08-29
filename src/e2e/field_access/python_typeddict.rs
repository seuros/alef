//! Derives which IR types the pyo3 backend emits as a Python `TypedDict` (subscript access,
//! `result["field"]`) rather than a `@dataclass` / `pydantic.BaseModel` / `msgspec.Struct` /
//! native `#[pyclass]` (attribute access, `result.field`), so the python e2e generator renders
//! accessors that agree with what the backend actually emits.
//!
//! Before this module existed, the python e2e renderer had no way to know which shape the pyo3
//! backend chose for a `[workspace.dto] python_output = "typed-dict"` crate: it always emitted
//! `.field` attribute access, which is `AttributeError: 'dict' object has no attribute 'field'`
//! against a return type the backend actually emits as a `TypedDict` (a plain `dict` at
//! runtime). The fix asks the pyo3 backend's own predicate
//! (`crate::backends::pyo3::gen_bindings::errors::is_dataclass_backed_config`) instead of
//! reimplementing `python_output_style()`-plus-`is_return_type` here — see the
//! `two-generators-disagree` skill: a second, parallel copy of that rule is the defect shape
//! this fixes, not a valid alternative fix. ~keep
use std::collections::{HashMap, HashSet};

use crate::backends::pyo3::gen_bindings::errors::is_dataclass_backed_config;
use crate::core::config::PythonDtoStyle;
use crate::core::ir::TypeDef;
use crate::e2e::codegen::call_ir::named_type;

use super::types::PythonTypedDictMap;

/// Build the `TypedDict`-membership set and `(type, field) -> next type` traversal edges
/// [`PythonTypedDictMap`] needs, by inspecting every `TypeDef` this crate declares.
///
/// A type is classified as `TypedDict` under the exact same condition `options.py` uses to
/// decide whether to emit it as one: `typ.is_return_type && is_dataclass_backed_config(typ,
/// output_style, reexported)`. A field is additionally recorded as a traversal edge when its
/// [`named_type`]-resolved type is another `TypeDef` in this crate, exactly as
/// `ir_enum::build_ir_enum_map` and `ir_collection::build_ir_collection_map` do, so a
/// multi-segment path can advance its "current owner type" cursor one segment at a time before
/// asking `is_typeddict` at each link.
pub(super) fn build_python_typeddict_map(
    type_defs: &[TypeDef],
    output_style: PythonDtoStyle,
    reexported_types: &[String],
) -> PythonTypedDictMap {
    let struct_names: HashSet<&str> = type_defs.iter().map(|t| t.name.as_str()).collect();
    let reexported: ahash::AHashSet<&str> = reexported_types.iter().map(String::as_str).collect();

    let mut typeddict_types: HashSet<String> = HashSet::new();
    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();

    for type_def in type_defs {
        if type_def.is_return_type && is_dataclass_backed_config(type_def, output_style, &reexported) {
            typeddict_types.insert(type_def.name.clone());
        }
        for field in &type_def.fields {
            let Some(named) = named_type(&field.ty) else {
                continue;
            };
            if struct_names.contains(named) {
                field_types
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone(), named.to_string());
            }
        }
    }

    PythonTypedDictMap {
        typeddict_types,
        field_types,
        root_type: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeRef};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn return_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            fields,
            is_return_type: true,
            has_default: true,
            ..TypeDef::default()
        }
    }

    /// `ParseOutput { metadata: Metadata }`, `Metadata { title: String }`, both `is_return_type`.
    /// Under `TypedDict` output style neither is reexported, so both classify as `TypedDict`.
    fn type_defs() -> Vec<TypeDef> {
        vec![
            return_type(
                "ParseOutput",
                vec![field("metadata", TypeRef::Named("Metadata".to_string()))],
            ),
            return_type("Metadata", vec![field("title", TypeRef::String)]),
        ]
    }

    #[test]
    fn a_typed_dict_output_style_return_type_is_classified_as_typeddict() {
        let map = build_python_typeddict_map(&type_defs(), PythonDtoStyle::TypedDict, &[]);
        assert!(map.typeddict_types.contains("ParseOutput"));
        assert!(map.typeddict_types.contains("Metadata"));
    }

    #[test]
    fn a_reexported_return_type_is_not_classified_as_typeddict() {
        let map = build_python_typeddict_map(&type_defs(), PythonDtoStyle::TypedDict, &["ParseOutput".to_string()]);
        assert!(!map.typeddict_types.contains("ParseOutput"));
        assert!(map.typeddict_types.contains("Metadata"));
    }

    #[test]
    fn a_dataclass_output_style_return_type_is_not_classified_as_typeddict() {
        let map = build_python_typeddict_map(&type_defs(), PythonDtoStyle::Dataclass, &[]);
        assert!(map.typeddict_types.is_empty());
    }

    #[test]
    fn a_named_field_is_recorded_as_a_traversal_edge_regardless_of_typeddict_classification() {
        let map = build_python_typeddict_map(&type_defs(), PythonDtoStyle::Dataclass, &[]);
        assert_eq!(
            map.field_types.get("ParseOutput").and_then(|f| f.get("metadata")),
            Some(&"Metadata".to_string())
        );
    }
}
