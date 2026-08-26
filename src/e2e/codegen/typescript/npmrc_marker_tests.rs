//! Regression coverage for alef task #477: the `.npmrc` `E2eCodegen::generate` emits for the
//! node/napi e2e suite uses `generated_header: false`, so `alef`'s ownership tracking depends
//! entirely on `hash::content_has_alef_marker` finding a marker in the rendered content itself.
//! A prior version hand-spelled `"; alef-generated ..."`, which that guard does not recognize,
//! permanently stranding `.npmrc` as unowned/unadoptable.
//!
//! Split into its own file rather than grown inline in `typescript/mod.rs`: that file is
//! already at the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md). ~keep

use super::*;
use crate::core::config::NewAlefConfig;

fn generated_npmrc_content() -> String {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"
"#,
    )
    .expect("valid toml");
    let mut e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    e2e.dep_mode = crate::e2e::config::DependencyMode::Registry;
    let resolved = cfg.resolve().expect("resolve ok").remove(0);
    let codegen = TypeScriptCodegen;
    let files = codegen
        .generate(&[], &e2e, &resolved, &[], &[], &[], &[])
        .expect("generate ok");
    files
        .iter()
        .find(|f| f.path.ends_with(".npmrc"))
        .expect("node codegen must emit .npmrc")
        .content
        .clone()
}

/// Asserts through the real guard function (`hash::content_has_alef_marker`) rather than a
/// hand-copied literal, so this fails if the emitter and the guard ever disagree again.
#[test]
fn npmrc_marker_is_recognised_by_the_real_ownership_guard() {
    let content = generated_npmrc_content();
    assert!(
        crate::core::hash::content_has_alef_marker(&content),
        ".npmrc must carry a marker the real alef ownership guard recognises, got: {content}"
    );
}

/// Negative control: content with no ownership marker at all must still be reported unowned,
/// proving the guard above is not vacuously true.
#[test]
fn content_with_no_marker_is_not_recognised_by_the_ownership_guard() {
    let content = "; just a plain hand-written npmrc\nfrozen-lockfile=false\n";
    assert!(
        !crate::core::hash::content_has_alef_marker(content),
        "content with no alef marker must not be recognised as alef-owned, got: {content}"
    );
}
