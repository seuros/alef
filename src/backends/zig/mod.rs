//! Zig binding generator backend for alef.
//!
//! Phase 1A skeleton: registers `ZigBackend` and exposes `BuildConfig`
//! with `BuildDependency::Ffi`. Real codegen lands in Phase 1B.

pub(crate) mod gen_bindings;
pub(crate) mod template_env;
mod trait_bridge;
pub(crate) mod type_map;

pub use gen_bindings::ZigBackend;
pub use trait_bridge::ZigTraitBridgeGenerator;

// ~keep Reachable by the docs layer (`docs::signatures`, `docs::language_pages`) so a Zig
// function/method's documented param and return types come from the same predicate the
// emitter itself consults, rather than a docs-local guess at whether a `Named` DTO crosses
// the wrapper boundary as a struct or as JSON bytes.
pub(crate) use gen_bindings::{zig_boundary_param_type, zig_boundary_return_type};
