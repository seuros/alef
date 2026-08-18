//! Go (cgo) binding generator backend for alef.

pub(crate) mod cgo_features;
mod gen_bindings;
pub mod gen_visitor;
pub(crate) mod template_env;
pub mod trait_bridge;
pub mod type_map;

pub use gen_bindings::GoBackend;
pub(crate) use gen_bindings::types::{
    GoEnumRepresentation, GoStructEnumVariantField, go_adjacent_tagged_constructor,
    go_data_enum_untagged_variant_matches, go_data_enum_variant_field, go_data_enum_variant_scalar_tuple_field,
    go_data_enum_variant_struct, go_enum_constant_for_wire_value, go_enum_representation, go_struct_enum_tag_field,
    go_struct_enum_variant_fields, needs_omitempty_pointer,
};
