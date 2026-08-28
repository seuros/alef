/// Resolve the Kotlin package for JNI-mode output.
///
/// Delegates to [`crate::core::jni::jni_package`] — the canonical resolver shared with
/// `alef-backend-jni`'s own package resolution, so the Kotlin `external fun` declarations this
/// crate emits and the Rust `Java_*` symbols the jni backend emits can never disagree about which
/// package an unconfigured crate falls back to. This used to be a second, hand-copied precedence
/// chain here; see that function's doc comment for the drift that duplication caused. ~keep
pub(in crate::backends::kotlin) fn jni_kotlin_package(config: &ResolvedCrateConfig) -> String {
    crate::core::jni::jni_package(config)
}

/// Resolve the output path for a JNI-mode Kotlin file.
///
/// Uses `[crates.output] kotlin_android` when available, otherwise falls
/// back to `[crates.output] kotlin`, and finally the conventional
/// `packages/kotlin/src/main/kotlin/<pkg>/` layout.
///
/// `output_for("kotlin_android")` answers the Gradle project *root* -- where
/// `build.gradle.kts` lives -- not a source directory: joining `filename` onto it directly
/// wrote JNI-mode `.kt` files straight into the project root instead of under
/// `src/main/kotlin/<pkg>/`. Delegate to the kotlin_android backend's own
/// [`crate::backends::kotlin_android::kotlin_source_dir`], which already resolves the same
/// root-vs-source-dir ambiguity `ProjectLayout` handles for that backend's own file
/// placement. ~keep
pub(in crate::backends::kotlin) fn jni_output_path(config: &ResolvedCrateConfig, filename: &str) -> PathBuf {
    if config.output_for("kotlin_android").is_some() {
        return crate::backends::kotlin_android::kotlin_source_dir(config).join(filename);
    }
    let kotlin_root = config
        .output_for("kotlin")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/kotlin".to_string());
    let package = jni_kotlin_package(config);
    let package_path = package.replace('.', "/");
    if config.explicit_output.kotlin.is_some() {
        PathBuf::from(&kotlin_root).join(filename)
    } else {
        PathBuf::from(&kotlin_root)
            .join("src/main/kotlin")
            .join(&package_path)
            .join(filename)
    }
}
