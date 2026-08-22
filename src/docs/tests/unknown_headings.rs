//! Rustdoc headings alef does not recognise must still nest under the section that contains them.
//!
//! `clean_doc` rewrites a handful of conventional rustdoc section headings (`# Errors`,
//! `# Returns`, `# Panics`, …) to bold labels. Every *other* heading survives as a real markdown
//! heading and has to be re-levelled to sit under the generated heading it was spliced beneath.
//! Two separate re-levelling bugs let such a heading escape above its own parent — in the worst
//! case as a bare `#` H1 inside a page rooted at `##`, which trips rumdl MD001/MD025 and turns
//! `poly lint` red in the consumer repo:
//!
//! - the language pages anchored the doc's *first* heading at the target level, so a doc opening
//!   at a level that was already deep enough was passed through untouched;
//! - the shared pages (`types.md`, `errors.md`) shifted by a *fixed* number of levels, which is
//!   only correct for a doc that opens at `#`.
//!
//! The fixture headings are deliberately ones alef has no special handling for: the guarantee
//! under test is about *unrecognised* headings in general, not about growing the recognised set.
//!
//! Note on MD001: heading *increment* violations can also originate in the source doc's own
//! ordering (a doc that opens with `##### Detail` before `# Overview` is malformed however it is
//! re-levelled, since fixing it would mean reordering the author's prose). These tests therefore
//! assert the property alef owns — no heading escapes its section — rather than MD001 on
//! arbitrary input. `headings.rs` covers MD001 for well-formed docs. ~keep

use super::*;
use crate::docs::test_helpers::{make_function, make_test_config};

/// The common rustdoc shape: every section heading at `#`, one of them unrecognised.
const UNIFORM_HEADINGS: &str = "Does a thing.\n\n# Returns\n\nA value.\n\n# Observability\n\nEmits a metric.\n";

/// A doc whose first heading is deeper than the level its section nests at, followed by a
/// shallower unrecognised heading. The first heading being deep is what used to disable
/// re-levelling for the whole doc. ~keep
const DEEP_FIRST_HEADING: &str = "Does a thing.\n\n##### Deep First\n\nText.\n\n# Observability\n\nMore text.\n";

fn api_with_doc(doc: &str) -> ApiSurface {
    let mut func = make_function(
        "compute_total",
        vec![],
        TypeRef::Primitive(PrimitiveType::U32),
        false,
        None,
    );
    func.doc = doc.to_string();

    let mut ty = empty_type("Widget");
    ty.doc = doc.to_string();

    let mut api = crate::docs::test_helpers::empty_api();
    api.crate_name = "mylib".to_string();
    api.functions = vec![func];
    api.types = vec![ty];
    api
}

/// Heading level of `line`, or `None` if it is not an ATX heading.
fn heading_level(line: &str) -> Option<usize> {
    if !line.starts_with('#') {
        return None;
    }
    let level = line.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&level).then_some(level)
}

fn generated_pages(doc: &str) -> Vec<crate::core::GeneratedFile> {
    let api = api_with_doc(doc);
    let config = make_test_config();
    generate_docs(&api, &config, &[Language::Rust], "out").unwrap()
}

/// True for headings alef emits itself (page titles and item names), as opposed to headings that
/// came out of a doc comment.
fn is_alef_heading(title: &str) -> bool {
    title.ends_with("()")
        || title == "Widget"
        || title.starts_with("Rust API Reference")
        || matches!(
            title,
            "Functions" | "Types" | "Other Types" | "Types Reference" | "Error Reference" | "Configuration Reference"
        )
}

#[test]
fn should_not_emit_top_level_heading_for_unrecognised_rustdoc_section() {
    for doc in [UNIFORM_HEADINGS, DEEP_FIRST_HEADING] {
        for file in generated_pages(doc) {
            let offenders: Vec<&str> = file
                .content
                .lines()
                .filter(|line| heading_level(line) == Some(1))
                .collect();
            assert!(
                offenders.is_empty(),
                "{}: every generated page is rooted at `##`, so an H1 can only come from a rustdoc \
                 heading that escaped re-levelling (MD025): {offenders:?}\n{}",
                file.path.display(),
                file.content
            );
        }
    }
}

#[test]
fn should_nest_unrecognised_rustdoc_heading_under_its_own_section() {
    for doc in [UNIFORM_HEADINGS, DEEP_FIRST_HEADING] {
        for file in generated_pages(doc) {
            // Walk the page tracking the level of the last heading alef emitted itself. Every
            // heading that came out of a doc comment must be strictly deeper than the alef
            // heading it was rendered under. ~keep
            let mut section_level: Option<usize> = None;
            for line in file.content.lines() {
                let Some(level) = heading_level(line) else {
                    continue;
                };
                let title = line.trim_start_matches('#').trim();
                if is_alef_heading(title) {
                    section_level = Some(level);
                    continue;
                }

                let parent = section_level.expect("a doc heading always follows an alef-emitted heading");
                assert!(
                    level > parent,
                    "{}: doc heading {line:?} sits at H{level} under an H{parent} section — it reads \
                     as a sibling of its own parent",
                    file.path.display()
                );
            }
        }
    }
}
