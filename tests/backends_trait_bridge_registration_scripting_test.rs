//! `Backend::trait_bridge_registration_surface` for the scripting and dynamic-language targets.
//!
//! Same contract as `backends_trait_bridge_registration_surface_test.rs`: each test asserts the
//! exact reported symbols *and* that the generated output declares them, so renaming an emitted
//! entry point without updating the surface fails here.
//!
//! The configured names are deliberately not what a trait-derived scheme would produce
//! (`install_sample_plugin`, not `register_sample_plugin`), which is what makes Gleam's
//! trait-derived register name distinguishable from the verbatim-name backends.

use alef::backends::dart::DartBackend;
use alef::backends::extendr::ExtendrBackend;
use alef::backends::gleam::GleamBackend;
use alef::backends::jni::JniBackend;
use alef::backends::kotlin::KotlinBackend;
use alef::backends::magnus::MagnusBackend;
use alef::backends::napi::NapiBackend;
use alef::backends::php::PhpBackend;
use alef::backends::rustler::RustlerBackend;
use alef::backends::swift::SwiftBackend;
use alef::backends::wasm::WasmBackend;
use alef::core::backend::{Backend, TraitBridgeRegistrationSurface};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

const TRAIT: &str = "SamplePlugin";
const REGISTER_FN: &str = "install_sample_plugin";
const UNREGISTER_FN: &str = "remove_sample_plugin";
const CLEAR_FN: &str = "clear_sample_plugins";

