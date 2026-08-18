mod config_options;
mod enums;
mod helpers;
mod mapping;
mod structs;

pub(super) use config_options::gen_config_options;
pub(crate) use enums::{
    GoEnumRepresentation, GoStructEnumVariantField, go_adjacent_tagged_constructor,
    go_data_enum_untagged_variant_matches, go_data_enum_variant_field, go_data_enum_variant_scalar_tuple_field,
    go_data_enum_variant_struct, go_enum_constant_for_wire_value, go_enum_representation, go_struct_enum_tag_field,
    go_struct_enum_variant_fields,
};
pub(super) use enums::{gen_enum_type, is_passthrough_raw_message_enum};
pub(crate) use helpers::needs_omitempty_pointer;
pub(super) use helpers::{
    emit_type_doc, gen_duration_millis_helper, gen_last_error_helper, gen_ptr_helper, gen_unmarshal_bytes_helper,
    is_tuple_field,
};
pub(super) use mapping::{cgo_type_for_primitive, go_return_expr, primitive_max_sentinel};
pub(super) use structs::{gen_opaque_type, gen_opaque_type_free_only, gen_struct_type, go_struct_field_names};

#[cfg(test)]
#[path = "types/tests.rs"]
mod tests;
