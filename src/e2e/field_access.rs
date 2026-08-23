//! Field path resolution for nested struct/map access in e2e assertions.
//!
//! The `FieldResolver` maps fixture field paths (e.g., "metadata.title") to
//! actual API struct paths (e.g., "metadata.document.title") and generates
//! language-specific accessor expressions.

mod ir_collection;
mod ir_enum;
mod optional_renderers;
mod parse;
mod renderers;
mod resolver;
mod types;

pub use types::{
    DartFirstClassMap, FieldResolver, IrCollectionMap, IrEnumMap, PhpGetterMap, StringyField, StringyFieldKind,
    SwiftFirstClassMap,
};

#[cfg(test)]
#[path = "field_access/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "field_access/ir_enum_tests.rs"]
mod ir_enum_tests;

#[cfg(test)]
#[path = "field_access/csharp_optional_index_tests.rs"]
mod csharp_optional_index_tests;

#[cfg(test)]
#[path = "field_access/zig_method_call_accessor_tests.rs"]
mod zig_method_call_accessor_tests;

#[cfg(test)]
#[path = "field_access/ir_wire_optional_fields_tests.rs"]
mod ir_wire_optional_fields_tests;
