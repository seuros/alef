//! Coverage for `[crates.custom_modules] ffi = [...]` — the mechanism that lets a
//! hand-written sibling file under the FFI crate's `src/` directory be reached
//! from the generated `lib.rs` (`pub mod <name>;`), so its `extern "C"` functions
//! are compiled into the cdylib and seen by cbindgen instead of sitting orphaned
//! on disk.

use super::super::{FfiBackend, validate_custom_modules_exist};
use super::common::*;
use crate::core::backend::Backend;

fn write_module_file(src_dir: &std::path::Path, module: &str) {
    std::fs::write(src_dir.join(format!("{module}.rs")), "// hand-written FFI module\n").expect("write module file");
}

fn config_with_custom_modules(src_dir: &std::path::Path, modules: &[&str]) -> crate::core::config::ResolvedCrateConfig {
    let module_list = modules.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(", ");
    resolved_one(&format!(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.output]
ffi = "{src_dir}"

[crates.custom_modules]
ffi = [{module_list}]
"#,
        src_dir = src_dir.display(),
    ))
}

/// `sample_config()` carries no `[crates.custom_modules]` table at all. An
/// explicit `ffi = []` on the same base config must byte-for-byte match its
/// output — the empty-list path in `gen_lib_rs`'s `for module in custom_mods`
/// loop must be a true no-op, not merely "produces no visible `pub mod`".
///
/// This does not regress against unfixed code: `custom_modules.ffi` already
/// existed and was already a no-op when empty before this change. It locks in
/// that pre-existing behavior now that it has test coverage at all.
#[test]
fn no_custom_modules_configured_leaves_lib_rs_byte_identical() {
    let api = sample_api();
    let baseline_files = FfiBackend.generate_bindings(&api, &sample_config()).expect("baseline generation");
    let baseline_lib = &baseline_files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;

    let directory = tempfile::tempdir().expect("temporary directory");
    let explicit_empty = config_with_custom_modules(directory.path(), &[]);
    let files = FfiBackend.generate_bindings(&api, &explicit_empty).expect("explicit-empty generation");
    let lib = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;

    assert_eq!(lib, baseline_lib);
    assert!(!lib.contains("pub mod "), "no custom module should be declared:\n{lib}");
}

/// One configured module, backed by a real file, must be declared verbatim
/// with `pub mod <name>;`, with no `pub use` re-export, ahead of every
/// generated item.
///
/// This does not regress against unfixed code either: `gen_lib_rs` already
/// emitted `pub mod {module};` for `custom_modules.ffi` entries before this
/// change (this test is new coverage for existing emission behavior, not a
/// behavior change). Only `custom_module_missing_file_fails_generation_with_checked_paths`
/// below exercises code this change actually adds.
#[test]
fn one_custom_module_declares_pub_mod_before_generated_items() {
    let directory = tempfile::tempdir().expect("temporary directory");
    write_module_file(directory.path(), "cancellation");
    let config = config_with_custom_modules(directory.path(), &["cancellation"]);

    let api = sample_api();
    let files = FfiBackend.generate_bindings(&api, &config).expect("generation with one custom module");
    let lib = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;

    assert!(lib.contains("pub mod cancellation;"), "missing declaration:\n{lib}");
    assert!(
        !lib.contains("pub use cancellation::"),
        "no re-export should be emitted for FFI custom modules:\n{lib}"
    );

    let mod_pos = lib.find("pub mod cancellation;").expect("declaration present");
    let first_generated_item_pos = lib.find("my_lib_last_error_code").expect("generated item present");
    assert!(
        mod_pos < first_generated_item_pos,
        "custom module declaration must precede generated items"
    );
}

