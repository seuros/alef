//! Names a docs snippet's loop bindings so they cannot collide with what the snippet has already
//! bound where the loop is emitted.
//!
//! A fixture's `iterate` operation carries the loop variable's name verbatim, and nothing stopped
//! an author from picking the name the call's result is already bound to. Every generator then
//! rendered the collection accessor off the result and the loop binding off the same word:
//!
//! ```text
//! for (const result of result.results ?? [])   // TS2448 / TS7022
//! for (var result : result.results())          // error: variable result is already defined
//! ```
//!
//! Rust, Python and Go accept that (their loop binding shadows from the body onward, and the
//! iterated expression is evaluated in the enclosing scope), which is why the name reached a
//! published fixture at all — the snippet reads fine in the languages that permit shadowing and
//! does not compile in the ones that do not. The name is a generated-code decision either way, so
//! it is decided here, once, and applied by rewriting the *operation* before any generator
//! resolves it: the per-item field accessors are rendered from `item` as their root, so renaming
//! afterwards would mean rewriting rendered expressions instead of naming them right the first
//! time. ~keep

use std::borrow::Cow;
use std::collections::BTreeSet;

use crate::e2e::fixture::{Fixture, FixtureDocsOperation};

/// The loop-binding name used when the fixture's own choice is unavailable.
///
/// A plain noun rather than a decorated form of the author's word (`resultItem`, `result2`): it
/// needs no case conversion to read idiomatically in either a camelCase or a snake_case target,
/// and this module deliberately owns no naming policy. It is also the name the fixture schema's
/// own `iterate` example uses. ~keep
const FALLBACK_ITEM_NAME: &str = "item";

/// `fixture` with every `iterate` operation whose loop binding would collide with one of
/// `bound_names` renamed, or the fixture untouched when none would.
///
/// `bound_names` is what the caller's emitted snippet has in scope where the loop lands — at
/// minimum the call's result variable, which is the collision that actually shipped. A caller that
/// binds more names (a destructured element, a client handle) passes those too.
pub(crate) fn without_shadowed_loop_bindings<'a>(fixture: &'a Fixture, bound_names: &[&str]) -> Cow<'a, Fixture> {
    let taken: BTreeSet<&str> = bound_names.iter().copied().collect();
    if !shadows_any(fixture, &taken) {
        return Cow::Borrowed(fixture);
    }
    let mut renamed = fixture.clone();
    if let Some(presentation) = renamed.docs.as_mut().and_then(|docs| docs.presentation.as_mut()) {
        for operation in &mut presentation.operations {
            if let FixtureDocsOperation::Iterate { item, .. } = operation
                && taken.contains(item.as_str())
            {
                *item = unshadowed_name(&taken);
            }
        }
    }
    Cow::Owned(renamed)
}

fn shadows_any(fixture: &Fixture, taken: &BTreeSet<&str>) -> bool {
    fixture
        .docs
        .as_ref()
        .and_then(|docs| docs.presentation.as_ref())
        .is_some_and(|presentation| {
            presentation.operations.iter().any(|operation| {
                matches!(operation, FixtureDocsOperation::Iterate { item, .. } if taken.contains(item.as_str()))
            })
        })
}

/// [`FALLBACK_ITEM_NAME`], or the first numbered variant of it that is free.
///
/// Sibling loops may share a name — each `for` opens its own scope in both targets — so only the
/// caller's bound names are avoided, and the common case never reaches a suffix.
fn unshadowed_name(taken: &BTreeSet<&str>) -> String {
    if !taken.contains(FALLBACK_ITEM_NAME) {
        return FALLBACK_ITEM_NAME.to_string();
    }
    (2..)
        .map(|suffix| format!("{FALLBACK_ITEM_NAME}{suffix}"))
        .find(|candidate| !taken.contains(candidate.as_str()))
        .expect("an unbounded sequence of candidate names always yields a free one")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::fixture::{FixtureDocs, FixtureDocsPresentation, SideEffectClass};

    fn iterate_fixture(items: &[&str]) -> Fixture {
        Fixture {
            id: "list_entries".into(),
            description: "List entries".into(),
            docs: Some(FixtureDocs {
                topic: "guides".into(),
                stem: None,
                paths: Default::default(),
                title: None,
                description: None,
                input: None,
                shows: Vec::new(),
                error: None,
                presentation: Some(FixtureDocsPresentation {
                    call: None,
                    input: None,
                    args: None,
                    files: Vec::new(),
                    operations: items
                        .iter()
                        .map(|item| FixtureDocsOperation::Iterate {
                            path: "results".into(),
                            item: (*item).to_string(),
                            fields: vec!["text".into()],
                            display: false,
                            optional: true,
                        })
                        .collect(),
                }),
                client: None,
                side_effects: SideEffectClass::Safe,
                coverage_exceptions: Default::default(),
                sample_url_vars: Default::default(),
            }),
            ..Fixture::default()
        }
    }

    fn item_names(fixture: &Fixture) -> Vec<String> {
        fixture
            .docs
            .as_ref()
            .and_then(|docs| docs.presentation.as_ref())
            .map(|presentation| {
                presentation
                    .operations
                    .iter()
                    .map(|operation| match operation {
                        FixtureDocsOperation::Iterate { item, .. } => item.clone(),
                        FixtureDocsOperation::Show { path, .. } => path.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn renames_a_loop_binding_that_shadows_the_result_variable() {
        let fixture = iterate_fixture(&["result"]);
        let renamed = without_shadowed_loop_bindings(&fixture, &["result"]);
        assert_eq!(item_names(&renamed), vec!["item".to_string()]);
    }

    #[test]
    fn leaves_a_loop_binding_that_collides_with_nothing_alone() {
        let fixture = iterate_fixture(&["entry"]);
        let renamed = without_shadowed_loop_bindings(&fixture, &["result"]);
        assert!(matches!(renamed, Cow::Borrowed(_)), "no collision must not clone");
        assert_eq!(item_names(&renamed), vec!["entry".to_string()]);
    }

    #[test]
    fn skips_a_fallback_name_that_is_itself_bound() {
        let fixture = iterate_fixture(&["item"]);
        let renamed = without_shadowed_loop_bindings(&fixture, &["item"]);
        assert_eq!(item_names(&renamed), vec!["item2".to_string()]);
    }

    #[test]
    fn renames_only_the_operations_that_collide() {
        let fixture = iterate_fixture(&["entry", "result"]);
        let renamed = without_shadowed_loop_bindings(&fixture, &["result"]);
        assert_eq!(item_names(&renamed), vec!["entry".to_string(), "item".to_string()]);
    }
}
