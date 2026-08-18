//! Go (cgo) binding generator backend for alef.

pub(crate) mod cgo_features;
mod gen_bindings;
pub mod gen_visitor;
pub(crate) mod template_env;
pub mod trait_bridge;
pub mod type_map;

pub use gen_bindings::GoBackend;
pub(crate) use gen_bindings::types::{
    GoEnumRepresentation, go_enum_constant_for_wire_value, go_enum_representation, needs_omitempty_pointer,
};
