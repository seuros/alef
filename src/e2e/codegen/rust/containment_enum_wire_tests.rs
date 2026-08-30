//! Regression coverage for the CONTAINMENT operators on a serde-RENAMED enum variant in the
//! Rust e2e generator — the surface `renamed_enum_wire_assertion_tests.rs` covers for `equals`.
//!
//! The defect these pin: `containment_predicate`'s enum arm emits
//! `format!("{:?}", field).to_lowercase().contains(&EXPECTED.to_lowercase())`. `Debug` renders
//! the RUST identifier; a fixture's expectation is the SERDE WIRE value. Under a
//! `#[serde(rename)]` / `#[serde(rename_all)]` the two spellings diverge and the needle can
//! never be found, so a correct result failed its own assertion. The equality fix reconciled
//! that for `assert_eq!` and left all four containment operators on the wrong surface.
//!
//! ~keep These tests deliberately drive `render_assertion` rather than `containment_expected`
//! directly. The unit under repair is not the lookup — `ir_enum_tests.rs` already table-tests
//! the map's recording rules — it is the WIRING: FIVE separate call sites each built their own
//! needle with `value_to_rust_string`, four in the operator arms and a fifth in the wildcard
//! renderer, which returns before those arms are ever reached and stringifies WITHOUT the
//! `to_lowercase()`. A fix reaching only some of them would leave the generator disagreeing
//! with itself about one enum, which is precisely the two-generators failure mode the shared
//! oracle exists to prevent, so every site is covered here by an operator-level test.

use std::collections::{HashMap, HashSet};

use super::assertions::render_assertion;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// The accessor spelling the byte-exact non-wildcard cases are built around. Pinned by
/// [`harness_renders_the_field_access_these_tests_assume`] so a change in accessor rendering
/// fails there, with its own name, instead of being misread as a wire-translation regression.
/// The wildcard cases assert on needle substrings instead, since their emitted expression
/// composes several accessors whose exact shape is not what those tests are about.
const FIELD_ACCESS: &str = "result.kind";

fn node_kind_enums() -> Vec<EnumDef> {
    vec![EnumDef {
        name: "NodeKind".to_string(),
        variants: vec![
            variant("KeyValue", Some("key-value")),
            variant("Plain", None),
            variant("Anchor", Some("Plain")),
            variant("Bold", Some("bold")),
        ],
        ..EnumDef::default()
    }]
}

fn variant(name: &str, serde_rename: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        serde_rename: serde_rename.map(str::to_string),
        ..EnumVariant::default()
    }
}

