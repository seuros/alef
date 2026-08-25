//! End-to-end regression for alef#368: a real `napi build` invocation, in a fixture with
//! *both* a workspace-root `package.json` and a differently-named crate-local one, must bake
//! the crate-local name into the generated loader -- never the root's.
//!
//! A fixture with only one `package.json` cannot distinguish this fix from the defect it
//! fixes: napi-rs's default (`<cwd>/package.json`) and the crate-local file would name the
//! same package either way. Two differently-named manifests are the whole point. This test
//! runs the exact command [`build_command_for`] emits -- not a hand-rolled equivalent -- so a
//! regression in either the command string or napi-rs's own resolution behavior is caught.
//!
//! Requires `npx` (and network access, on first run, to fetch `@napi-rs/cli` and the `napi`/
//! `napi-derive` crates) and a working `cargo`. Skips rather than fails when `npx` is not on
//! `PATH`, matching this repo's convention for tests that depend on an external toolchain
//! (see e.g. `snippets::validators::typescript`).

use super::*;
use crate::core::backend::{BuildConfig, BuildDependency};
use std::io::Write as _;
use std::path::Path;

fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("fixture path must have a parent")).expect("create fixture dir");
    let mut file = std::fs::File::create(path).expect("create fixture file");
    file.write_all(contents.as_bytes()).expect("write fixture file");
}

#[test]
fn napi_build_bakes_the_crate_local_package_name_not_the_workspace_roots() {
    if which::which("npx").is_err() {
        return;
    }

    let project = tempfile::tempdir().expect("create tempdir");
    let root = project.path();

    // The workspace-root package.json a real consumer monorepo has -- deliberately named
    // differently from the binding crate's own, so a loader that accidentally reads this one
    // instead is unambiguous.
    write_file(&root.join("package.json"), r#"{"name":"workspace-root-pkg","version":"0.0.0","private":true}"#);

    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    )
    .expect("parse fixture alef.toml");
    let config = alef_cfg.resolve().expect("resolve fixture config").remove(0);
    let build_config = BuildConfig {
        tool: "napi",
        crate_suffix: "-node",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };
    let command = build_command_for(Language::Node, &build_config, &config, false);

    let crate_dir = root.join("crates/mylib-node");
    write_file(
        &crate_dir.join("Cargo.toml"),
        "[package]\nname = \"mylib-node\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = \
         [\"cdylib\"]\n\n[dependencies]\nnapi = { version = \"3\", default-features = false, features = \
         [\"napi9\"] }\nnapi-derive = \"3\"\n\n[build-dependencies]\nnapi-build = \"2\"\n",
    );
    write_file(&crate_dir.join("build.rs"), "fn main() {\n    napi_build::setup();\n}\n");
    write_file(
        &crate_dir.join("src/lib.rs"),
        "#[macro_use]\nextern crate napi_derive;\n\n#[napi]\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    );
    // The crate-local package.json a real Node binding crate ships -- named differently from
    // the workspace root above. `napi.targets` only needs to be non-empty for the loader's
    // per-platform optional-dependency requires to be generated at all.
    write_file(
        &crate_dir.join("package.json"),
        r#"{"name":"crate-local-pkg","version":"0.1.0","napi":{"binaryName":"mylib","targets":["x86_64-apple-darwin","aarch64-apple-darwin","x86_64-unknown-linux-gnu"]}}"#,
    );

    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(root)
        .status()
        .expect("spawn napi build");
    assert!(status.success(), "napi build must succeed: {command}");

    let loader = std::fs::read_to_string(crate_dir.join("index.js")).expect("read generated loader");
    assert!(
        loader.contains("crate-local-pkg-"),
        "the generated loader must require optional-dependency packages named after the \
         crate-local package.json, not the workspace root's"
    );
    assert!(
        !loader.contains("workspace-root-pkg-"),
        "the generated loader must never bake in the workspace-root package.json's name -- \
         doing so requires packages that were never published, exactly the alef#368 defect"
    );
}
