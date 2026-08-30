use crate::codegen::naming::{to_go_name, wire_field_name};
use crate::core::ir::{EnumDef, FieldDef};

use super::enums::{GoEnumRepresentation, go_enum_representation};
use super::helpers::is_tuple_field;

/// The exported field name and JSON key declared for a sealed-interface variant field. ~keep
pub(crate) fn go_data_enum_variant_field(enum_def: &EnumDef, field: &FieldDef) -> Option<(String, String)> {
    if is_tuple_field(field) {
        return None;
    }
    let go_name = to_go_name(&field.name);
    let wire_name = wire_field_name(
        &field.name,
        field.serde_rename.as_deref(),
        enum_def.rename_all_fields.as_deref(),
    );
    Some((go_name, wire_name))
}

pub(crate) fn is_unit_struct_field_enum(enum_def: &EnumDef) -> bool {
    matches!(
        go_enum_representation(enum_def),
        GoEnumRepresentation::UnitString
            | GoEnumRepresentation::NewtypeTupleString
            | GoEnumRepresentation::AdjacentTaggedStruct
            | GoEnumRepresentation::TupleTaggedStruct
            | GoEnumRepresentation::ExternallyTaggedStruct
    )
}

pub(crate) fn is_data_interface_struct_field_enum(enum_def: &EnumDef) -> bool {
    go_enum_representation(enum_def) == GoEnumRepresentation::DataInterface
}
