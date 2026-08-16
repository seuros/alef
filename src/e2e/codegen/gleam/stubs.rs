/// Emit a Gleam test backend stub.
///
/// Gleam has no `test_backend` stub generator yet. Panic rather than return a
/// placeholder `TestBackendEmission` a caller could accidentally splice into
/// generated code. ~keep
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> crate::e2e::codegen::TestBackendEmission {
    panic!(
        "Gleam e2e generator: fixture `{}` requires a Gleam test_backend stub for trait `{}`, but the Gleam test-backend emitter is unimplemented; refusing to emit a call with a comment where the argument belongs",
        fixture.id, trait_bridge.trait_name
    );
}
