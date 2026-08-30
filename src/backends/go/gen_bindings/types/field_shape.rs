use crate::core::ir::EnumDef;

use super::enums::{GoEnumRepresentation, go_enum_representation};

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
