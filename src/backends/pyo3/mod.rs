//! Python (PyO3) binding generator backend for alef.

// pub(crate): e2e::codegen::python calls gen_bindings::crate_has_serde to mirror this backend's
// from_json eligibility gate. ~keep
pub(crate) mod gen_bindings;
mod gen_stubs;
#[cfg(test)]
mod kwarg_unpack_tests;
mod py_signature;
#[cfg(test)]
mod signature_agreement_tests;
mod template_env;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::Pyo3Backend;
