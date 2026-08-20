//! On a stock scaffold (no `[crates.output]` entries) the scaffolder and `alef generate`
//! held three different opinions about where a binding crate lives:
//!
//! 1. The scaffolders write a manifest at `crates/{core_crate_dir}-{suffix}` for Python,
//!    Node, PHP, and FFI (`src/scaffold/languages/{python,node,php,ffi}.rs`).
//! 2. `alef generate` wrote generated sources under whatever `[crates.output]` resolved
//!    to, which for an unconfigured language fell back to `packages/{lang}` --
//!    `OutputTemplate::resolve`'s unconfigured single-crate default.
//! 3. wasm has no scaffolded `Cargo.toml` at all -- its manifest is written by
//!    `WasmBackend::generate_bindings` itself, from the same resolved output path as (2).
//!
//! `crates/{core_crate_dir}-{suffix}` and `packages/{lang}` disagree, so cargo could not
//! find the library the scaffolded manifest declared. This test pins the fix: it runs the
//! *actual* scaffolder and the *actual* output-path resolution used by `alef generate`, and
//! asserts the crate root each one lands on agrees. Neither side is a hard-coded path
//! literal, so a future edit that reintroduces the disagreement fails this test regardless
//! of which side moved.

use alef::core::backend::GeneratedFile;
use alef::core::config::{Language, NewAlefConfig, OutputLayout, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};
use alef::scaffold::scaffold;
use std::path::PathBuf;

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

/// A stock scaffold over every language this regression covers, with no `[crates.output]`
/// table at all -- the exact shape that reproduced the CI gate failure.
fn config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python", "node", "php", "ffi", "wasm"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test"
license = "MIT"
repository = "https://example.invalid/toolkit"
authors = ["Test Author"]
"#,
    )
    .expect("fixture config parses");
    cfg.resolve().expect("fixture config resolves").remove(0)
}

/// The crate root implied by the scaffolder's *real* output: the directory holding the
/// scaffolded file named `marker_file` whose parent directory ends with `dir_suffix` --
/// distinguishing e.g. `crates/toolkit-node/Cargo.toml` from `crates/toolkit-php/Cargo.toml`
/// among every scaffolded file, not just matching the first `Cargo.toml` found.
fn scaffolded_crate_root(files: &[GeneratedFile], marker_file: &str, dir_suffix: &str) -> PathBuf {
    files
        .iter()
        .find(|file| {
            file.path.file_name().is_some_and(|name| name == marker_file)
                && file
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .is_some_and(|dir| dir.to_string_lossy().ends_with(dir_suffix))
        })
        .unwrap_or_else(|| {
            panic!(
                "no scaffolded `{marker_file}` under a `*{dir_suffix}` directory among: {:?}",
                files.iter().map(|f| &f.path).collect::<Vec<_>>()
            )
        })
        .path
        .parent()
        .unwrap_or_else(|| panic!("`{marker_file}` has no parent directory"))
        .to_path_buf()
}

/// The crate root `alef generate` resolves for a language with no `[crates.output]`
/// override: read from the same `output_paths` map every backend's `output_for` /
/// `resolve_output_dir` call reads, split with the same `OutputLayout` the backends use
/// to separate a crate root from its `src` directory.
fn generated_crate_root(config: &ResolvedCrateConfig, lang: &str) -> PathBuf {
    let output_dir = config
        .output_for(lang)
        .unwrap_or_else(|| panic!("no resolved output path for {lang}"));
    OutputLayout::from_output_dir(&output_dir.to_string_lossy()).root
}

#[test]
fn scaffolder_and_generate_agree_on_every_binding_crate_root() {
    let api = api();
    let config = config();
    let languages = [
        Language::Python,
        Language::Node,
        Language::Php,
        Language::Ffi,
        Language::Wasm,
    ];
    let files = scaffold(&api, &config, &languages).expect("scaffold succeeds over the fixture");

    // wasm has no scaffolded Cargo.toml (WasmBackend writes its own), so its manifest-bearing
    // marker is the npm `package.json` the scaffolder does write into the same crate root.
    let cases: &[(&str, &str, &str)] = &[
        ("python", "Cargo.toml", "-py"),
        ("node", "Cargo.toml", "-node"),
        ("php", "Cargo.toml", "-php"),
        ("ffi", "Cargo.toml", "-ffi"),
        ("wasm", "package.json", "-wasm"),
    ];

    for (lang, marker_file, dir_suffix) in cases {
        let scaffolded_root = scaffolded_crate_root(&files, marker_file, dir_suffix);
        let generated_root = generated_crate_root(&config, lang);
        assert_eq!(
            scaffolded_root,
            generated_root,
            "{lang}: the scaffolder wrote `{marker_file}` into `{}`, but `alef generate`'s \
             default output path resolves to a crate rooted at `{}` -- the scaffolded \
             manifest and the generated sources must share one crate root, or cargo cannot \
             find the library the manifest declares",
            scaffolded_root.display(),
            generated_root.display(),
        );
    }
}
