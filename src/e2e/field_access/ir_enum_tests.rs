//! Table-driven tests for `crate::e2e::field_access::ir_enum` and its integration into
//! `FieldResolver::is_enum` — the fix for the defect where enum-ness was decided purely from
//! a hand-written `alef.toml` `fields_enum` list instead of the crate's own IR.

use std::collections::HashSet;

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;

use super::ir_enum::{build_ir_enum_map, is_enum_path};
use super::types::IrEnumMap;

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn type_def(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        fields,
        ..TypeDef::default()
    }
}

fn enum_def(name: &str) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        ..EnumDef::default()
    }
}

/// The fixture at the heart of the reported defect: two structs each declare a field named
/// `kind`, but only one of them is actually enum-typed. `DataNode.kind: DataNodeKind` (a real
/// IR enum) sits beside `PlainNode.kind: String`. A name-keyed rule cannot get both right.
fn ambiguous_kind_type_defs() -> Vec<TypeDef> {
    vec![
        type_def(
            "DataNode",
            vec![field("kind", TypeRef::Named("DataNodeKind".to_string()))],
        ),
        type_def("PlainNode", vec![field("kind", TypeRef::String)]),
    ]
}

fn ambiguous_kind_enums() -> Vec<EnumDef> {
    vec![enum_def("DataNodeKind")]
}

#[test]
fn a_field_whose_declared_type_is_a_real_enum_is_derived_as_enum() {
    let map = build_ir_enum_map(&ambiguous_kind_type_defs(), &ambiguous_kind_enums());
    let map = IrEnumMap {
        root_type: Some("DataNode".to_string()),
        ..map
    };

    assert!(is_enum_path(&map, "kind"), "DataNode.kind is DataNodeKind, a real enum");
}

#[test]
fn a_field_with_the_same_name_but_a_string_type_on_a_different_owner_is_not_enum() {
    let map = build_ir_enum_map(&ambiguous_kind_type_defs(), &ambiguous_kind_enums());
    let map = IrEnumMap {
        root_type: Some("PlainNode".to_string()),
        ..map
    };

    assert!(
        !is_enum_path(&map, "kind"),
        "PlainNode.kind is String — the bare name 'kind' must not decide this"
    );
}

#[test]
fn an_option_wrapped_enum_field_is_derived_as_enum() {
    let type_defs = vec![type_def(
        "Response",
        vec![field(
            "status",
            TypeRef::Optional(Box::new(TypeRef::Named("Status".to_string()))),
        )],
    )];
    let enums = vec![enum_def("Status")];
    let map = build_ir_enum_map(&type_defs, &enums);
    let map = IrEnumMap {
        root_type: Some("Response".to_string()),
        ..map
    };

    assert!(is_enum_path(&map, "status"), "Option<Status> must unwrap to the enum");
}

#[test]
fn a_vec_wrapped_element_field_reached_via_wildcard_traversal_is_derived_as_enum() {
    // `Result.links: Vec<Link>`, `Link.link_type: LinkType` (enum) — mirrors the
    // `links[].link_type` path form the Rust wildcard-assertion renderer produces.
    let type_defs = vec![
        type_def(
            "Result",
            vec![field(
                "links",
                TypeRef::Vec(Box::new(TypeRef::Named("Link".to_string()))),
            )],
        ),
        type_def("Link", vec![field("link_type", TypeRef::Named("LinkType".to_string()))]),
    ];
    let enums = vec![enum_def("LinkType")];
    let map = build_ir_enum_map(&type_defs, &enums);
    let map = IrEnumMap {
        root_type: Some("Result".to_string()),
        ..map
    };

    assert!(
        is_enum_path(&map, "links[].link_type"),
        "Vec<Link>.link_type must be reached through the wildcard array segment"
    );
    // The already-split element sub-path (what a hand-written `fields_enum` entry would
    // name) must NOT resolve on its own without the array segment: `link_type` is not a
    // direct field of `Result`, the root type.
    assert!(
        !is_enum_path(&map, "link_type"),
        "a bare leaf name must not resolve against the wrong owner type"
    );
}

#[test]
fn a_nested_indexed_path_is_derived_as_enum() {
    // `Response.choices: Vec<Choice>`, `Choice.finish_reason: FinishReason` (enum) — mirrors
    // `choices[0].finish_reason`.
    let type_defs = vec![
        type_def(
            "Response",
            vec![field(
                "choices",
                TypeRef::Vec(Box::new(TypeRef::Named("Choice".to_string()))),
            )],
        ),
        type_def(
            "Choice",
            vec![field("finish_reason", TypeRef::Named("FinishReason".to_string()))],
        ),
    ];
    let enums = vec![enum_def("FinishReason")];
    let map = build_ir_enum_map(&type_defs, &enums);
    let map = IrEnumMap {
        root_type: Some("Response".to_string()),
        ..map
    };

    assert!(is_enum_path(&map, "choices[0].finish_reason"));
}

#[test]
fn a_missing_root_type_answers_false_rather_than_guessing() {
    let map = build_ir_enum_map(&ambiguous_kind_type_defs(), &ambiguous_kind_enums());
    // root_type left as None (build_ir_enum_map never sets it).
    assert!(!is_enum_path(&map, "kind"));
}

#[test]
fn a_path_through_an_unknown_field_answers_false() {
    let map = build_ir_enum_map(&ambiguous_kind_type_defs(), &ambiguous_kind_enums());
    let map = IrEnumMap {
        root_type: Some("DataNode".to_string()),
        ..map
    };

    assert!(!is_enum_path(&map, "nonexistent_field"));
    assert!(!is_enum_path(&map, "nonexistent_parent.kind"));
}

