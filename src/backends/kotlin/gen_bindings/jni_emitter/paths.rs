/// Resolve the Kotlin package for JNI-mode output.
///
/// Prefers `[crates.kotlin_android] package`, then `[crates.kotlin] package`,
/// then falls back to `config.kotlin_package()`.
pub(in crate::backends::kotlin) fn jni_kotlin_package(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|a| a.package.clone())
        .or_else(|| config.kotlin.as_ref().and_then(|k| k.package.clone()))
        .unwrap_or_else(|| config.kotlin_package())
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
