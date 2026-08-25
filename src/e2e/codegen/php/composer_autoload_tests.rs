//! Where the generated `e2e/php/composer.json` resolves the binding namespace.
//!
//! Every assertion here parses the PSR-4 map out of a *rendered* `composer.json`, because the
//! defect this file exists to catch lived between an intermediate path value (which the older
//! `default_pkg_path_tests` asserted, and which was correct) and the string the renderer
//! actually emitted after appending its own `/src/` suffix.

use crate::core::backend::Backend;
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::codegen::php::PhpCodegen;
use crate::e2e::config::E2eConfig;

fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
    cfg.resolve().expect("resolve").remove(0)
}

/// The single PSR-4 target declared by a rendered `composer.json`.
fn psr4_target(composer_json: &str) -> String {
    let parsed: serde_json::Value = serde_json::from_str(composer_json)
        .unwrap_or_else(|e| panic!("composer.json must parse: {e}\n{composer_json}"));
    let map = parsed["autoload"]["psr-4"]
        .as_object()
        .unwrap_or_else(|| panic!("composer.json must declare autoload.psr-4:\n{composer_json}"));
    assert_eq!(map.len(), 1, "expected exactly one PSR-4 entry, got: {map:?}");
    map.values()
        .next()
        .and_then(serde_json::Value::as_str)
        .expect("PSR-4 target must be a string")
        .to_string()
}

/// The PSR-4 target of the `e2e/php/composer.json` alef generates for `config`.
fn e2e_autoload_target(config: &ResolvedCrateConfig) -> String {
    let files = PhpCodegen
        .generate(&[], &E2eConfig::default(), config, &[], &[], &[], &[])
        .expect("php e2e generation must succeed");
    let composer = files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "composer.json"))
        .expect("php e2e generation must emit composer.json");
    psr4_target(&composer.content)
}

/// The PSR-4 target of the repository-root `composer.json` alef scaffolds for `config`.
fn root_autoload_target(config: &ResolvedCrateConfig) -> String {
    let files =
        crate::scaffold::scaffold(&ApiSurface::default(), config, &[Language::Php]).expect("php scaffold must succeed");
    let root = files
        .iter()
        .find(|f| f.path == std::path::Path::new("composer.json"))
        .expect("php scaffold must emit a root composer.json");
    psr4_target(&root.content)
}

/// The directory `alef generate` actually writes the PHP userland classes into, read back off
/// the emitted file paths rather than re-derived from config.
fn generated_class_directory(config: &ResolvedCrateConfig) -> String {
    let files = crate::backends::php::PhpBackend
        .generate_public_api(&ApiSurface::default(), config)
        .expect("php public api generation must succeed");
    let class_file = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "php"))
        .expect("php public api generation must emit a class file");
    let dir = class_file
        .path
        .parent()
        .expect("a generated class file has a parent directory")
        .to_string_lossy()
        .into_owned();
    format!("{}/", dir.trim_end_matches('/'))
}

const CO_LOCATED_WITHOUT_SRC_SEGMENT: &str = r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
[crates.output]
php = "crates/my-lib-php"
"#;

const CO_LOCATED_WITH_SRC_SEGMENT: &str = r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
[crates.output]
php = "crates/my-lib-php/src/"
"#;

const UNCONFIGURED_OUTPUT: &str = r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
"#;

const PACKAGE_ROOTED_OUTPUT: &str = r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
[crates.output]
php = "packages/php"
"#;

const LAYOUTS: &[(&str, &str)] = &[
    ("co-located without a src segment", CO_LOCATED_WITHOUT_SRC_SEGMENT),
    ("co-located with a src segment", CO_LOCATED_WITH_SRC_SEGMENT),
    ("unconfigured output", UNCONFIGURED_OUTPUT),
    ("package-rooted output", PACKAGE_ROOTED_OUTPUT),
];

/// Compare the e2e PSR-4 target against `expected` for every layout, reporting all layouts that
/// disagree rather than aborting on the first — a per-layout defect is invisible when an earlier
/// layout already failed the assertion.
fn collect_mismatches(expected: impl Fn(&ResolvedCrateConfig) -> String) -> Vec<String> {
    LAYOUTS
        .iter()
        .filter_map(|(label, toml_text)| {
            let config = resolve_config(toml_text);
            let actual = e2e_autoload_target(&config);
            let want = expected(&config);
            (actual != want).then(|| format!("  {label}: e2e says {actual:?}, expected {want:?}"))
        })
        .collect()
}

/// The e2e autoload map must name the directory alef writes the classes into. It used to append
/// a fixed `/src/` to the resolved package root, so any layout whose output path did not already
/// end in `src` sent Composer to a directory no alef stage ever writes — which only resolves if
/// an unmanaged byte-identical copy of the class tree is kept alongside the managed one.
#[test]
fn e2e_autoload_target_is_the_directory_alef_generates_classes_into() {
    let mismatches = collect_mismatches(|config| format!("../../{}", generated_class_directory(config)));
    assert!(
        mismatches.is_empty(),
        "e2e composer.json must autoload the generated class directory:\n{}",
        mismatches.join("\n")
    );
}

/// The e2e project and the repository root resolve the same namespace, so their PSR-4 targets
/// must name one directory — the e2e copy differing only by the `e2e/php/` relative prefix.
#[test]
fn e2e_and_root_autoload_targets_name_the_same_directory() {
    let mismatches = collect_mismatches(|config| format!("../../{}", root_autoload_target(config)));
    assert!(
        mismatches.is_empty(),
        "e2e and root composer.json must not drift apart:\n{}",
        mismatches.join("\n")
    );
}

/// An explicit `[crates.e2e.packages.php] path` still wins — the derived default only applies
/// when the consumer has not named a package root themselves.
#[test]
fn explicit_e2e_package_path_overrides_the_derived_default() {
    let config = resolve_config(CO_LOCATED_WITHOUT_SRC_SEGMENT);
    let mut e2e_config = E2eConfig::default();
    e2e_config.packages.insert(
        "php".to_string(),
        crate::e2e::config::PackageRef {
            path: Some("../../vendor/hand-rolled".to_string()),
            ..Default::default()
        },
    );
    let files = PhpCodegen
        .generate(&[], &e2e_config, &config, &[], &[], &[], &[])
        .expect("php e2e generation must succeed");
    let composer = files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "composer.json"))
        .expect("php e2e generation must emit composer.json");
    assert_eq!(psr4_target(&composer.content), "../../vendor/hand-rolled/src/");
}
