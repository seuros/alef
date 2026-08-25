//! Regression for the "two paths disagree" defect: `alef docs`/`alef all` used to run a
//! pre-flight, purely-static gate (the deleted `build_dependency::enforce_build_dependency`)
//! ahead of `run_validation` that bailed under `strict` whenever a language had no configured
//! `docs.snippets.sessions.<target>.before` step -- regardless of whether the language's own
//! validator needed one. `alef snippets check` never ran that gate at all, and a consumer's real
//! `compile`-level validators (which build the snippet from source themselves) reported clean
//! passes for languages the gate had already condemned as "no build guarantee" before a single
//! toolchain ran.
//!
//! `RustValidator` is exactly such a self-building validator: with no session configured
//! (`session: None`), `validate_batch_with_context` compiles the snippet in an isolated scratch
//! crate via `cargo check`, no external `before` step required (see
//! `validators::rust::RustValidator::validate_batch_with_context`). A plain Rust snippet with
//! zero `docs.snippets.sessions` configuration must therefore validate cleanly at `compile`
//! level, strict or not -- the deleted gate used to bail on it anyway, purely from the absence of
//! a `before` command in static config. ~keep

use super::*;
use std::fs;

#[test]
fn strict_compile_validation_does_not_bail_without_a_configured_before_step() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("docs/snippets/rust")).unwrap();
    fs::write(
        root.join("docs/snippets/rust/example.md"),
        "```rust\nfn add(left: i32, right: i32) -> i32 {\n    left + right\n}\n```\n",
    )
    .unwrap();

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python"]

[workspace.docs.snippets]
dirs = ["docs/snippets"]
validation_level = "compile"
strict = true

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let api = make_minimal_api("1.0.0");

    let (_files, result) = generate_docs_stage(&api, &config, &[Language::Python], None, root);

    result.expect(
        "a language with no configured `before` build step must not fail strict validation when \
         its own validator compiles the snippet standalone -- the removed pre-flight gate used to \
         bail here from static session config alone, without ever checking what the validator \
         actually does",
    );
}
