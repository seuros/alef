/// Emit a Kotlin test backend stub.
///
/// Kotlin (JVM) is not currently in e2e.languages, so trait_bridge e2e tests
/// are not generated for it. This path is defensively unreachable in practice —
/// panic rather than return a placeholder `TestBackendEmission` a caller could
/// accidentally splice into generated code. ~keep
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> crate::e2e::codegen::TestBackendEmission {
    panic!(
        "Kotlin (JVM) e2e generator: fixture `{}` requires a Kotlin (JVM) test_backend stub for trait `{}`, but the Kotlin (JVM) test-backend emitter is unimplemented; refusing to emit a call with a comment where the argument belongs",
        fixture.id, trait_bridge.trait_name
    );
}
