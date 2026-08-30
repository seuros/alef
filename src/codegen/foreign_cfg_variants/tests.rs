use super::*;
use crate::codegen::conversions::ConversionConfig;
use crate::core::config::NewAlefConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

const HOST_CRATE: &str = "test-lib";
const HOST_IMPORT: &str = "test_lib";
const GATED_VARIANT: &str = "WebSocket";
const GATE: &str = r#"feature = "ws""#;

/// Every language whose backend re-emits Rust and therefore drops a foreign cfg-gated variant
/// through a different generator. Each one used to contribute its own WARN for the single
/// variant below. ~keep
const FAN_OUT_LANGUAGES: [Language; 4] = [Language::Php, Language::Elixir, Language::Wasm, Language::Node];

fn config_for(languages: &[Language]) -> ResolvedCrateConfig {
    let language_list = languages
        .iter()
        .map(|language| format!("\"{language}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml_src = format!(
        "[workspace]\nlanguages = [{language_list}]\n[[crates]]\nname = \"{HOST_CRATE}\"\n\
         sources = [\"src/lib.rs\"]\n"
    );
    let config: NewAlefConfig = toml::from_str(&toml_src).expect("fixture alef.toml must parse");
    config.resolve().expect("fixture alef.toml must resolve").remove(0)
}

fn config_with_features(language_features: &[(Language, &[&str])]) -> ResolvedCrateConfig {
    let languages = language_features
        .iter()
        .map(|(language, _)| format!("\"{language}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let feature_tables = language_features
        .iter()
        .map(|(language, features)| {
            let features = features
                .iter()
                .map(|feature| format!("\"{feature}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[crates.{language}]\nfeatures = [{features}]\n")
        })
        .collect::<String>();
    let toml_src = format!(
        "[workspace]\nlanguages = [{languages}]\n[[crates]]\nname = \"{HOST_CRATE}\"\n\
         sources = [\"src/lib.rs\"]\n{feature_tables}"
    );
    let config: NewAlefConfig = toml::from_str(&toml_src).expect("fixture alef.toml must parse");
    config.resolve().expect("fixture alef.toml must resolve").remove(0)
}

fn gated_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: Some(GATE.to_string()),
        ..Default::default()
    }
}

fn gated_variant_with_cfg(name: &str, cfg: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: Some(cfg.to_string()),
        ..Default::default()
    }
}

/// A first path segment that is not the host crate's own import name is exactly what
/// `is_host_owned_rust_path` reads to classify this enum -- and its cfg-gated variant -- as
/// FOREIGN. `serde_tag` plus the data-carrying `Http` variant routes it through the tagged /
/// flat-data-enum generators the four backends above use. ~keep
fn foreign_enum(variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: "Transport".to_string(),
        rust_path: "dep_crate::Transport".to_string(),
        serde_tag: Some("type".to_string()),
        variants,
        ..Default::default()
    }
}