/// Multiple configured modules must be declared in configured order, not
/// alphabetical order ("config" sorts before "config_builder", but the config
/// below lists them the other way around) and not any other derived order.
///
/// Same caveat as the single-module test above: the underlying `pub mod`
/// emission loop predates this change and already preserved config order:
/// this locks in that behavior with a test, it does not newly create it.
#[test]
fn multiple_custom_modules_declare_pub_mod_in_configured_order() {
    let directory = tempfile::tempdir().expect("temporary directory");
    write_module_file(directory.path(), "config_builder");
    write_module_file(directory.path(), "cancellation");
    let config_dir = directory.path().join("config");
    std::fs::create_dir_all(&config_dir).expect("create config/ dir");
    std::fs::write(config_dir.join("mod.rs"), "// hand-written FFI module\n").expect("write config/mod.rs");

    let config = config_with_custom_modules(directory.path(), &["config_builder", "cancellation", "config"]);

    let api = sample_api();
    let files = FfiBackend.generate_bindings(&api, &config).expect("generation with three custom modules");
    let lib = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;

    let config_builder_pos = lib.find("pub mod config_builder;").expect("config_builder declared");
    let cancellation_pos = lib.find("pub mod cancellation;").expect("cancellation declared");
    let config_pos = lib.find("pub mod config;").expect("config declared");

    assert!(
        config_builder_pos < cancellation_pos && cancellation_pos < config_pos,
        "modules must be declared in configured order (config_builder, cancellation, config), got positions \
         {config_builder_pos}, {cancellation_pos}, {config_pos} in:\n{lib}"
    );
}

/// A configured module with no matching file on disk must fail generation
/// with a message naming the crate, the module, and both candidate paths that
/// were checked — instead of silently emitting `pub mod <name>;` for a file
/// that does not exist, which rustc would only catch later as `E0583` against
/// the *generated* lib.rs, with no pointer back to the config key at fault.
///
/// This test goes red against unfixed code: before this change,
/// `FfiBackend::generate_bindings` never touched the filesystem and always
/// returned `Ok`, so a missing module file would not surface as an error
/// until a downstream `cargo build` of the generated crate ran.
#[test]
fn custom_module_missing_file_fails_generation_with_checked_paths() {
    let directory = tempfile::tempdir().expect("temporary directory");
    // Deliberately do not write cancellation.rs or cancellation/mod.rs.
    let config = config_with_custom_modules(directory.path(), &["cancellation"]);

    let error = FfiBackend
        .generate_bindings(&sample_api(), &config)
        .expect_err("missing custom module file must fail generation");
    let message = error.to_string();

    assert!(message.contains("my-lib"), "error must name the crate: {message}");
    assert!(message.contains("cancellation"), "error must name the module: {message}");
    let flat = directory.path().join("cancellation.rs");
    let nested = directory.path().join("cancellation").join("mod.rs");
    assert!(
        message.contains(&flat.display().to_string()),
        "error must name the checked flat-file path: {message}"
    );
    assert!(
        message.contains(&nested.display().to_string()),
        "error must name the checked nested-file path: {message}"
    );
}

/// Direct unit coverage of the validation helper, independent of the rest of
/// `generate_bindings`: an empty module list is always `Ok`, a module list is
/// resolved relative to the given `output_dir`, and either the flat
/// `<name>.rs` form or the nested `<name>/mod.rs` form satisfies it.
#[test]
fn validate_custom_modules_exist_accepts_flat_or_nested_file_and_empty_list() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let config = sample_config();

    assert!(validate_custom_modules_exist(&config, &directory.path().to_string_lossy(), &[]).is_ok());

    write_module_file(directory.path(), "flat_module");
    assert!(
        validate_custom_modules_exist(
            &config,
            &directory.path().to_string_lossy(),
            &["flat_module".to_string()],
        )
        .is_ok()
    );

    let nested_dir = directory.path().join("nested_module");
    std::fs::create_dir_all(&nested_dir).expect("create nested module dir");
    std::fs::write(nested_dir.join("mod.rs"), "// hand-written\n").expect("write nested mod.rs");
    assert!(
        validate_custom_modules_exist(
            &config,
            &directory.path().to_string_lossy(),
            &["nested_module".to_string()],
        )
        .is_ok()
    );

    let missing = validate_custom_modules_exist(&config, &directory.path().to_string_lossy(), &["absent".to_string()]);
    assert!(missing.is_err());
}
