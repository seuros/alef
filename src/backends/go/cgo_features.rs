//! Feature macros the generated cgo preambles must define before including the C header.

use crate::codegen::c_consumer;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

/// The `-D` flags every generated cgo preamble must carry alongside its `#include` of the FFI
/// header, or an empty string when the cdylib enables no feature at all.
///
/// The header cbindgen writes wraps each `#[cfg(feature = "x")]` export in
/// `#if defined({PREFIX_UPPER}_FEATURE_X)` — the mapping `gen_cbindgen_toml` writes into
/// cbindgen's `[defines]` table — and it is produced once from the *unfiltered* API surface, so
/// every gated declaration is guarded no matter how the library was built. cgo compiles that
/// header with no `-D` at all, which makes every guard false and deletes those declarations,
/// while the Go glue calls them unconditionally: the package fails with `could not determine what
/// C.<symbol> refers to` even though the linked library exports the symbol. The guards themselves
/// are load-bearing for C consumers who link a feature-reduced build, so the preprocessor
/// configuration — not the guards — is what has to be fixed. ~keep
pub(crate) fn cgo_feature_cflags(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    cgo_feature_macros(api, config)
        .iter()
        .map(|macro_name| format!("-D{macro_name}=1"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The header guard macros for the features the linked cdylib actually enables, sorted.
pub(crate) fn cgo_feature_macros(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<String> {
    let ffi_prefix = config.ffi_prefix();
    let mut macros: Vec<String> = ffi_default_features(api, config)
        .iter()
        .map(|feature| guard_macro_name(&ffi_prefix, feature))
        .collect();
    macros.sort_unstable();
    macros.dedup();
    macros
}

/// The header guard macro cbindgen wraps a `#[cfg(feature = "{feature}")]` export in, for an FFI
/// crate whose symbol prefix is `ffi_prefix`.
pub(crate) fn guard_macro_name(ffi_prefix: &str, feature: &str) -> String {
    feature_macro_name(&c_consumer::export_type_prefix(ffi_prefix), feature)
}

/// The FFI crate's `[features] default` list — the feature set the shipped cdylib is built with.
///
/// The macros must describe the *library*, not this binding's own configured feature list. The
/// two are routinely different: `features_for_language(Language::Go)` decides which gated call
/// sites `with_cfg_filtered_deep` emits, but the cdylib is built once by `cargo build -p
/// {ffi_crate}` with no `--features` override, so its exported symbol set is exactly this
/// `default` list. Deriving the `-D` set from the Go list instead would define macros for
/// features the library was never built with, turning a clear compile-time
/// `could not determine what C.<symbol> refers to` into an opaque link failure; deriving it from
/// the library keeps a genuinely-disabled feature genuinely absent and reports any Go-vs-FFI
/// divergence at the exact symbol (`warn_on_ffi_feature_drift` warns about the same divergence
/// at config level).
///
/// Mirrors `scaffold::languages::ffi::scaffold_ffi`, which is private to the scaffolding module
/// and is what actually writes the `default` list into the FFI `Cargo.toml`. The mirror is
/// verified rather than trusted: `ffi_default_features_matches_the_generated_ffi_cargo_toml`
/// below drives the real scaffolder and compares, so a change to either derivation fails a test
/// instead of silently desynchronising the macro set from the library. ~keep
fn ffi_default_features(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<String> {
    let passthrough: Vec<&str> = config
        .features_for_language(Language::Ffi)
        .iter()
        .map(String::as_str)
        .filter(|feature| *feature != "serde")
        .collect();
    let extra_declared: &[String] = config.ffi.as_ref().map(|c| c.extra_features.as_slice()).unwrap_or(&[]);
    let emitted: Vec<String> = crate::codegen::cfg::collect_cfg_features(api)
        .into_iter()
        .filter(|name| {
            !name.is_empty()
                && name != "serde"
                && !passthrough.contains(&name.as_str())
                && !extra_declared.iter().any(|declared| declared == name)
        })
        .collect();
    passthrough
        .into_iter()
        .map(str::to_string)
        .chain(emitted)
        .collect::<Vec<_>>()
}

/// Mirrors `backends::ffi::gen_bindings::helpers::cbindgen_feature_defines`, which is private to
/// the FFI backend: the macro cbindgen puts in the header for `feature = "x"` is
/// `{PREFIX_UPPER}_FEATURE_{X}` with every non-alphanumeric character replaced by `_`. Spelling it
/// any other way produces a `-D` that no guard in the header ever tests, which fails silently. ~keep
fn feature_macro_name(prefix_upper: &str, feature: &str) -> String {
    let suffix: String = feature
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix_upper}_FEATURE_{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::backend::Backend as _;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::FunctionDef;

    fn resolved(toml: &str) -> ResolvedCrateConfig {
        let config: NewAlefConfig = toml::from_str(toml).unwrap();
        config.resolve().unwrap().remove(0)
    }

    fn api_with_gated_functions(gates: &[(&str, Option<&str>)]) -> ApiSurface {
        let mut api = ApiSurface {
            crate_name: "ts_pack".to_string(),
            version: "0.1.0".to_string(),
            ..ApiSurface::default()
        };
        api.functions = gates
            .iter()
            .map(|(name, cfg)| FunctionDef {
                name: (*name).to_string(),
                rust_path: format!("ts_pack::{name}"),
                cfg: cfg.map(str::to_string),
                ..FunctionDef::default()
            })
            .collect();
        api
    }

    fn config_toml(ffi_features: &str) -> String {
        format!(
            r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "ts-pack"
sources = ["src/lib.rs"]
features = ["full"]
[crates.ffi]
prefix = "ts_pack"
{ffi_features}
[crates.go]
module = "github.com/test/ts-pack"
"#
        )
    }

    /// The `-D` names must be the ones cbindgen is told to guard with, so this compares against
    /// the `[defines]` table the FFI backend actually emits rather than against a literal. A `-D`
    /// spelled any other way is inert: it defines a macro no `#if` in the header ever tests, and
    /// nothing fails until a consumer compiles. ~keep
    #[test]
    fn cgo_feature_macros_are_spelled_the_way_cbindgen_defines_them() {
        let config = resolved(&config_toml(""));
        let api = api_with_gated_functions(&[
            ("ping", None),
            ("download", Some(r#"feature = "download""#)),
            ("render", Some(r#"feature = "document-render""#)),
        ]);

        let files = crate::backends::ffi::FfiBackend
            .generate_bindings(&api, &config)
            .expect("generate the FFI backend files");
        let cbindgen = files
            .iter()
            .find(|file| file.path.to_string_lossy().ends_with("cbindgen.toml"))
            .expect("cbindgen.toml is generated");
        let parsed: toml::Value = toml::from_str(&cbindgen.content).expect("cbindgen.toml is valid TOML");
        let guard_macros: Vec<&str> = parsed["defines"]
            .as_table()
            .expect("[defines] is a table")
            .values()
            .filter_map(toml::Value::as_str)
            .collect();

        assert!(
            guard_macros.contains(&"TS_PACK_FEATURE_DOWNLOAD"),
            "control: cbindgen must be told to guard the gated export, got: {guard_macros:?}"
        );

        let emitted = cgo_feature_macros(&api, &config);
        let cfg_referenced = crate::codegen::cfg::collect_cfg_features(&api);
        assert!(
            !cfg_referenced.is_empty(),
            "control: the fixture must reference at least one feature from a cfg gate"
        );
        for feature in &cfg_referenced {
            let macro_name = feature_macro_name("TS_PACK", feature);
            assert!(
                guard_macros.contains(&macro_name.as_str()),
                "cbindgen guards `{feature}` with a macro other than {macro_name}; defines are {guard_macros:?}"
            );
            assert!(
                emitted.contains(&macro_name),
                "the cgo preamble never defines {macro_name}, so every declaration cbindgen guards \
                 with it is deleted before cgo sees it; emitted: {emitted:?}"
            );
        }
    }

    #[test]
    fn cgo_feature_cflags_covers_features_discovered_only_from_cfg_gates() {
        let config = resolved(&config_toml(""));
        let api = api_with_gated_functions(&[
            ("ping", None),
            ("download", Some(r#"feature = "download""#)),
            ("render", Some(r#"feature = "document-render""#)),
        ]);
        assert_eq!(
            cgo_feature_cflags(&api, &config),
            "-DTS_PACK_FEATURE_DOCUMENT_RENDER=1 -DTS_PACK_FEATURE_DOWNLOAD=1 -DTS_PACK_FEATURE_FULL=1"
        );
    }

    #[test]
    fn cgo_feature_macros_omit_declare_only_extra_features() {
        let config = resolved(&config_toml(r#"extra_features = ["wasm-http"]"#));
        let api = api_with_gated_functions(&[
            ("fetch_native", Some(r#"feature = "native-http""#)),
            ("fetch_wasm", Some(r#"feature = "wasm-http""#)),
        ]);
        let macros = cgo_feature_macros(&api, &config);
        assert!(
            macros.contains(&"TS_PACK_FEATURE_NATIVE_HTTP".to_string()),
            "auto-discovered cfg features default ON in the FFI crate, got: {macros:?}"
        );
        assert!(
            !macros.contains(&"TS_PACK_FEATURE_WASM_HTTP".to_string()),
            "`extra_features` are declare-only and stay OFF, so their guards must stay false: {macros:?}"
        );
    }

    /// The `-D` set describes the shipped cdylib, so it must equal the `[features] default` list
    /// the scaffolder writes into the FFI `Cargo.toml`. Driving the real scaffolder rather than
    /// asserting a literal is the point: this is the one place the two derivations of "what the
    /// library was built with" are compared, and it is what stops them drifting apart. ~keep
    #[test]
    fn ffi_default_features_matches_the_generated_ffi_cargo_toml() {
        let config = resolved(&config_toml(r#"extra_features = ["wasm-http"]"#));
        let api = api_with_gated_functions(&[
            ("ping", None),
            ("download", Some(r#"feature = "download""#)),
            ("render", Some(r#"feature = "document-render""#)),
            ("fetch_wasm", Some(r#"feature = "wasm-http""#)),
        ]);

        let files = crate::scaffold::scaffold(&api, &config, &[Language::Ffi]).expect("scaffold the FFI crate");
        let manifest = files
            .iter()
            .find(|file| file.path.to_string_lossy().ends_with("-ffi/Cargo.toml"))
            .expect("the FFI crate manifest is scaffolded");
        let parsed: toml::Value = toml::from_str(&manifest.content).expect("scaffolded manifest is valid TOML");
        let scaffolded: Vec<String> = parsed["features"]["default"]
            .as_array()
            .expect("[features] default is an array")
            .iter()
            .map(|value| value.as_str().expect("feature names are strings").to_string())
            .collect();

        assert!(
            scaffolded.contains(&"download".to_string()),
            "control: the scaffolder must default a cfg-discovered feature ON, got: {scaffolded:?}"
        );

        let mut derived = ffi_default_features(&api, &config);
        let mut expected = scaffolded;
        derived.sort();
        expected.sort();
        assert_eq!(
            derived, expected,
            "the cgo -D set is derived from a different feature list than the one the FFI crate is built with"
        );
    }
}