fn plugin_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample-core".to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![TypeDef {
            name: TRAIT.to_owned(),
            rust_path: format!("sample_core::{TRAIT}"),
            is_trait: true,
            methods: vec![MethodDef {
                name: "handle".to_owned(),
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                error_type: Some("Error".to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn plugin_bridge() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: TRAIT.to_owned(),
        registry_getter: Some("sample_core::plugins::registry::get_sample_plugin_registry".to_owned()),
        register_fn: Some(REGISTER_FN.to_owned()),
        unregister_fn: Some(UNREGISTER_FN.to_owned()),
        clear_fn: Some(CLEAR_FN.to_owned()),
        ..Default::default()
    }
}

fn config_with_bridge(toml: &str) -> ResolvedCrateConfig {
    let parsed: NewAlefConfig = toml::from_str(toml).expect("fixture config must parse");
    let mut config = parsed.resolve().expect("fixture config must resolve").remove(0);
    config.trait_bridges = vec![plugin_bridge()];
    config
}

fn only_surface(backend: &dyn Backend, config: &ResolvedCrateConfig) -> TraitBridgeRegistrationSurface {
    let mut surfaces = backend.trait_bridge_registration_surface(&plugin_api(), config);
    assert_eq!(
        surfaces.len(),
        1,
        "{}: one configured trait bridge must report exactly one surface, got {surfaces:?}",
        backend.name()
    );
    surfaces.remove(0)
}

fn generated_text(backend: &dyn Backend, config: &ResolvedCrateConfig) -> String {
    backend
        .generate_bindings(&plugin_api(), config)
        .unwrap_or_else(|error| panic!("{}: generation failed: {error}", backend.name()))
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// PHP and Elixir emit their consumer-facing wrapper from `generate_public_api`, not from
/// `generate_bindings`, so the declaration lookup has to read that output instead.
fn generated_public_api_text(backend: &dyn Backend, config: &ResolvedCrateConfig) -> String {
    backend
        .generate_public_api(&plugin_api(), config)
        .unwrap_or_else(|error| panic!("{}: public API generation failed: {error}", backend.name()))
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_declares(backend: &dyn Backend, generated: &str, declaration: &str) {
    assert!(
        generated.contains(declaration),
        "{}: the reported registration surface names a symbol the generated output does not \
         declare -- expected to find `{declaration}`",
        backend.name()
    );
}

fn minimal_config(language: &str, extra: &str) -> ResolvedCrateConfig {
    config_with_bridge(&format!(
        "[workspace]\nlanguages = [\"{language}\"]\n\n\
         [[crates]]\nname = \"sample-core\"\nsources = [\"src/lib.rs\"]\n\n\
         [crates.{language}]\n{extra}"
    ))
}

#[test]
fn gleam_surface_names_the_trait_derived_register_shim_and_the_verbatim_rest() {
    let config = minimal_config("gleam", "");
    let surface = only_surface(&GleamBackend, &config);

    // Gleam derives only `register_*` from the trait; the configured `install_sample_plugin`
    // names the Erlang NIF the shim binds to, never the Gleam function.
    assert_eq!(surface.register_symbol.as_deref(), Some("register_sample_plugin"));
    assert_eq!(surface.unregister_symbol.as_deref(), Some(UNREGISTER_FN));
    assert_eq!(surface.clear_symbol.as_deref(), Some(CLEAR_FN));

    let generated = generated_text(&GleamBackend, &config);
    assert_declares(&GleamBackend, &generated, "pub fn register_sample_plugin(pid: Dynamic");
    assert_declares(&GleamBackend, &generated, "pub fn remove_sample_plugin(name: String)");
    assert_declares(&GleamBackend, &generated, "pub fn clear_sample_plugins()");
}

#[test]
fn dart_surface_names_the_lower_camel_methods_on_the_bridge_class() {
    let config = minimal_config("dart", "");
    let surface = only_surface(&DartBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SampleCoreBridge.installSamplePlugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SampleCoreBridge.removeSamplePlugin")
    );
    assert_eq!(
        surface.clear_symbol.as_deref(),
        Some("SampleCoreBridge.clearSamplePlugins")
    );

    let generated = generated_text(&DartBackend, &config);
    assert_declares(&DartBackend, &generated, "class SampleCoreBridge");
    for method in ["installSamplePlugin", "removeSamplePlugin", "clearSamplePlugins"] {
        assert_declares(&DartBackend, &generated, method);
    }
}

#[test]
fn swift_surface_names_the_top_level_forwarder_functions() {
    let config = minimal_config("swift", "");
    let surface = only_surface(&SwiftBackend, &config);

    assert_eq!(surface.register_symbol.as_deref(), Some("installSamplePlugin"));
    assert_eq!(surface.unregister_symbol.as_deref(), Some("removeSamplePlugin"));
    assert_eq!(surface.clear_symbol.as_deref(), Some("clearSamplePlugins"));

    let generated = generated_text(&SwiftBackend, &config);
    assert_declares(&SwiftBackend, &generated, "public func installSamplePlugin(");
    assert_declares(&SwiftBackend, &generated, "public func removeSamplePlugin(");
    assert_declares(&SwiftBackend, &generated, "public func clearSamplePlugins(");
}

#[test]
fn swift_reports_nothing_for_an_options_field_bridge() {
    let mut config = minimal_config("swift", "");
    config.trait_bridges[0].bind_via = alef::core::config::BridgeBinding::OptionsField;
    config.trait_bridges[0].options_type = Some("SampleOptions".to_owned());
    config.trait_bridges[0].options_field = Some("plugin".to_owned());

    let surfaces = SwiftBackend.trait_bridge_registration_surface(&plugin_api(), &config);

    assert!(
        surfaces.is_empty(),
        "an options_field bridge hands the host a handle instead of a registry, and the \
         forwarder emitter skips it; got {surfaces:?}"
    );
}

#[test]
fn napi_surface_names_the_camel_case_module_exports() {
    let config = minimal_config("node", "");
    let surface = only_surface(&NapiBackend, &config);

    assert_eq!(surface.register_symbol.as_deref(), Some("installSamplePlugin"));
    assert_eq!(surface.unregister_symbol.as_deref(), Some("removeSamplePlugin"));
    assert_eq!(surface.clear_symbol.as_deref(), Some("clearSamplePlugins"));

    let generated = generated_text(&NapiBackend, &config);
    // The Rust item keeps the configured name; napi-rs exports it under the camel form.
    assert_declares(&NapiBackend, &generated, &format!("pub fn {REGISTER_FN}("));
    assert_declares(&NapiBackend, &generated, "js_name = \"removeSamplePlugin\"");
    assert_declares(&NapiBackend, &generated, "js_name = \"clearSamplePlugins\"");
}

#[test]
fn napi_reports_no_register_symbol_without_a_registry_getter() {
    let mut config = minimal_config("node", "");
    config.trait_bridges[0].registry_getter = None;

    let surface = only_surface(&NapiBackend, &config);

    assert_eq!(
        surface.register_symbol, None,
        "gen_registration_fn emits nothing without a registry_getter, so no JS export exists"
    );
    assert_eq!(surface.unregister_symbol.as_deref(), Some("removeSamplePlugin"));
}

#[test]
fn napi_emits_no_bridge_and_reports_no_surface_when_the_target_is_excluded() {
    for excluded in ["node", "napi"] {
        let mut config = minimal_config("node", "");
        config.trait_bridges[0].exclude_languages = vec![excluded.to_owned()];

        let surfaces = NapiBackend.trait_bridge_registration_surface(&plugin_api(), &config);
        let generated = generated_text(&NapiBackend, &config);

        assert_eq!(
            surfaces.len(),
            0,
            "`exclude_languages = [\"{excluded}\"]` suppresses the `#[napi]` items, so nothing is \
             left to document; got {surfaces:?}"
        );
        assert!(
            !generated.contains(REGISTER_FN),
            "`exclude_languages = [\"{excluded}\"]` must suppress the registration item too"
        );
        assert!(
            !generated.contains("JsSamplePluginBridge"),
            "`exclude_languages = [\"{excluded}\"]` must suppress the bridge wrapper struct"
        );
    }
}

#[test]
fn wasm_surface_names_the_js_names_stamped_on_the_wasm_bindgen_exports() {
    let config = minimal_config("wasm", "");
    let surface = only_surface(&WasmBackend, &config);

    assert_eq!(surface.register_symbol.as_deref(), Some("installSamplePlugin"));
    assert_eq!(surface.unregister_symbol.as_deref(), Some("removeSamplePlugin"));
    assert_eq!(surface.clear_symbol.as_deref(), Some("clearSamplePlugins"));

    let generated = generated_text(&WasmBackend, &config);
    for js_name in ["installSamplePlugin", "removeSamplePlugin", "clearSamplePlugins"] {
        assert_declares(&WasmBackend, &generated, &format!("js_name = \"{js_name}\""));
    }
}

#[test]
fn wasm_emits_no_bridge_and_reports_no_surface_when_the_target_is_excluded() {
    let mut config = minimal_config("wasm", "");
    config.trait_bridges[0].exclude_languages = vec!["wasm".to_owned()];

    let surfaces = WasmBackend.trait_bridge_registration_surface(&plugin_api(), &config);
    let generated = generated_text(&WasmBackend, &config);

    assert_eq!(
        surfaces.len(),
        0,
        "`exclude_languages = [\"wasm\"]` suppresses the `#[wasm_bindgen]` items, so nothing is \
         left to document; got {surfaces:?}"
    );
    for js_name in ["installSamplePlugin", "removeSamplePlugin", "clearSamplePlugins"] {
        assert!(
            !generated.contains(js_name),
            "`exclude_languages = [\"wasm\"]` must suppress the `{js_name}` export too"
        );
    }
    assert!(
        !generated.contains("WasmSamplePluginBridge"),
        "`exclude_languages = [\"wasm\"]` must suppress the bridge wrapper struct"
    );
}

#[test]
fn magnus_surface_names_the_module_functions_bound_under_the_configured_names() {
    let config = minimal_config("ruby", "");
    let surface = only_surface(&MagnusBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SampleCore.install_sample_plugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SampleCore.remove_sample_plugin")
    );
    assert_eq!(surface.clear_symbol.as_deref(), Some("SampleCore.clear_sample_plugins"));

    let generated = generated_text(&MagnusBackend, &config);
    assert_declares(&MagnusBackend, &generated, "define_module(\"SampleCore\")");
    for ruby_name in [REGISTER_FN, UNREGISTER_FN, CLEAR_FN] {
        assert_declares(
            &MagnusBackend,
            &generated,
            &format!("define_module_function(\"{ruby_name}\""),
        );
    }
}

#[test]
fn php_surface_names_the_static_methods_on_the_public_wrapper_class() {
    let config = minimal_config("php", "");
    let surface = only_surface(&PhpBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SampleCore::installSamplePlugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SampleCore::removeSamplePlugin")
    );
    assert_eq!(surface.clear_symbol.as_deref(), Some("SampleCore::clearSamplePlugins"));

    let generated = generated_public_api_text(&PhpBackend, &config);
    assert_declares(&PhpBackend, &generated, "class SampleCore");
    for method in ["installSamplePlugin", "removeSamplePlugin", "clearSamplePlugins"] {
        assert_declares(&PhpBackend, &generated, &format!("function {method}("));
    }
}

#[test]
fn rustler_surface_names_the_elixir_delegates_on_the_app_module() {
    let config = minimal_config("elixir", "");
    let surface = only_surface(&RustlerBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SampleCore.install_sample_plugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SampleCore.remove_sample_plugin")
    );
    assert_eq!(surface.clear_symbol.as_deref(), Some("SampleCore.clear_sample_plugins"));

    let generated = generated_public_api_text(&RustlerBackend, &config);
    assert_declares(&RustlerBackend, &generated, "defmodule SampleCore do");
    for func in [REGISTER_FN, UNREGISTER_FN] {
        assert_declares(&RustlerBackend, &generated, &format!("def {func}("));
    }
    // The clear delegate takes no arguments, and Elixir spells a zero-arity `def` without
    // parentheses -- `def clear_sample_plugins do`. ~keep
    assert_declares(&RustlerBackend, &generated, &format!("def {CLEAR_FN} do"));
}

#[test]
fn rustler_reports_nothing_when_the_bridge_excludes_either_spelling_of_the_target() {
    for excluded in ["elixir", "rustler"] {
        let mut config = minimal_config("elixir", "");
        config.trait_bridges[0].exclude_languages = vec![excluded.to_owned()];

        let surfaces = RustlerBackend.trait_bridge_registration_surface(&plugin_api(), &config);

        assert!(
            surfaces.is_empty(),
            "`exclude_languages = [\"{excluded}\"]` suppresses the Elixir delegates, so nothing \
             is left to document; got {surfaces:?}"
        );
    }
}

#[test]
fn kotlin_jvm_reports_no_registration_surface_because_it_emits_none() {
    let config = minimal_config("kotlin", "package = \"io.sample.core\"");

    let surfaces = KotlinBackend.trait_bridge_registration_surface(&plugin_api(), &config);

    assert!(
        surfaces.is_empty(),
        "`generate_jvm` emits no register/unregister/clear function of its own -- a Kotlin/JVM \
         consumer calls the generated Java bridge class directly -- so there is no Kotlin symbol \
         to document; got {surfaces:?}"
    );
}

#[test]
fn jni_reports_no_registration_surface_because_its_shims_are_not_a_host_api() {
    // `jni` is not standalone: config resolution rejects it unless `kotlin_android` is also
    // enabled, because the shims it emits exist for that target to link against. ~keep
    let config = config_with_bridge(
        "[workspace]\nlanguages = [\"jni\", \"kotlin_android\"]\n\n\
         [[crates]]\nname = \"sample-core\"\nsources = [\"src/lib.rs\"]\n\n\
         [crates.jni]\n",
    );

    let surfaces = JniBackend.trait_bridge_registration_surface(&plugin_api(), &config);

    assert!(
        surfaces.is_empty(),
        "the JNI backend emits `Java_..._nativeRegister*` ABI shims that the Kotlin/Java side \
         links against, not an API a consumer calls; got {surfaces:?}"
    );
}

#[test]
fn extendr_surface_names_the_verbatim_r_functions() {
    let config = minimal_config("r", "");
    let surface = only_surface(&ExtendrBackend, &config);

    assert_eq!(surface.register_symbol.as_deref(), Some(REGISTER_FN));
    assert_eq!(surface.unregister_symbol.as_deref(), Some(UNREGISTER_FN));
    assert_eq!(surface.clear_symbol.as_deref(), Some(CLEAR_FN));

    let generated = generated_text(&ExtendrBackend, &config);
    for r_fn in [REGISTER_FN, UNREGISTER_FN, CLEAR_FN] {
        assert_declares(&ExtendrBackend, &generated, &format!("pub fn {r_fn}("));
    }
}

#[test]
fn extendr_reports_no_register_symbol_without_a_registry_getter() {
    let mut config = minimal_config("r", "");
    config.trait_bridges[0].registry_getter = None;

    let surface = only_surface(&ExtendrBackend, &config);

    assert_eq!(
        surface.register_symbol, None,
        "gen_registration_fn emits nothing without a registry_getter, so R receives no such \
         function"
    );
    assert_eq!(surface.unregister_symbol.as_deref(), Some(UNREGISTER_FN));
    assert_eq!(surface.clear_symbol.as_deref(), Some(CLEAR_FN));
}

#[test]
fn extendr_reports_nothing_when_the_bridge_excludes_either_spelling_of_the_target() {
    for excluded in ["r", "extendr"] {
        let mut config = minimal_config("r", "");
        config.trait_bridges[0].exclude_languages = vec![excluded.to_owned()];

        let surfaces = ExtendrBackend.trait_bridge_registration_surface(&plugin_api(), &config);

        assert!(
            surfaces.is_empty(),
            "`exclude_languages = [\"{excluded}\"]` suppresses the `#[extendr]` items, so \
             nothing is left to document; got {surfaces:?}"
        );
    }
}