/// `Result { kind: NodeKind }`, where `NodeKind` exercises all four recording outcomes of
/// `IrEnumMap::enum_wire_variants` on ONE enum, so every case below runs against identical
/// resolver state and only the fixture value differs:
///
/// * `KeyValue` -> `"key-value"` — a rename that genuinely separates the two spellings, hence
///   recorded, hence the only case whose emitted needle may change;
/// * `Plain` — unrenamed, so its wire value IS its identifier and there is nothing to record;
/// * `Anchor` -> `"Plain"` — a wire value that collides with ANOTHER variant's identifier,
///   excluded from the map because translating it would silently redirect the assertion from
///   the variant the fixture names on the wire surface to a different variant entirely;
/// * `Bold` -> `"bold"` — a rename differing from the identifier only by case, recorded like
///   any other, and the case the containment predicate's `to_lowercase()` renders a no-op.
fn renamed_enum_resolver() -> FieldResolver {
    let type_defs = vec![TypeDef {
        name: "Result".to_string(),
        fields: vec![FieldDef {
            name: "kind".to_string(),
            ty: TypeRef::Named("NodeKind".to_string()),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let ir_enum_map = FieldResolver::ir_enum_fields(&type_defs, &node_kind_enums());
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(ir_enum_map, Some("Result".to_string()))
}

/// A resolver that knows `kind` is enum-typed ONLY from the hand-maintained `fields_enum`
/// config, with no IR behind it — the shape every fixture suite whose result type never
/// resolved still has.
fn config_only_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_enum_fields(HashSet::from(["kind".to_string()]))
}

/// `Result { links: Vec<Link> }`, `Link { link_type: NodeKind }` — the shape a wildcard fixture
/// path (`links[].link_type`) traverses, carrying the same `NodeKind` as
/// [`renamed_enum_resolver`] so the two surfaces are compared on one enum.
fn wildcard_enum_resolver() -> FieldResolver {
    let type_defs = vec![
        TypeDef {
            name: "Result".to_string(),
            fields: vec![FieldDef {
                name: "links".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Link".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Link".to_string(),
            fields: vec![FieldDef {
                name: "link_type".to_string(),
                ty: TypeRef::Named("NodeKind".to_string()),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ];
    let ir_enum_map = FieldResolver::ir_enum_fields(&type_defs, &node_kind_enums());
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(ir_enum_map, Some("Result".to_string()))
}

fn render_wildcard(operator: &str, fixture_value: &str) -> String {
    render(
        &Assertion {
            assertion_type: operator.to_string(),
            field: Some("links[].link_type".to_string()),
            value: Some(serde_json::Value::String(fixture_value.to_string())),
            ..Assertion::default()
        },
        &wildcard_enum_resolver(),
    )
}

fn render(assertion: &Assertion, resolver: &FieldResolver) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        "sample",
        "sample",
        false,
        &[],
        resolver,
        false,
        false,
        false,
        false,
        false,
        None,
    );
    out
}

fn render_single(operator: &str, resolver: &FieldResolver, fixture_value: &str) -> String {
    render(
        &Assertion {
            assertion_type: operator.to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String(fixture_value.to_string())),
            ..Assertion::default()
        },
        resolver,
    )
}

fn render_multi(operator: &str, resolver: &FieldResolver, fixture_values: &[&str]) -> String {
    render(
        &Assertion {
            assertion_type: operator.to_string(),
            field: Some("kind".to_string()),
            values: Some(
                fixture_values
                    .iter()
                    .map(|v| serde_json::Value::String((*v).to_string()))
                    .collect(),
            ),
            ..Assertion::default()
        },
        resolver,
    )
}

/// The exact predicate `containment_predicate`'s enum arm builds for `needle`.
fn predicate(needle: &str) -> String {
    format!("format!(\"{{:?}}\", {FIELD_ACCESS}).to_lowercase().contains(&r#\"{needle}\"#.to_lowercase())")
}

fn contains_line(needle: &str) -> String {
    format!(
        "    assert!({}, \"expected to contain: {{}}\", r#\"{needle}\"#);\n",
        predicate(needle)
    )
}

fn not_contains_line(needle: &str) -> String {
    format!(
        "    assert!(!{}, \"expected NOT to contain: {{}}\", r#\"{needle}\"#);\n",
        predicate(needle)
    )
}

/// Harness guard: everything below hard-codes [`FIELD_ACCESS`] into its expected bytes.
#[test]
fn harness_renders_the_field_access_these_tests_assume() {
    let ir = renamed_enum_resolver();
    assert_eq!(ir.accessor("kind", "rust", "result"), FIELD_ACCESS);
    assert!(ir.is_enum("kind"), "the IR must classify `kind` as enum-typed");

    let config_only = config_only_resolver();
    assert_eq!(config_only.accessor("kind", "rust", "result"), FIELD_ACCESS);
    assert!(
        config_only.is_enum("kind"),
        "the hand-maintained config entry must classify `kind` as enum-typed"
    );
}

/// `contains` must search for the RUST identifier the emitted `Debug` rendering produces, not
/// the wire spelling the fixture records.
///
/// Reverted, the needle stays `r#"key-value"#`: `format!("{:?}", NodeKind::KeyValue)` is
/// `"KeyValue"`, which case-folds to `keyvalue` and does not contain `key-value`, so a
/// correct result fails its own generated assertion.
#[test]
fn contains_searches_for_the_rust_identifier_of_a_renamed_variant() {
    let rendered = render_single("contains", &renamed_enum_resolver(), "key-value");
    assert_eq!(
        rendered,
        contains_line("KeyValue"),
        "`contains` must reconcile the wire value onto the Debug surface, got: {rendered}"
    );
}

/// The same reconciliation must reach every containment operator, not just `contains`.
///
/// Reverted, all three of these keep the wire spelling. `not_contains` is the nastiest of them:
/// it does not fail loudly, it PASSES — `!Debug.contains("key-value")` is trivially true for
/// every variant — so a fixture asserting a variant is absent stops testing anything at all.
#[test]
fn every_containment_operator_reconciles_the_renamed_variant() {
    let resolver = renamed_enum_resolver();

    let contains_all = render_multi("contains_all", &resolver, &["key-value", "not-a-variant"]);
    assert_eq!(
        contains_all,
        format!("{}{}", contains_line("KeyValue"), contains_line("not-a-variant")),
        "`contains_all` must translate per value, got: {contains_all}"
    );

    let not_contains = render_single("not_contains", &resolver, "key-value");
    assert_eq!(
        not_contains,
        not_contains_line("KeyValue"),
        "`not_contains` must translate, or it asserts the absence of a string no variant renders, \
         got: {not_contains}"
    );

    let contains_any = render_multi("contains_any", &resolver, &["key-value", "Plain"]);
    assert_eq!(
        contains_any,
        format!(
            "    assert!({} || {}, \"expected to contain at least one of the specified values\");\n",
            predicate("KeyValue"),
            predicate("Plain")
        ),
        "`contains_any` must translate each alternative independently, got: {contains_any}"
    );
}

/// The controls. A fix that blanket-translates every enum fixture value, or that collapses to a
/// constant, has to fail here even though the test above passes.
///
/// * `Plain` is BOTH an unrenamed variant's identifier and `Anchor`'s wire value. The map
///   deliberately excludes it, so it must pass through verbatim; a blanket translation would
///   emit `Anchor` and quietly retarget the assertion at the wrong variant.
/// * `not-a-variant` names nothing on this enum and must survive byte-for-byte, so a genuinely
///   wrong fixture still generates a failing assertion instead of being rewritten into a
///   passing one.
/// * The three renderings stay mutually distinct, which a collapse-to-constant fix cannot do.
#[test]
fn collision_and_unknown_fixture_values_pass_through_untranslated() {
    let resolver = renamed_enum_resolver();

    let collision = render_single("contains", &resolver, "Plain");
    assert_eq!(
        collision,
        contains_line("Plain"),
        "a wire value that is another variant's identifier must not be translated, got: {collision}"
    );

    let unknown = render_single("contains", &resolver, "not-a-variant");
    assert_eq!(
        unknown,
        contains_line("not-a-variant"),
        "an unrecognized fixture value must not be rewritten, got: {unknown}"
    );

    let renamed = render_single("contains", &resolver, "key-value");
    assert_ne!(renamed, collision);
    assert_ne!(renamed, unknown);
    assert_ne!(collision, unknown);
}

/// ~keep The `to_lowercase()` determination, pinned as behaviour rather than left in prose.
///
/// `Bold` -> `"bold"` differs from its identifier only by case, so the map records it and the
/// needle is rewritten to `Bold`. Because the predicate folds BOTH sides, the rewrite cannot
/// change whether the assertion holds — `bold` and `Bold` fold to the same needle. That is the
/// whole interaction: case folding makes a case-only rename a no-op for containment (while
/// remaining a real fix for `equals`, which does not fold), and it introduces no reason to
/// exclude such a variant from the shared map. Excluding it here would give containment its own
/// recording rule and split the one oracle back into two.
#[test]
fn a_case_only_rename_is_translated_and_the_predicate_is_unchanged_by_it() {
    let resolver = renamed_enum_resolver();

    let rendered = render_single("contains", &resolver, "bold");
    assert_eq!(
        rendered,
        contains_line("Bold"),
        "the shared map records a case-only rename like any other, got: {rendered}"
    );

    let folded_needle = "Bold".to_lowercase();
    assert_eq!(
        folded_needle,
        "bold".to_lowercase(),
        "both spellings must fold together, which is what makes the rewrite semantically inert"
    );
}

/// Without the IR there is no rename to consult, so the fixture literal must survive
/// untranslated. This pins the additive property: the lookup can only change output where the
/// IR positively resolves a recorded rename.
///
/// Reverted this passes too — it is here to fail a fix that translates from something other
/// than the shared map, or that reaches for a case-folded guess when the map has no answer.
#[test]
fn config_only_enum_classification_leaves_the_containment_needle_untranslated() {
    let rendered = render_single("contains", &config_only_resolver(), "key-value");
    assert_eq!(
        rendered,
        contains_line("key-value"),
        "no IR means no rename knowledge; the literal must be emitted verbatim, got: {rendered}"
    );
}

/// The SECOND live site. `render_rust_wildcard_assertion` builds its own needle for
/// `links[].link_type`-shaped paths and never reaches `containment_predicate`, so routing only
/// the non-wildcard arms would have left the identical mismatch one branch over — and a worse
/// one: this predicate has no `to_lowercase()`, so the comparison is exact and a renamed
/// variant can never match.
///
/// Reverted, the emitted needle is `r#"key-value"#` while the element stringifies to
/// `"KeyValue"`: `contains` is false for the very variant the fixture names.
#[test]
fn wildcard_containment_reconciles_the_renamed_variant() {
    let rendered = render_wildcard("contains", "key-value");

    assert!(
        rendered.contains("format!(\"{:?}\", e.link_type)"),
        "the wildcard element must stringify through Debug for this test to exercise the enum \
         arm at all, got: {rendered}"
    );
    assert!(
        rendered.contains(".contains(r#\"KeyValue\"#)"),
        "the wildcard needle must be the Rust identifier, got: {rendered}"
    );
    assert!(
        !rendered.contains("r#\"key-value\"#"),
        "no occurrence of the wire spelling may survive in the emitted needle, got: {rendered}"
    );
}

/// `not_contains` on the wildcard path, and the same two controls the non-wildcard tests carry.
///
/// The `not_contains` half is the vacuous-pass case again: untranslated, no element's `Debug`
/// rendering can contain `key-value`, so the assertion holds for every possible result.
#[test]
fn wildcard_containment_keeps_its_controls() {
    let negated = render_wildcard("not_contains", "key-value");
    assert!(
        negated.contains(".contains(r#\"KeyValue\"#)") && negated.contains("assert!(!"),
        "`not_contains` must negate a predicate that searches for the identifier, got: {negated}"
    );

    let collision = render_wildcard("contains", "Plain");
    assert!(
        collision.contains(".contains(r#\"Plain\"#)"),
        "a wire value that is another variant's identifier must not be translated, got: {collision}"
    );

    let unknown = render_wildcard("contains", "not-a-variant");
    assert!(
        unknown.contains(".contains(r#\"not-a-variant\"#)"),
        "an unrecognized fixture value must not be rewritten, got: {unknown}"
    );
}

/// The bare element half must never be the path the rename lookup is keyed on. `link_type` is
/// not declared on the ROOT type, so resolving it there is either a miss or a hit on an
/// unrelated owner's same-named field — the misclassification
/// `ir_enum_tests::a_vec_wrapped_element_field_reached_via_wildcard_traversal_is_derived_as_enum`
/// pins at the IR layer. This asserts the codegen layer honours it.
#[test]
fn the_bare_element_half_does_not_resolve_the_enum_on_its_own() {
    let resolver = wildcard_enum_resolver();

    assert!(
        resolver.is_enum("links[].link_type"),
        "the full wildcard path must resolve to the enum"
    );
    assert!(
        !resolver.is_enum("link_type"),
        "the bare leaf must not resolve against the root type, or the wildcard caller could key \
         the rename lookup on the wrong owner"
    );
    assert_eq!(
        resolver.enum_variant_for_wire_value("link_type", "key-value"),
        None,
        "the bare leaf must yield no rename"
    );
    assert_eq!(
        resolver.enum_variant_for_wire_value("links[].link_type", "key-value"),
        Some("KeyValue"),
        "the full path must yield the renamed variant"
    );
}
