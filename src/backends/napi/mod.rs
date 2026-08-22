//! Node.js (NAPI-RS) binding generator backend for alef.

mod gen_bindings;
pub(crate) mod template_env;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::NapiBackend;
/// Re-exported so the TypeScript e2e snippet generator's tests can typecheck a generated
/// snippet against the exact `.d.ts` union type this function produces, rather than a
/// hand-guessed copy of it. See `internal_tagged_union_dts_lines`'s doc comment for why this
/// matters. Test-only: nothing in the non-test binary calls it. ~keep
#[cfg(test)]
pub(crate) use gen_bindings::errors::internal_tagged_union_dts_lines;
