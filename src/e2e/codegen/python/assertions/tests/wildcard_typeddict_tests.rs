//! Regression coverage for the Python wildcard element accessor's `TypedDict`-vs-attribute
//! classification, which the confirmed consumer defect showed was being answered at the wrong
//! level of a `container[].field` path.
//!
//! ~keep Registered from the sibling `wildcard_tests.rs` for the same file-size reason
//! `assertion_wildcard_element_tests.rs` is: `python/mod.rs` is at its recorded ceiling in
//! `tests/file_size_baseline.txt` and may not grow, and `assertions.rs` itself is close enough to
//! its own cap that new test code belongs in a split-out file rather than inline.
//!
//! The defect: `render_python_wildcard_assertion` renders the container half through
//! `FieldResolver::accessor` (anchored at the call's RESULT type) and the element half through
//! what was then the shared, cross-language `FieldResolver::element_accessor`. For Python that
//! shared path rendered through `render_python_with_optionals`, whose `TypedDict`-vs-attribute
//! owner cursor ALSO started at the result root type -- even for the element half. A `TypedDict`
//! result envelope (subscript access) whose collection elements are a separate, non-`TypedDict`
//! type (native `#[pyclass]`, attribute access) got the envelope's classification applied to its
//! elements too: `_e["kind"]` against an object that only supports `_e.kind`, which is exactly
//! the runtime failure a consumer reported (`TypeError: 'SampleItem' object is not
//! subscriptable`) after `result["structure"]` had already evaluated fine one level up.
//!
//! This fixture reproduces both levels disagreeing in the SAME expression: the result envelope IS
//! a `TypedDict` (outer subscript) while the collection element type is NOT (inner attribute) --
//! a single-level fixture cannot distinguish "classify per level" from "assume one style
//! everywhere", because both directions agree when there is only one level. ~keep

use std::collections::{HashMap, HashSet};

use crate::e2e::codegen::python::assertions::render_assertion;
use crate::e2e::field_access::{FieldResolver, PythonTypedDictMap};
use crate::e2e::fixture::Assertion;

/// The call's declared result type, classified as a `TypedDict` -- the pyo3 backend emits it as a
/// plain `dict` at runtime, so its own fields subscript (`result["structure"]`).
const ROOT_TYPE: &str = "SampleResult";

/// The IR type of `structure`'s elements. NOT classified as `TypedDict`: the pyo3 backend emits
/// it as a native `#[pyclass]`, so its fields are attribute access (`_e.kind`), exactly the shape
/// alef's own generated `.pyi` stub documents for such a type.
const ELEMENT_TYPE: &str = "SampleItem";

fn envelope_typeddict_resolver() -> FieldResolver {
    let mut map = PythonTypedDictMap {
        typeddict_types: HashSet::from([ROOT_TYPE.to_string()]),
        ..Default::default()
    };
    map.field_types
        .entry(ROOT_TYPE.to_string())
        .or_default()
        .insert("structure".to_string(), ELEMENT_TYPE.to_string());

    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_python_typeddict_map(map, Some(ROOT_TYPE.to_string()))
}

fn render(resolver: &FieldResolver) -> String {
    let assertion = Assertion {
        assertion_type: "contains".to_string(),
        field: Some("structure[].kind".to_string()),
        value: Some(serde_json::json!("Function")),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        resolver,
        &HashSet::new(),
        &HashMap::new(),
        false,
    );
    out
}

/// ACCEPTANCE: a path crossing both levels renders the outer container as a subscript (the
/// envelope IS a `TypedDict`) and the inner element field as an attribute (the element type is
/// NOT) in the SAME expression. Before the fix this rendered
/// `any("Function" in str(_e["kind"]) for _e in (result["structure"] or []))`, reproducing
/// `TypeError: 'SampleItem' object is not subscriptable` at runtime against the element.
#[test]
fn wildcard_element_access_style_is_resolved_independently_of_the_container_level() {
    let rendered = render(&envelope_typeddict_resolver());
    assert_eq!(
        rendered,
        "    assert any(\"Function\" in str(_e.kind) for _e in (result[\"structure\"] or []))\n"
    );
}

/// CONTROL: when the element type is ALSO classified as `TypedDict`, the element half subscripts
/// too -- proving the fix asks the element's own classification rather than always defaulting to
/// attribute access.
#[test]
fn wildcard_element_access_subscripts_when_the_element_type_is_also_a_typeddict() {
    let mut resolver = envelope_typeddict_resolver();
    resolver = {
        let mut map = resolver.python_typeddict_map().clone();
        map.typeddict_types.insert(ELEMENT_TYPE.to_string());
        resolver.with_python_typeddict_map(map, Some(ROOT_TYPE.to_string()))
    };

    let rendered = render(&resolver);
    assert_eq!(
        rendered,
        "    assert any(\"Function\" in str(_e[\"kind\"]) for _e in (result[\"structure\"] or []))\n"
    );
}
