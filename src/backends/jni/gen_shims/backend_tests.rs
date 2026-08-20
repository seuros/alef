/// Regression coverage for [`JniBackend::generate_bindings`]'s tolerance of a missing
/// `[crates.kotlin_android]` block. Kept in a dedicated file rather than appended to
/// `gen_shims/tests.rs`, which is already at the 1,000-line module cap.
#[cfg(test)]
mod backend_tests {
    use super::*;

    /// `jni` used to hard-bail when `[crates.kotlin_android]` was absent, even though every
    /// accessor `emit_lib_rs` actually calls (`jni_kotlin_package`, `jni_excluded_functions`,
    /// `jni_excluded_types`, `jni_capsule_types`) already tolerates `None` and falls back to
    /// the same vendor-neutral placeholder package `ResolvedCrateConfig::kotlin_package`
    /// gives `java`/`kotlin`. A consumer enabling `jni` without also configuring
    /// `kotlin_android` got a hard generate failure for a language they did configure.
    #[test]
    fn generate_bindings_succeeds_without_kotlin_android_config() {
        use crate::core::config::NewAlefConfig;

        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
"#,
        )
        .expect("fixture config parses");
        let config = raw.resolve().expect("fixture config resolves").remove(0);
        assert!(
            config.kotlin_android.is_none(),
            "fixture must not configure kotlin_android"
        );

        let api = ApiSurface::default();
        let files = JniBackend
            .generate_bindings(&api, &config)
            .expect("jni generation must not require [crates.kotlin_android]");

        assert_eq!(files.len(), 1, "jni emits exactly one shim file: {files:?}");
        assert!(
            files[0].content.contains("unconfigured/alef"),
            "with no kotlin_android/kotlin package configured, jni's error class must fall back \
             to the same vendor-neutral placeholder package java/kotlin use: {}",
            files[0].content
        );
    }
}
