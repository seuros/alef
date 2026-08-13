//! Swift binding generator backend for alef.
//!
//! Phase 2A skeleton: registers `SwiftBackend` targeting Apple platforms
//! (macOS, iOS, tvOS, watchOS, visionOS). Linux Swift uses the same backend
//! with a separate CI matrix; no platform-specific codegen is needed here.
//! Real codegen (swift-bridge wiring, type generation) lands in Phase 2B.

pub mod gen_bindings;
pub mod gen_rust_crate;
pub mod naming;
mod template_env;
pub(crate) mod type_map;

pub use gen_bindings::SwiftBackend;

pub(crate) fn signatures_reference_named<'a>(
    types: impl IntoIterator<Item = &'a crate::core::ir::TypeDef>,
    functions: impl IntoIterator<Item = &'a crate::core::ir::FunctionDef>,
    name: &str,
) -> bool {
    let function_references = functions.into_iter().any(|function| {
        function.return_type.references_named(name)
            || function.params.iter().any(|param| param.ty.references_named(name))
    });
    function_references
        || types.into_iter().any(|ty| {
            ty.fields.iter().any(|field| field.ty.references_named(name))
                || ty.methods.iter().any(|method| {
                    method.return_type.references_named(name)
                        || method.params.iter().any(|param| param.ty.references_named(name))
                })
        })
}