fn http_variant() -> EnumVariant {
    EnumVariant {
        name: "Http".to_string(),
        fields: vec![FieldDef {
            name: "url".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn surface(enum_def: EnumDef) -> ApiSurface {
    ApiSurface {
        crate_name: HOST_CRATE.to_string(),
        version: "0.1.0".to_string(),
        enums: vec![enum_def],
        ..Default::default()
    }
}

/// Drive every generator a real run would drive for this enum: the two shared conversion
/// directions (napi, magnus, rustler and wasm all route through these), the visitor-result
/// lowering, and four whole backends through the public `Backend` trait.
fn run_every_generator(api: &ApiSurface, config: &ResolvedCrateConfig) {
    let enum_def = &api.enums[0];
    let conversion_config = ConversionConfig::default();
    let _ = crate::codegen::conversions::gen_enum_from_core_to_binding_cfg(enum_def, HOST_IMPORT, &conversion_config);
    let _ = crate::codegen::conversions::gen_enum_from_binding_to_core_cfg(enum_def, HOST_IMPORT, &conversion_config);
    let _ = crate::codegen::visitor_result::visitor_result_metadata_from_enum_checked(enum_def, "Visitor", HOST_IMPORT);
    for language in FAN_OUT_LANGUAGES {
        let backend = crate::cli::registry::get_backend(language);
        let _ = backend.generate_bindings(api, config);
    }
}

fn count_lines_at(lines: &[&str], level: &str) -> usize {
    lines
        .iter()
        .filter(|line| line.contains(level) && line.contains(GATED_VARIANT))
        .count()
}

/// THE REGRESSION. One foreign cfg-gated variant used to produce one WARN per (backend,
/// direction, generator) that dropped it, so a consumer using `[[crates.source_crates]]` saw the
/// same fact restated on every clean regen, scaling with the backend count. Only the fan-out was
/// wrong: the detail still has to reach anyone debugging codegen, so it moved to DEBUG rather
/// than disappearing.
///
/// The DEBUG floor is what keeps this test from passing vacuously. A run that generated nothing
/// -- a fixture the generators never classified as foreign, a backend that errored before
/// reaching its enum pass -- would emit zero WARN too, and would be indistinguishable from a real
/// pass on the WARN assertion alone. ~keep
#[tracing_test::traced_test]
#[test]
fn generators_do_not_repeat_the_same_dropped_variant_once_per_backend_and_direction() {
    let api = surface(foreign_enum(vec![http_variant(), gated_variant(GATED_VARIANT)]));
    let config = config_for(&FAN_OUT_LANGUAGES);

    run_every_generator(&api, &config);

    logs_assert(|lines: &[&str]| {
        let warnings = count_lines_at(lines, "WARN");
        let details = count_lines_at(lines, "DEBUG");
        if warnings > 0 {
            return Err(format!(
                "one dropped variant must not be warned about once per generator: got {warnings} \
                 WARN lines naming {GATED_VARIANT}, alongside {details} DEBUG lines"
            ));
        }
        if details < 3 {
            return Err(format!(
                "the generators never reached their cfg-gated-variant pass, so this test proves \
                 nothing: expected at least 3 DEBUG lines naming {GATED_VARIANT}, got {details}"
            ));
        }
        Ok(())
    });
}

/// The other half of the same invariant: removing the fan-out must not remove the signal. The
/// consumer still gets exactly one actionable WARN naming the variant, from the run-level pass.
#[tracing_test::traced_test]
#[test]
fn the_run_level_pass_reports_a_dropped_variant_exactly_once() {
    let api = surface(foreign_enum(vec![http_variant(), gated_variant(GATED_VARIANT)]));
    let language_features = FAN_OUT_LANGUAGES.map(|language| (language, &["ws"][..]));
    let config = config_with_features(&language_features);

    warn_foreign_cfg_gated_variants(&api, &config, &FAN_OUT_LANGUAGES);
    run_every_generator(&api, &config);

    logs_assert(|lines: &[&str]| match count_lines_at(lines, "WARN") {
        1 => Ok(()),
        other => Err(format!(
            "the whole run must report the dropped variant exactly once, got {other}: {lines:#?}"
        )),
    });
}

/// Deduplication keys on the variant, not on "has this run warned at all" -- a pass that
/// collapsed to one warning per run would silence every variant after the first.
#[tracing_test::traced_test]
#[test]
fn two_distinct_dropped_variants_are_each_reported() {
    let api = surface(foreign_enum(vec![
        http_variant(),
        gated_variant(GATED_VARIANT),
        gated_variant("Quic"),
    ]));

    let language_features = FAN_OUT_LANGUAGES.map(|language| (language, &["ws"][..]));
    warn_foreign_cfg_gated_variants(&api, &config_with_features(&language_features), &FAN_OUT_LANGUAGES);

    logs_assert(|lines: &[&str]| {
        let websocket = count_lines_at(lines, "WARN");
        let quic = lines.iter().filter(|line| line.contains("Quic")).count();
        if websocket == 1 && quic == 1 {
            Ok(())
        } else {
            Err(format!(
                "each dropped variant needs its own warning, got {websocket} for {GATED_VARIANT} \
                 and {quic} for Quic"
            ))
        }
    });
}

/// A host-owned cfg-gated variant keeps its arm and its `#[cfg(...)]` -- forwarding already
/// declared that feature -- so there is nothing to report. Without this control the pass would
/// look correct while warning about every gated variant in the surface.
#[tracing_test::traced_test]
#[test]
fn a_host_owned_cfg_gated_variant_is_not_reported() {
    let mut enum_def = foreign_enum(vec![http_variant(), gated_variant(GATED_VARIANT)]);
    enum_def.rust_path = format!("{HOST_IMPORT}::Transport");

    warn_foreign_cfg_gated_variants(&surface(enum_def), &config_for(&FAN_OUT_LANGUAGES), &FAN_OUT_LANGUAGES);

    assert!(
        !logs_contain(GATED_VARIANT),
        "a cfg gate the generated crate's own [features] table forwards is valid, not a drop"
    );
}

/// An ungated foreign variant is dropped by nobody, so the pass must stay silent about it even
/// though its enum is foreign-owned.
#[tracing_test::traced_test]
#[test]
fn an_ungated_foreign_variant_is_not_reported() {
    let api = surface(foreign_enum(vec![http_variant()]));

    warn_foreign_cfg_gated_variants(&api, &config_for(&FAN_OUT_LANGUAGES), &FAN_OUT_LANGUAGES);

    assert!(
        !logs_contain("Http"),
        "only a cfg gate this crate cannot declare forces a variant to be dropped"
    );
}

/// Dart and Swift's Rust-bridge generators classify ownership against the plain crate name, every
/// other backend against `core_import`. When a `[crate] core_import` facade makes those two
/// spellings disagree, the enum is host-owned for one set of backends and foreign for the other,
/// so the run-level warning's universal claim ("dropped from every generated binding") is false
/// and must not be made. The per-backend DEBUG lines still carry it. ~keep
#[tracing_test::traced_test]
#[test]
fn a_variant_only_some_backends_drop_is_left_to_the_per_backend_detail() {
    let toml_src = format!(
        "[workspace]\nlanguages = [\"swift\", \"php\"]\n[[crates]]\nname = \"{HOST_CRATE}\"\n\
         sources = [\"src/lib.rs\"]\ncore_import = \"facade\"\n"
    );
    let parsed: NewAlefConfig = toml::from_str(&toml_src).expect("fixture alef.toml must parse");
    let config = parsed.resolve().expect("fixture alef.toml must resolve").remove(0);
    let mut enum_def = foreign_enum(vec![http_variant(), gated_variant(GATED_VARIANT)]);
    // Host-owned to Swift (which asks against `test_lib`), foreign to PHP (which asks against
    // the `facade` core_import).
    enum_def.rust_path = format!("{HOST_IMPORT}::Transport");

    warn_foreign_cfg_gated_variants(&surface(enum_def), &config, &[Language::Swift, Language::Php]);

    assert!(
        !logs_contain(GATED_VARIANT),
        "a drop that is not universal across the run's backends must not be claimed as universal"
    );
}

#[tracing_test::traced_test]
#[test]
fn a_test_or_testkit_variant_is_not_reported_when_every_language_enables_only_full() {
    let languages = [Language::Php, Language::Node];
    let config = config_with_features(&[(Language::Php, &["full"]), (Language::Node, &["full"])]);
    let variant = gated_variant_with_cfg(GATED_VARIANT, r#"any(test, feature = "testkit")"#);

    warn_foreign_cfg_gated_variants(&surface(foreign_enum(vec![variant])), &config, &languages);

    assert!(
        !logs_contain(GATED_VARIANT),
        "the canonical evaluator proves any(test, testkit) unreachable with only full enabled"
    );
}

#[tracing_test::traced_test]
#[test]
fn a_variant_enabled_for_the_requested_language_is_reported() {
    let languages = [Language::Php];
    let config = config_with_features(&[(Language::Php, &["testkit"])]);
    let variant = gated_variant_with_cfg(GATED_VARIANT, r#"feature = "testkit""#);

    warn_foreign_cfg_gated_variants(&surface(foreign_enum(vec![variant])), &config, &languages);

    assert!(
        logs_contain(GATED_VARIANT),
        "an enabled foreign variant is still dropped and actionable"
    );
}

#[tracing_test::traced_test]
#[test]
fn a_variant_enabled_for_any_requested_language_is_reported() {
    let languages = [Language::Php, Language::Node];
    let config = config_with_features(&[(Language::Php, &["testkit"]), (Language::Node, &["full"])]);
    let variant = gated_variant_with_cfg(GATED_VARIANT, r#"any(test, feature = "testkit")"#);

    warn_foreign_cfg_gated_variants(&surface(foreign_enum(vec![variant])), &config, &languages);

    assert!(
        logs_contain(GATED_VARIANT),
        "one enabled language makes the foreign drop actionable for the run"
    );
}

#[tracing_test::traced_test]
#[test]
fn a_variant_with_an_unknown_target_predicate_is_reported() {
    let languages = [Language::Php];
    let config = config_with_features(&[(Language::Php, &[])]);
    let variant = gated_variant_with_cfg(GATED_VARIANT, r#"target_os = "windows""#);

    warn_foreign_cfg_gated_variants(&surface(foreign_enum(vec![variant])), &config, &languages);

    assert!(
        logs_contain(GATED_VARIANT),
        "an indeterminate target predicate must keep the conservative warning"
    );
}

#[test]
fn host_crate_spellings_follow_each_backends_own_derivation() {
    let toml_src = format!(
        "[workspace]\nlanguages = [\"swift\", \"php\"]\n[[crates]]\nname = \"{HOST_CRATE}\"\n\
         sources = [\"src/lib.rs\"]\ncore_import = \"facade\"\n"
    );
    let parsed: NewAlefConfig = toml::from_str(&toml_src).expect("fixture alef.toml must parse");
    let config = parsed.resolve().expect("fixture alef.toml must resolve").remove(0);
    let api = surface(foreign_enum(vec![http_variant()]));

    let spellings = host_crate_spellings(&api, &config, &[Language::Swift, Language::Php]);

    assert_eq!(
        spellings,
        BTreeSet::from([HOST_IMPORT.to_string(), "facade".to_string()]),
        "Swift's bridge asks against the crate name, PHP's against core_import"
    );
}

#[test]
fn generated_docs_do_not_advertise_a_variant_every_backend_drops() {
    let mut api = surface(foreign_enum(vec![http_variant(), gated_variant(GATED_VARIANT)]));
    api.types.push(TypeDef {
        name: "ClientOptions".to_string(),
        rust_path: format!("{HOST_IMPORT}::ClientOptions"),
        fields: vec![FieldDef {
            name: "transport".to_string(),
            ty: TypeRef::Named("Transport".to_string()),
            doc: "Which transport to use.\n- `Transport::Http` is always available.\n- `Transport::WebSocket` requires the dependency feature.\n- `AdvancedTransport::WebSocket` remains available."
                .to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });

    let projected =
        project_docs_without_unreachable_foreign_variants(&api, &config_for(&FAN_OUT_LANGUAGES), &FAN_OUT_LANGUAGES)
            .expect("project generated docs");
    let doc = &projected.types[0].fields[0].doc;

    assert!(
        doc.contains("Transport::Http"),
        "reachable variant docs must survive: {doc}"
    );
    assert!(
        !doc.lines()
            .any(|line| line == "- `Transport::WebSocket` requires the dependency feature."),
        "generated docs must not advertise a variant omitted from every binding: {doc}"
    );
    assert!(
        doc.contains("AdvancedTransport::WebSocket"),
        "a longer, unrelated enum name must not be treated as the dropped reference: {doc}"
    );
    assert!(
        api.types[0].fields[0].doc.contains("Transport::WebSocket"),
        "the extracted source IR must remain unchanged"
    );
}
