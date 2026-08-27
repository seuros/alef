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

fn generated_npmrc_file() -> crate::core::backend::GeneratedFile {
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
        .into_iter()
        .find(|f| f.path.ends_with(".npmrc"))
        .expect("node codegen must emit .npmrc")
}

fn generated_npmrc_content() -> String {
    generated_npmrc_file().content
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

/// Regression for alef task #509: `alef adopt` classified an already-marked `.npmrc` as
/// unstampable and fell back to recording it in the presence-only `.alef-ownership.toml`
/// bucket, even though this generator proves the format markable by marking it itself.
///
/// Asserted generically -- "whatever path this generator self-marks must be a path
/// `cli::pipeline::generate::write`'s marker table can also stamp" -- rather than hardcoding
/// `.npmrc`'s extension, so the test still catches the drift if a future generator change moves
/// the marker to a different self-marked path this table has not been taught about. If `.npmrc`
/// is ever dropped from `write::marker_header_syntax`, this fails with "generator marks a path
/// adopt cannot stamp"; that is the exact symptom `alef adopt --write` reproduced before this
/// task's fix, where the file was recorded as owned instead of ever having its own bytes
/// stamped.
#[test]
fn every_path_the_generator_self_marks_is_stampable_by_adopt() {
    let file = generated_npmrc_file();
    assert!(
        crate::core::hash::content_has_alef_marker(&file.content),
        "precondition: the generator must actually self-mark this path for the test below to \
         mean anything, got: {}",
        file.content
    );
    assert!(
        crate::cli::pipeline::is_markable_path(&file.path),
        "generator marks a path adopt cannot stamp: {} carries an alef marker in its generated \
         content, but `write::marker_header_syntax` has no comment syntax registered for it -- \
         `alef adopt --write` would record this path in `.alef-ownership.toml` instead of \
         stamping the marker onto the bytes on disk",
        file.path.display()
    );
}
