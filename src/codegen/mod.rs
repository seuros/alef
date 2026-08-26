//! Shared code generation utilities for all language backends.
//! Provides struct/enum/function generators, type mapping, and conversion helpers.

pub(crate) mod template_env;

pub mod builder;
pub mod c_consumer;
pub mod cfg;
pub mod config_gen;
pub mod conversions;
pub mod crate_attributes;
pub mod defaults;
pub mod doc_emission;
pub mod error_gen;
pub mod field_init;
pub mod fn_dedup;
pub mod generators;
pub mod keywords;
pub mod mut_writeback;
pub mod naming;
pub mod serde_enum_repr;
pub mod shared;
pub mod type_mapper;
pub(crate) mod visitor_context;
pub(crate) mod visitor_context_abi;
pub(crate) mod visitor_result;

#[cfg(test)]
mod duration_wire_cross_backend_tests;
#[cfg(test)]
mod serde_enum_wire_cross_backend_tests;
