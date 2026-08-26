//! C# wrapper class and method code generation.

mod adapters;
mod bridge_fields;
mod class;
mod native_call;
mod wrappers;

pub(super) use class::gen_wrapper_class;
