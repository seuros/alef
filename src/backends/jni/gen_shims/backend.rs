/// Backend that emits the Rust JNI shim crate source.
#[derive(Debug, Default, Clone, Copy)]
pub struct JniBackend;

impl Backend for JniBackend {
    fn name(&self) -> &str {
        "jni"
    }

    fn language(&self) -> Language {
        Language::Jni
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: false,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: true,
            supports_service_api: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // No hard requirement on `[crates.kotlin_android]` here: every downstream
        // accessor (`jni_kotlin_package`, `jni_excluded_functions`, `jni_excluded_types`,
        // `jni_capsule_types`) already tolerates its absence, falling back to the same
        // vendor-neutral placeholder package (`ResolvedCrateConfig::kotlin_package`) the
        // `kotlin_android` backend itself defaults to when unconfigured. Bailing here would
        // make `jni` the only language that hard-fails generation for a config gap every
        // sibling accessor already treats as a soft default. ~keep
        let output_path = jni_output_path(config);
        let content = emit_lib_rs(api, config);
        Ok(vec![GeneratedFile {
            path: output_path,
            content,
            generated_header: true,
        }])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        super::service_api::generate(api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-jni",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

/// Default output directory: `crates/<crate-base>-jni/src/lib.rs`
///
/// `crate-base` is `config.jni_crate_base()`: `[crates.jni] crate_dir` when
/// set, otherwise `config.name`.  The override lets consumers whose name
/// carries a language suffix (e.g. `"sample-markdown-rs"`) produce a crate
/// at `crates/sample-markdown-jni/` that matches all other binding crates.
fn jni_output_path(config: &ResolvedCrateConfig) -> PathBuf {
    let jni_crate = format!("{}-jni", config.jni_crate_base());
    PathBuf::from(format!("crates/{jni_crate}/src/lib.rs"))
}