/// End-to-end proof that `FieldResolver::is_enum` actually consults the IR fallback once
/// `with_ir_enum_map` wires it in — not just the standalone `is_enum_path` helper.
#[test]
fn field_resolver_is_enum_consults_the_ir_fallback_when_config_is_silent() {
    let map = FieldResolver::ir_enum_fields(&ambiguous_kind_type_defs(), &ambiguous_kind_enums());
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_enum_map(map, Some("DataNode".to_string()));

    assert!(
        resolver.is_enum("kind"),
        "fields_enum was never configured; the IR alone must answer this"
    );
}

/// The companion case: the same field name on the type where it is genuinely a `String` must
/// stay `false`, proving the resolver-level integration is exactly as owner-aware as
/// `is_enum_path` itself.
#[test]
fn field_resolver_is_enum_does_not_misclassify_the_same_name_on_a_different_owner() {
    let map = FieldResolver::ir_enum_fields(&ambiguous_kind_type_defs(), &ambiguous_kind_enums());
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_enum_map(map, Some("PlainNode".to_string()));

    assert!(!resolver.is_enum("kind"));
}

/// Hard requirement: an explicitly-configured `fields_enum` entry must keep winning even when
/// the IR would (wrongly, or simply because the config author knows something the IR can't
/// see, e.g. a type alias) disagree — regressing an already-correct consumer config is
/// unacceptable.
#[test]
fn an_explicit_fields_enum_entry_wins_even_when_the_ir_disagrees() {
    let map = FieldResolver::ir_enum_fields(&ambiguous_kind_type_defs(), &ambiguous_kind_enums());
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_enum_fields(HashSet::from(["kind".to_string()]))
    // Anchored at PlainNode, where the IR says `kind` is a plain String.
    .with_ir_enum_map(map, Some("PlainNode".to_string()));

    assert!(
        resolver.is_enum("kind"),
        "an explicit fields_enum entry must win over an IR disagreement"
    );
}

/// A resolver that never calls `with_ir_enum_map` at all (every existing call site before
/// this fix, and every backend that hasn't been wired up yet) must behave exactly as before:
/// `is_enum` answers strictly from `fields_enum`.
#[test]
fn a_resolver_with_no_ir_enum_map_wired_in_behaves_exactly_as_before() {
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );

    assert!(!resolver.is_enum("kind"));

    let resolver = resolver.with_enum_fields(HashSet::from(["kind".to_string()]));
    assert!(resolver.is_enum("kind"));
}

/// `variant_payload_is_collection` must distinguish a tuple variant whose single field is
/// itself `Vec<T>` (`Found(Vec<Entry>)`) from a variant wrapping a struct that merely contains
/// a collection field elsewhere (`Wrapped(Payload)`) — the shape distinction
/// `FieldResolver::union_variant_payload_is_collection` needs when a fixture path names only
/// the variant, with no field inside its payload (the "the payload itself is the list" case
/// `csharp`/`kotlin` count_min assertions used to silently drop).
#[test]
fn variant_payload_is_collection_distinguishes_a_direct_vec_payload_from_a_wrapping_struct() {
    let enums = vec![EnumDef {
        name: "Outcome".to_string(),
        variants: vec![
            EnumVariant {
                name: "Found".to_string(),
                fields: vec![field("_0", TypeRef::Vec(Box::new(TypeRef::Named("Entry".to_string()))))],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Wrapped".to_string(),
                fields: vec![field("payload", TypeRef::Named("Payload".to_string()))],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Empty".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }];
    let map = build_ir_enum_map(&[], &enums);

    assert!(
        map.variant_payload_is_collection
            .get("Outcome")
            .is_some_and(|variants| variants.contains("Found")),
        "Found(Vec<Entry>) wraps a collection directly"
    );
    assert!(
        !map.variant_payload_is_collection
            .get("Outcome")
            .is_some_and(|variants| variants.contains("Wrapped")),
        "Wrapped(Payload) wraps a struct, not a collection"
    );
    assert!(
        !map.variant_payload_is_collection
            .get("Outcome")
            .is_some_and(|variants| variants.contains("Empty")),
        "a fieldless variant has no payload to classify"
    );
}

/// The resolver-level surface `csharp`/`kotlin` call: `union_variant_payload_is_collection`
/// answers `true` for the direct-`Vec` variant and `false` for both the struct-wrapping variant
/// and an unknown union/variant name, without ever needing a field name — unlike
/// `union_variant_field_is_collection`, which requires one and cannot answer this question.
#[test]
fn resolver_union_variant_payload_is_collection_matches_the_ir() {
    let enums = vec![EnumDef {
        name: "Outcome".to_string(),
        variants: vec![
            EnumVariant {
                name: "Found".to_string(),
                fields: vec![field("_0", TypeRef::Vec(Box::new(TypeRef::Named("Entry".to_string()))))],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Wrapped".to_string(),
                fields: vec![field("payload", TypeRef::Named("Payload".to_string()))],
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }];
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_enum_map(FieldResolver::ir_enum_fields(&[], &enums), None);

    assert!(resolver.union_variant_payload_is_collection("Outcome", "Found"));
    assert!(!resolver.union_variant_payload_is_collection("Outcome", "Wrapped"));
    assert!(!resolver.union_variant_payload_is_collection("Outcome", "Missing"));
    assert!(!resolver.union_variant_payload_is_collection("UnknownUnion", "Found"));
}
