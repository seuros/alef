//! Swift snippets must not hand an optional to `print`/`debugPrint` uncast.
//!
//! Both take `Any...`, and Swift warns on every implicit optional-to-`Any` coercion:
//! `expression implicitly coerced from 'RustString?' to 'Any'`. The snippet validator compiles
//! with `-warnings-as-errors` (`snippets::validators::swift`), so that warning is a hard failure —
//! it is why three crawlberg Swift snippets failed while the same snippets passed in every other
//! language. This is a display concern only: the accessor chain the snippet builds is correct, and
//! `?.` is already inserted where the chain needs it.
//!
//! Two shapes produce an optional, and the fix has to cover both:
//! - an optional *link* — `markdown` is `Option<Markdown>`, so `result.markdown()?.content()` is a
//!   `RustString?` even though `content` itself is not optional;
//! - an optional *leaf* — `final_url` is `Option<String>`, and the renderer emits no `?` on a leaf,
//!   so `result.finalUrl()` looks total but is a `RustString?`.
//!
//! The non-optional expressions in the same snippet are the negative control: an indiscriminate
//! "always cast" change compiles just as well but makes every documentation snippet noisier, so
//! this file fails it.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

/// A fixture that shows an optional link, an optional leaf and a total leaf, and iterates a
/// collection whose items carry one optional and one total field.
fn presentation_fixture() -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "markdown_basic",
        "description": "Convert a page to markdown",
        "input": null,
        "docs": {"topic": "markdown", "presentation": {"operations": [
            {"op": "show", "path": "markdown.content"},
            {"op": "show", "path": "final_url"},
            {"op": "show", "path": "status_code"},
            {"op": "iterate", "path": "links", "item": "link", "fields": ["url", "title"]}
        ]}}
    }))
    .expect("the presentation fixture must parse")
}

fn presentation_config() -> E2eConfig {
    let mut e2e = E2eConfig::default();
    e2e.call.function = "scrape".into();
    e2e.result_fields = ["markdown", "final_url", "status_code", "links"]
        .into_iter()
        .map(String::from)
        .collect();
    e2e.fields_optional = ["markdown", "final_url", "title"]
        .into_iter()
        .map(String::from)
        .collect();
    e2e.fields_array = ["links"].into_iter().map(String::from).collect();
    e2e
}

fn render_snippet() -> String {
    super::snippet::render(
        &presentation_fixture(),
        &presentation_config(),
        &ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        },
        &[],
        &[],
    )
    .expect("the Swift snippet must render")
}

#[test]
fn an_optional_link_in_the_chain_is_cast_before_it_reaches_debug_print() {
    let rendered = render_snippet();
    assert!(
        rendered.contains("debugPrint(result.markdown()?.content() as Any)"),
        "an optional-chained accessor must be cast to Any before printing; got:\n{rendered}"
    );
}

#[test]
fn an_optional_leaf_is_cast_even_though_the_accessor_carries_no_question_mark() {
    let rendered = render_snippet();
    assert!(
        rendered.contains("debugPrint(result.finalUrl() as Any)"),
        "an optional leaf accessor must be cast to Any before printing; got:\n{rendered}"
    );
}

#[test]
fn an_optional_field_of_an_iterated_item_is_cast_before_it_reaches_debug_print() {
    let rendered = render_snippet();
    assert!(
        rendered.contains("debugPrint(link.title() as Any)"),
        "an optional field read inside the iteration body must be cast to Any; got:\n{rendered}"
    );
}

/// Negative control: a total expression must stay uncast, or the cast is being applied blindly
/// and proves nothing about the optional cases above.
#[test]
fn a_total_expression_is_printed_without_a_cast() {
    let rendered = render_snippet();
    assert!(
        rendered.contains("debugPrint(result.statusCode())\n"),
        "a non-optional accessor must be printed uncast; got:\n{rendered}"
    );
    assert!(
        rendered.contains("debugPrint(link.url())\n"),
        "a non-optional field of an iterated item must be printed uncast; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("result.statusCode() as Any"),
        "the cast must be driven by optionality, not applied to every printed value; got:\n{rendered}"
    );
}

/// The whole-result fallback prints the bound `result`, which is never optional on the path that
/// emits it (`expects_error` and `returns_void` both suppress it), so it must stay uncast.
#[test]
fn the_whole_result_fallback_is_printed_without_a_cast() {
    let fixture = Fixture {
        id: "count".into(),
        description: "Count".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "count_items".into();

    let rendered = super::snippet::render(
        &fixture,
        &e2e,
        &ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        },
        &[],
        &[],
    )
    .expect("the Swift snippet must render");

    assert!(rendered.contains("print(result)"), "{rendered}");
    assert!(!rendered.contains("print(result as Any)"), "{rendered}");
}
