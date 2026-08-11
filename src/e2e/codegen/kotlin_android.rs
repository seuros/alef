//! Kotlin Android e2e test generator using kotlin.test and JUnit 5.
//!
//! Generates host-JVM tests that validate the AAR-bundled Java facade and Kotlin wrapper
//! via JNA against the generated FFI library. Tests are emitted to `e2e/kotlin_android/src/test/kotlin/`
//! without requiring an Android emulator — the tests run directly on the host JVM against
//! the shared library.

mod enum_fixtures;
mod gradle;
mod gradle_wrapper;
mod project;
mod stubs;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;

use super::E2eCodegen;

pub use stubs::emit_test_backend;

/// Kotlin Android e2e code generator.
/// Emits a host-JVM test project that depends on the AAR-bundled Java facade
/// and Kotlin wrapper via sourceSets and JNA, without requiring an Android emulator.
pub struct KotlinAndroidE2eCodegen;

impl E2eCodegen for KotlinAndroidE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        project::generate(groups, e2e_config, config, type_defs)
    }

    fn render_snippet_body(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<String> {
        Ok(crate::e2e::codegen::kotlin::snippet::render_snippet_body(
            fixture, e2e_config, config, type_defs, true,
        ))
    }

    fn language_name(&self) -> &'static str {
        "kotlin_android"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride};
    use crate::e2e::fixture::Fixture;

    #[test]
    fn snippet_uses_android_coroutine_call_without_junit_harness() {
        let fixture = Fixture {
            id: "quick_start".into(),
            description: "Quick start".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "load_document".into(),
            result_var: "document".into(),
            r#async: true,
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin_android".into(),
            CallOverride {
                class: Some("DocumentApi".into()),
                ..CallOverride::default()
            },
        );
        let body = KotlinAndroidE2eCodegen
            .render_snippet_body(
                &fixture,
                &E2eConfig {
                    call,
                    ..E2eConfig::default()
                },
                &ResolvedCrateConfig::default(),
                &[],
                &[],
            )
            .expect("snippet renders");

        assert!(body.contains("kotlinx.coroutines.runBlocking"));
        assert!(body.contains("DocumentApi.loadDocument()"));
        assert!(!body.contains("@Test"));
        assert!(!body.contains("assert"));
    }

    #[test]
    fn excluded_binding_fixture_uses_native_disabled_test() {
        let rendered = crate::e2e::template_env::render(
            "kotlin_android/excluded_fixtures.kt.jinja",
            minijinja::context! {
                package_name => "dev.sample",
                fixtures => vec!["visitor_round_trip"],
                reason => "visitor is excluded by crates.kotlin_android.exclude_functions",
            },
        );

        assert!(rendered.contains("@Disabled(\"visitor is excluded by crates.kotlin_android.exclude_functions\")"));
        assert!(rendered.contains("fun visitor_round_trip() {}"));
    }
}
