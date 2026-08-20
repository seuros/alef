//! The WASM backend generates its own `Cargo.toml` instead of going through
//! `scaffold::scaffold`, so it is the one backend that can emit a cargo manifest into a
//! directory it does not own. It did: with no `[crates.output] wasm` entry the resolved
//! output path is crate-root-shaped (`packages/wasm`), the backend took its parent as the
//! crate root, and `packages/Cargo.toml` appeared beside every other language's package.
//! Cargo walks upward looking for a workspace, so the stray manifest broke the *sibling*
//! languages' builds — swift's was the one that went red first.

use alef::backends::wasm::WasmBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};
use std::path::{Path, PathBuf};

fn api() -> ApiSurface {
    ApiSurface {
        crate_name: "toolkit".to_string(),
        functions: vec![FunctionDef {
            name: "summarize".to_string(),
            rust_path: "toolkit::summarize".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn config_with_output(output: Option<&str>) -> ResolvedCrateConfig {
    let output_table = output.map_or_else(String::new, |path| format!("[crates.output]\nwasm = '{path}'\n"));
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]

[crates.wasm]
{output_table}"#
    ))
    .expect("fixture config parses");
    cfg.resolve().expect("fixture config resolves").remove(0)
}

fn find<'a>(files: &'a [GeneratedFile], file_name: &str) -> &'a Path {
    files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|name| name == file_name))
        .unwrap_or_else(|| panic!("no generated {file_name} in {:?}", paths(files)))
        .path
        .as_path()
}

fn paths(files: &[GeneratedFile]) -> Vec<PathBuf> {
    files.iter().map(|file| file.path.clone()).collect()
}

/// The property that actually failed: whatever shape the configured output path takes, the
/// generated manifest must sit at the root of a directory that contains every other file
/// the backend emits. A manifest anywhere else is a manifest in somebody else's tree.
#[test]
fn the_wasm_manifest_always_contains_the_sources_it_declares() {
    for output in [
        None,
        Some("crates/toolkit-wasm/src/"),
        Some("crates/toolkit-wasm/src"),
        Some("packages/wasm"),
    ] {
        let files = WasmBackend
            .generate_bindings(&api(), &config_with_output(output))
            .expect("wasm generation");

        let manifest = find(&files, "Cargo.toml").to_path_buf();
        let crate_root = manifest.parent().expect("manifest has a parent directory");

        for file in &files {
            assert!(
                file.path.starts_with(crate_root),
                "output {output:?}: `{}` is emitted outside the crate rooted at `{}`",
                file.path.display(),
                crate_root.display()
            );
        }

        // `gen_cargo_toml` emits no `[lib] path`, so cargo resolves the library target at
        // `<crate root>/src/lib.rs` and nowhere else. ~keep
        assert_eq!(
            find(&files, "lib.rs"),
            crate_root.join("src").join("lib.rs"),
            "output {output:?}: lib.rs must be where the emitted manifest declares it"
        );
    }
}

/// The four consumer repos all spell `[crates.output] wasm` as a `src`-suffixed path, so
/// that shape must keep resolving to exactly the paths it always did.
#[test]
fn a_src_suffixed_output_path_keeps_its_existing_layout() {
    let files = WasmBackend
        .generate_bindings(&api(), &config_with_output(Some("crates/toolkit-wasm/src/")))
        .expect("wasm generation");

    assert_eq!(find(&files, "Cargo.toml"), Path::new("crates/toolkit-wasm/Cargo.toml"));
    assert_eq!(find(&files, "lib.rs"), Path::new("crates/toolkit-wasm/src/lib.rs"));
}

/// The default, unconfigured case — the one the generated-output gate exercises and the
/// one that emitted `packages/Cargo.toml`.
#[test]
fn an_unconfigured_crate_emits_its_manifest_inside_its_own_package_directory() {
    let files = WasmBackend
        .generate_bindings(&api(), &config_with_output(None))
        .expect("wasm generation");

    assert_eq!(find(&files, "Cargo.toml"), Path::new("packages/wasm/Cargo.toml"));
    assert_eq!(find(&files, "lib.rs"), Path::new("packages/wasm/src/lib.rs"));
}
