//! C FFI binding generator backend for alef.

pub(crate) mod gen_bindings;
pub mod gen_bridge_field;
mod gen_visitor;
/// Test-only: the shared handle-ABI stamp assertion every hand-declaring
/// backend's stamp test calls. Lives here because the FFI backend owns the
/// authoritative handle representation. ~keep
#[cfg(test)]
pub(crate) mod handle_abi_stamp;
pub(crate) mod template_env;
pub mod trait_bridge;
pub mod type_map;

pub use gen_bindings::FfiBackend;
