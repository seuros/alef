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
use crate::e2e::codegen::wildcard_element_fixture::{ENVELOPE_ROOT_TYPE, ROOT_TYPE as REPORT_TYPE, envelope_resolver};
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
    render_field(resolver, "structure[].kind")
}

fn render_field(resolver: &FieldResolver, field: &str) -> String {
    let assertion = Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
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

/// CONTROL against an OVER-BROAD fix, and nothing more. When the element type is also classified
/// as `TypedDict`, element and container classify identically, so this expectation is what the
/// pre-fix root-anchored renderer produced too -- it passes with the fix reverted and therefore
/// discriminates nothing about anchoring. What it does guard is the opposite failure: a "fix" that
/// hard-coded attribute access for every element half (or that dropped the map lookup entirely)
/// would render `_e.kind` here and fail. Keep it as the negative bound on
/// `wildcard_element_access_style_is_resolved_independently_of_the_container_level` above, which
/// is the test that actually fails when the anchoring regresses. ~keep
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

/// The IR type of the two-level container's intermediate hop (`SampleResult.metadata`), itself a
/// `TypedDict` -- so both container hops subscript.
const NESTED_OWNER_TYPE: &str = "SampleMetadata";

/// The IR type of `metadata.favicons`'s elements. NOT a `TypedDict`, so the element half must be
/// attribute access even though every container hop above it subscripts.
const NESTED_ELEMENT_TYPE: &str = "SampleLinkInfo";

/// `SampleResult { metadata: SampleMetadata }`, `SampleMetadata { favicons: Vec<SampleLinkInfo> }`
/// -- a container path with TWO segments, which is the shape the real consumer report used
/// (`result["metadata"].favicons` then `_e.rel`) and which neither test above reaches: both use a
/// single-segment `array_part`, so the element-owner walk's loop only ever runs once for them.
fn nested_container_resolver() -> FieldResolver {
    let mut map = PythonTypedDictMap {
        typeddict_types: HashSet::from([ROOT_TYPE.to_string(), NESTED_OWNER_TYPE.to_string()]),
        ..Default::default()
    };
    map.field_types
        .entry(ROOT_TYPE.to_string())
        .or_default()
        .insert("metadata".to_string(), NESTED_OWNER_TYPE.to_string());
    map.field_types
        .entry(NESTED_OWNER_TYPE.to_string())
        .or_default()
        .insert("favicons".to_string(), NESTED_ELEMENT_TYPE.to_string());

    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_python_typeddict_map(map, Some(ROOT_TYPE.to_string()))
}

/// ACCEPTANCE (two-level container): the element-owner walk must advance through EVERY segment of
/// the container path, not just the first. Both container hops subscript (`SampleResult` and
/// `SampleMetadata` are `TypedDict`s) while the element stays attribute access.
///
/// Pre-fix -- with the element cursor started at `typeddict_map.root_type` -- this rendered
/// `_e["rel"]`, since `SampleResult` is a `TypedDict`. A walk that consulted only the FIRST
/// container segment would land on `SampleMetadata`, also a `TypedDict`, and likewise render
/// `_e["rel"]`; only advancing all the way to `SampleLinkInfo` produces `_e.rel`. ~keep
#[test]
fn wildcard_element_owner_is_resolved_through_every_segment_of_a_nested_container() {
    let rendered = render_field(&nested_container_resolver(), "metadata.favicons[].rel");
    assert_eq!(
        rendered,
        "    assert any(\"Function\" in str(_e.rel) for _e in (result[\"metadata\"][\"favicons\"] or []))\n"
    );
}

/// The IR type of `Report.records`'s elements in the shared envelope fixture.
const ENVELOPE_ELEMENT_TYPE: &str = "Entry";

/// The shared envelope fixture (`Envelope { results: Vec<Report> }`, `Report { records:
/// Vec<Entry> }`) plus the `TypedDict` classification a `python_output = "typed-dict"` crate
/// produces for it: every return type in the chain is a `TypedDict`, so the container renders as
/// `result["results"][0]["records"]` and the element half must subscript too.
fn envelope_typeddict_projection_resolver() -> FieldResolver {
    let mut map = PythonTypedDictMap {
        typeddict_types: HashSet::from([
            ENVELOPE_ROOT_TYPE.to_string(),
            REPORT_TYPE.to_string(),
            ENVELOPE_ELEMENT_TYPE.to_string(),
        ]),
        ..Default::default()
    };
    map.field_types
        .entry(ENVELOPE_ROOT_TYPE.to_string())
        .or_default()
        .insert("results".to_string(), REPORT_TYPE.to_string());
    map.field_types
        .entry(REPORT_TYPE.to_string())
        .or_default()
        .insert("records".to_string(), ENVELOPE_ELEMENT_TYPE.to_string());

    envelope_resolver("python").with_python_typeddict_map(map, Some(ENVELOPE_ROOT_TYPE.to_string()))
}

/// REGRESSION: the element-owner walk must start from the SAME path the container half was
/// rendered from -- `FieldResolver::result_relative_path`, envelope projection applied -- not from
/// the raw `resolve`d fixture spelling.
///
/// `records` is not declared on `Envelope`; the projection relocates the container to
/// `results[0].records`. Walking the raw `records` instead evaluates `advance("Envelope",
/// "records")`, which finds no edge, so the owner resolves to `None`, `is_typeddict(None)` is
/// `false`, and the element half falls back to attribute access -- rendering `_e.kind` against a
/// plain `dict`. That is `TypeError: 'dict' object has no attribute 'kind'`, and it means the
/// element-anchoring fix was inert on exactly the projected shapes it existed to serve.
///
/// This test fails with `python_element_accessor` walking `self.resolve(array_path)` and passes
/// with it walking `self.result_relative_path(array_path)`. The element type being a `TypedDict`
/// here is load-bearing: if it were not, the correct answer and the `None` fallback would both be
/// `_e.kind` and the test could not fail. ~keep
#[test]
fn wildcard_element_owner_follows_the_containers_envelope_projection() {
    let rendered = render_field(&envelope_typeddict_projection_resolver(), "records[].kind");
    assert_eq!(
        rendered,
        "    assert any(\"Function\" in str(_e[\"kind\"]) for _e in (result[\"results\"][0][\"records\"] or []))\n"
    );
}
